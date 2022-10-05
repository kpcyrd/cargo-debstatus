use crate::errors::*;
use postgres::{Client, NoTls};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

const POSTGRES: &str = "postgresql://udd-mirror:udd-mirror@udd-mirror.debian.net/udd";
const CACHE_EXPIRE: Duration = Duration::from_secs(90 * 60);

#[derive(Debug, Serialize, Deserialize)]
pub struct CacheEntry {
    pub from: SystemTime,
    pub crate_status: CrateStatus,
}

/// The current status of a crate in Debian.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum CrateStatus {
    Available,
    AvailableInNew,
    Outdated,
    Missing,
}

impl CrateStatus {
    pub(crate) fn in_debian(&self) -> bool {
        *self != CrateStatus::Missing
    }
}

impl fmt::Display for CrateStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let str = match self {
            CrateStatus::Available => "in debian",
            CrateStatus::AvailableInNew => "in debian NEW queue",
            CrateStatus::Outdated => "outdated",
            CrateStatus::Missing => "missing",
        };
        write!(f, "{}", str)
    }
}

// TODO: also use this for outdated check(?)
fn is_compatible(a: &str, b: &str) -> Result<bool, Error> {
    let a = Version::parse(a)?;
    let b = Version::parse(b)?;

    if a.major > 0 || b.major > 0 {
        return Ok(a.major == b.major);
    }

    if a.minor > 0 || b.minor > 0 {
        return Ok(a.minor == b.minor);
    }

    Ok(a.patch == b.patch)
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

    fn cache_path(&self, target: &str, package: &str, version: &str) -> PathBuf {
        self.cache_dir
            .join(format!("{}-{}-{}", target, package, version))
    }

    fn check_cache(
        &self,
        target: &str,
        package: &str,
        version: &str,
    ) -> Result<Option<CrateStatus>, Error> {
        let path = self.cache_path(target, package, version);

        if !path.exists() {
            return Ok(None);
        }

        let buf = fs::read(path)?;
        // ignore I/O and deserialization errors when trying to read the cache
        if let Ok(cache) = serde_json::from_slice::<CacheEntry>(&buf) {
            if SystemTime::now().duration_since(cache.from)? > CACHE_EXPIRE {
                return Ok(None);
            } else {
                return Ok(Some(cache.crate_status));
            }
        }
        Ok(None)
    }

    fn write_cache(
        &self,
        target: &str,
        package: &str,
        version: &str,
        crate_status: CrateStatus,
    ) -> Result<(), Error> {
        let cache = CacheEntry {
            from: SystemTime::now(),
            crate_status: crate_status,
        };
        let buf = serde_json::to_vec(&cache)?;
        fs::write(self.cache_path(target, package, version), &buf)?;
        Ok(())
    }

    pub fn search(&mut self, package: &str, version: &str) -> Result<CrateStatus, Error> {
        if let Some(crate_status) = self.check_cache("sid", package, version)? {
            return Ok(crate_status);
        }

        // config.shell().status("Querying", format!("sid: {}", package))?;
        info!("Querying -> sid: {}", package);
        let crate_status = self.search_generic(
            "SELECT version::text FROM sources WHERE source=$1 AND release='sid';",
            package,
            version,
        )?;

        self.write_cache("sid", package, version, crate_status)?;
        Ok(crate_status)
    }

    pub fn search_new(&mut self, package: &str, version: &str) -> Result<CrateStatus, Error> {
        if let Some(crate_status) = self.check_cache("new", package, version)? {
            return Ok(crate_status);
        }

        // config.shell().status("Querying", format!("new: {}", package))?;
        info!("Querying -> new: {}", package);
        let mut crate_status = self.search_generic(
            "SELECT version::text FROM new_sources WHERE source=$1;",
            package,
            version,
        )?;

        if crate_status == CrateStatus::Available {
            crate_status = CrateStatus::AvailableInNew;
        }

        self.write_cache("new", package, version, crate_status)?;
        Ok(crate_status)
    }

    pub fn search_generic(
        &mut self,
        query: &str,
        package: &str,
        version: &str,
    ) -> Result<CrateStatus, Error> {
        let package = package.replace('_', "-");
        let rows = self.sock.query(query, &[&format!("rust-{}", package)])?;

        for row in &rows {
            let debversion: String = row.get(0);

            let debversion = match debversion.find('-') {
                Some(idx) => debversion.split_at(idx).0,
                _ => &debversion,
            };

            // println!("{:?} ({:?}) => {:?}", debversion, version, is_compatible(debversion, version)?);

            if is_compatible(debversion, version)? {
                return Ok(CrateStatus::Available);
            }
        }

        if rows.len() > 0 {
            return Ok(CrateStatus::Outdated);
        }

        Ok(CrateStatus::Missing)
    }
}
