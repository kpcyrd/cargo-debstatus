use cargo::Config;
use cargo::util::{self, important_paths, CargoResult, Cfg, Rustc};
use postgres::{self, TlsMode};
use semver::{Version, VersionReq};
use serde_json;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};


const POSTGRES: &str = "postgresql://udd-mirror:udd-mirror@udd-mirror.debian.net/udd";
const CACHE_EXPIRE: Duration = Duration::from_secs(90 * 60);

#[derive(Debug, Serialize, Deserialize)]
pub struct CacheEntry {
    pub from: SystemTime,
    pub found: Option<Version>,
    pub matches: bool,
}

#[derive(Debug)]
pub enum DebianRecord {
    Sid(Version),
    New(Version),
    NonMatching(Version),
}

fn is_compatible(a: &Version, r: &str) -> CargoResult<bool> {
    let r = VersionReq::parse(r)?;

    if r.matches(&a) {
        Ok(true)
    } else {
        Ok(false)
    }
}

pub struct Connection {
    sock: postgres::Connection,
    cache_dir: PathBuf,
}

impl Connection {
    pub fn new() -> CargoResult<Connection> {
        // let tls = postgres::tls::native_tls::NativeTls::new()?;
        // let sock = postgres::Connection::connect(POSTGRES, TlsMode::Require(&tls))?;
        // TODO: udd-mirror doesn't support tls
        let sock = postgres::Connection::connect(POSTGRES, TlsMode::None)?;

        let cache_dir = dirs::cache_dir().expect("cache directory not found")
                                         .join("cargo-debstatus");

        fs::create_dir_all(&cache_dir)?;

        Ok(Connection {
            sock,
            cache_dir,
        })
    }

    fn cache_path(&self, target: &str, package: &str, version: &str) -> PathBuf {
        self.cache_dir.join(format!("{}-{}-{}", target, package, version))
    }

    fn check_cache(&self, target: &str, package: &str, version: &str) -> CargoResult<Option<(bool, Option<Version>)>> {
        let path = self.cache_path(target, package, version);

        if !path.exists() {
            return Ok(None);
        }

        let buf = fs::read(path)?;
        let cache: CacheEntry = serde_json::from_slice(&buf)?;

        if SystemTime::now().duration_since(cache.from)? > CACHE_EXPIRE {
            Ok(None)
        } else {
            Ok(Some((cache.matches, cache.found)))
        }
    }

    fn write_cache(&self, target: &str, package: &str, version: &str, found: &Option<(bool, Version)>) -> CargoResult<()> {
        let matches = if let Some(found) = found {
            found.0
        } else {
            false
        };
        let cache = CacheEntry {
            from: SystemTime::now(),
            found: found.clone().map(|x|x.1),
            matches,
        };
        let buf = serde_json::to_vec(&cache)?;
        fs::write(self.cache_path(target, package, version), &buf)?;
        Ok(())
    }

    pub fn search(&self, config: &Config, package: &str, version: &str) -> CargoResult<Option<DebianRecord>> {
        if let Some(found) = self.check_cache("sid", package, version)? {
            if found.0 {
                return Ok(found.1.map(DebianRecord::Sid));
            } else {
                return Ok(found.1.map(DebianRecord::NonMatching));
            }
        }

        config.shell().status("Querying", format!("sid: {}", package))?;
        let found = self.search_generic("SELECT version::text FROM sources WHERE source=$1 AND release='sid';",
                            package, version)?;

        self.write_cache("sid", package, version, &found)?;
        if let Some(found) = found {
            if found.0 {
                Ok(Some(DebianRecord::Sid(found.1)))
            } else {
                Ok(Some(DebianRecord::NonMatching(found.1)))
            }
        } else {
            Ok(None)
        }
    }

    pub fn search_new(&self, config: &Config, package: &str, version: &str) -> CargoResult<Option<DebianRecord>> {
        if let Some(found) = self.check_cache("new", package, version)? {
            if found.0 {
                return Ok(found.1.map(DebianRecord::New));
            } else {
                return Ok(found.1.map(DebianRecord::NonMatching));
            }
        }

        config.shell().status("Querying", format!("new: {}", package))?;
        let found = self.search_generic("SELECT version::text FROM new_sources WHERE source=$1;",
                            package, version)?;

        self.write_cache("new", package, version, &found)?;
        if let Some(found) = found {
            if found.0 {
                Ok(Some(DebianRecord::New(found.1)))
            } else {
                Ok(Some(DebianRecord::NonMatching(found.1)))
            }
        } else {
            Ok(None)
        }
    }

    pub fn search_generic(&self, query: &str, package: &str, version: &str) -> CargoResult<Option<(bool, Version)>> {
        let package = package.replace("_", "-");
        let rows = self.sock.query(query,
                                   &[&format!("rust-{}", package)])?;

        let mut versions = Vec::new();
        for row in &rows {
            let debversion: String = row.get(0);

            let debversion = match debversion.find('-') {
                Some(idx) => debversion.split_at(idx).0,
                _ => &debversion,
            };
            versions.push(Version::parse(debversion).unwrap());
        }
        versions.sort_unstable();
        versions.reverse();

        for debversion in &versions {
            // println!("{} ({:?}) => {:?}", debversion, version, is_compatible(debversion, version)?);

            if is_compatible(debversion, version)? {
                return Ok(Some((true, debversion.clone())));
            }
        }

        if versions.len() > 0 {
            return Ok(Some((false, versions[0].clone())));
        }

        Ok(None)
    }
}
