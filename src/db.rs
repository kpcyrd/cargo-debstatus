use crate::errors::*;
use postgres::{Client, NoTls};
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

const POSTGRES: &str = "postgresql://udd-mirror:udd-mirror@udd-mirror.debian.net/udd";
const CACHE_EXPIRE: Duration = Duration::from_secs(90 * 60);

#[derive(Debug, Serialize, Deserialize)]
pub struct CacheEntry {
    pub from: SystemTime,
    pub found: bool,
}

// TODO: also use this for outdated check(?)
fn is_compatible(debversion: &str, crateversion: &VersionReq) -> Result<bool, Error> {
    let debversion = debversion.replace('~', "-");
    let debversion = Version::parse(&debversion)?;

    Ok(crateversion.matches(&debversion))
}

pub struct Connection {
    sock: Client,
    cache_dir: PathBuf,
}

impl Connection {
    pub fn new() -> Result<Connection, Error> {
        // let tls = postgres::tls::native_tls::NativeTls::new()?;
        // let sock = postgres::Connection::connect(POSTGRES, TlsMode::Require(&tls))?;
        // TODO: udd-mirror doesn't support tls
        debug!("Connecting to database");
        let sock = Client::connect(POSTGRES, NoTls)?;
        debug!("Got database connection");

        let cache_dir = dirs::cache_dir()
            .expect("cache directory not found")
            .join("cargo-debstatus");

        fs::create_dir_all(&cache_dir)?;

        Ok(Connection { sock, cache_dir })
    }

    fn cache_path(&self, target: &str, package: &str, version: &Version) -> PathBuf {
        self.cache_dir
            .join(format!("{target}-{package}-{}", version))
    }

    fn check_cache(
        &self,
        target: &str,
        package: &str,
        version: &Version,
    ) -> Result<Option<bool>, Error> {
        let path = self.cache_path(target, package, version);

        if !path.exists() {
            return Ok(None);
        }

        let buf = fs::read(path)?;
        let cache: CacheEntry = serde_json::from_slice(&buf)?;

        if SystemTime::now().duration_since(cache.from)? > CACHE_EXPIRE {
            Ok(None)
        } else {
            Ok(Some(cache.found))
        }
    }

    fn write_cache(
        &self,
        target: &str,
        package: &str,
        version: &Version,
        found: bool,
    ) -> Result<(), Error> {
        let cache = CacheEntry {
            from: SystemTime::now(),
            found,
        };
        let buf = serde_json::to_vec(&cache)?;
        fs::write(self.cache_path(target, package, version), buf)?;
        Ok(())
    }

    pub fn search(&mut self, package: &str, version: &Version) -> Result<bool, Error> {
        if let Some(found) = self.check_cache("sid", package, version)? {
            return Ok(found);
        }

        // config.shell().status("Querying", format!("sid: {}", package))?;
        info!("Querying -> sid: {}", package);
        let found = self.search_generic(
            "SELECT version::text FROM sources WHERE source in ($1, $2) AND release='sid';",
            package,
            version,
        )?;

        self.write_cache("sid", package, version, found)?;
        Ok(found)
    }

    pub fn search_new(&mut self, package: &str, version: &Version) -> Result<bool, Error> {
        if let Some(found) = self.check_cache("new", package, version)? {
            return Ok(found);
        }

        // config.shell().status("Querying", format!("new: {}", package))?;
        info!("Querying -> new: {}", package);
        let found = self.search_generic(
            "SELECT version::text FROM new_sources WHERE source in ($1, $2);",
            package,
            version,
        )?;

        self.write_cache("new", package, version, found)?;
        Ok(found)
    }

    pub fn search_generic(
        &mut self,
        query: &str,
        package: &str,
        version: &Version,
    ) -> Result<bool, Error> {
        let package = package.replace('_', "-");
        let semver_package = if version.major == 0 {
            format!("rust-{package}-{}.{}", version.major, version.minor)
        } else {
            format!("rust-{package}-{}", version.major)
        };
        let rows = self
            .sock
            .query(query, &[&format!("rust-{package}"), &semver_package])?;

        let version = version.to_string();
        let version = VersionReq::parse(&version)?;
        for row in &rows {
            let debversion: String = row.get(0);

            let debversion = match debversion.find('-') {
                Some(idx) => debversion.split_at(idx).0,
                _ => &debversion,
            };

            // println!("{:?} ({:?}) => {:?}", debversion, version, is_compatible(debversion, version)?);

            if is_compatible(debversion, &version)? {
                return Ok(true);
            }
        }

        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use crate::db::is_compatible;
    use semver::VersionReq;

    #[test]
    fn is_compatible_with_tilde() {
        assert!(is_compatible(
            "1.0.0~alpha.9",
            &VersionReq::parse("1.0.0-alpha.9").unwrap()
        )
        .unwrap());
    }

    #[test]
    fn is_compatible_follows_semver() {
        assert!(is_compatible("0.1.1", &VersionReq::parse("0.1.0").unwrap()).unwrap());
        assert!(!is_compatible("0.1.0", &VersionReq::parse("0.1.1").unwrap()).unwrap());
        assert!(is_compatible("1.1.0", &VersionReq::parse("1").unwrap()).unwrap());
    }
}
