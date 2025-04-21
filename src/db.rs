use crate::errors::*;
use postgres::types::ToSql;
use postgres::{Client as LiveClient, NoTls};
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

const POSTGRES: &str = "postgresql://udd-mirror:udd-mirror@udd-mirror.debian.net/udd";
const CACHE_EXPIRE: Duration = Duration::from_secs(90 * 60);

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub enum PkgStatus {
    NotFound,
    Outdated,
    Compatible,
    Found,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PkgInfo {
    pub status: PkgStatus,
    pub version: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CacheEntry {
    pub from: SystemTime,
    pub info: PkgInfo,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub enum PkgType {
    Source,
    Binary,
}

fn parse_deb_version(debversion: &str) -> Result<Version> {
    let mut debversion = debversion.replace('~', "-");
    if let Some((version, _suffix)) = debversion.split_once('+') {
        debversion = match version.matches('.').count() {
            0 => format!("{version}.0.0"),
            1 => format!("{version}.0"),
            2 => version.to_owned(),
            _ => bail!("wrong number of '.' characters in semver string: {version:?}"),
        };
    }
    let debversion = Version::parse(&debversion)?;
    Ok(debversion)
}

fn is_compatible(debversion: &str, crateversion: &VersionReq) -> Result<bool, Error> {
    let debversion = parse_deb_version(debversion)?;
    Ok(crateversion.matches(&debversion))
}

/// Trait which abstracts the SQL database for testing purposes
pub trait Client {
    /// Run a SQL query with parameters, returning a list of result rows
    fn run_query(&mut self, query: &str, params: &[&str]) -> Result<Vec<Vec<String>>, Error>;
}

impl Client for LiveClient {
    fn run_query(&mut self, query: &str, params: &[&str]) -> Result<Vec<Vec<String>>, Error> {
        let cast: Vec<_> = params.iter().map(|s| s as &(dyn ToSql + Sync)).collect();
        let res = self
            .query(query, &cast)
            .map_err(|err| err.into())
            .map(|rows| {
                rows.iter()
                    .map(|row| {
                        (0..(row.len()))
                            .map(|i| row.get::<usize, String>(i))
                            .collect()
                    })
                    .collect()
            });
        res
    }
}

pub struct Connection<C: Client> {
    sock: C,
    cache_dir: PathBuf,
}

impl Connection<LiveClient> {
    pub fn new() -> Result<Self, Error> {
        // let tls = postgres::tls::native_tls::NativeTls::new()?;
        // let sock = postgres::Connection::connect(POSTGRES, TlsMode::Require(&tls))?;
        // TODO: udd-mirror doesn't support tls
        debug!("Connecting to database");
        let sock = LiveClient::connect(POSTGRES, NoTls)?;
        debug!("Got database connection");

        let cache_dir = dirs::cache_dir()
            .expect("cache directory not found")
            .join("cargo-debstatus");

        fs::create_dir_all(&cache_dir)?;

        Ok(Connection { sock, cache_dir })
    }
}

impl<C: Client> Connection<C> {
    fn cache_path(&self, target: &str, package: &str, version: &Version) -> PathBuf {
        self.cache_dir
            .join(format!("{target}-{package}-{}", version))
    }

    fn check_cache(
        &self,
        target: &str,
        package: &str,
        version: &Version,
    ) -> Result<Option<PkgInfo>, Error> {
        let path = self.cache_path(target, package, version);

        if !path.exists() {
            return Ok(None);
        }

        let buf = fs::read(&path)?;
        // If the cache entry can't be deserialized, it's probably using an old
        // entry format, so let's discard it
        let cache: CacheEntry = match serde_json::from_slice(&buf) {
            Ok(e) => e,
            _ => {
                fs::remove_file(path)?;
                return Ok(None);
            }
        };

        if SystemTime::now().duration_since(cache.from)? > CACHE_EXPIRE {
            Ok(None)
        } else {
            debug!("Found package in cache: {package} -> {:?}", cache.info);
            Ok(Some(cache.info))
        }
    }

    fn write_cache(
        &self,
        target: &str,
        package: &str,
        version: &Version,
        info: &PkgInfo,
    ) -> Result<(), Error> {
        let cache = CacheEntry {
            from: SystemTime::now(),
            info: info.clone(),
        };
        let buf = serde_json::to_vec(&cache)?;
        fs::write(self.cache_path(target, package, version), buf)?;
        Ok(())
    }

    pub fn search(
        &mut self,
        package: &str,
        version: &Version,
        skip_cache: bool,
    ) -> Result<PkgInfo, Error> {
        if !skip_cache {
            if let Some(info) = self.check_cache("sid", package, version)? {
                return Ok(info);
            }
        }

        // config.shell().status("Querying", format!("sid: {}", package))?;
        info!("Querying -> sid (binary): {}", package);
        let mut info = self.search_generic(
            "SELECT version::text FROM packages WHERE package in ($1, $2) AND release='sid';",
            package,
            version,
            PkgType::Binary,
        )?;
        if info.status == PkgStatus::NotFound {
            info!("Querying -> sid (source): {}", package);
            info = self.search_generic(
                "SELECT version::text FROM sources WHERE source in ($1, $2) AND release='sid';",
                package,
                version,
                PkgType::Source,
            )?;
        }

        self.write_cache("sid", package, version, &info)?;
        Ok(info)
    }

    pub fn search_new(
        &mut self,
        package: &str,
        version: &Version,
        skip_cache: bool,
    ) -> Result<PkgInfo, Error> {
        if !skip_cache {
            if let Some(info) = self.check_cache("new", package, version)? {
                return Ok(info);
            }
        }

        // config.shell().status("Querying", format!("new: {}", package))?;
        info!("Querying -> new: {}", package);
        let info = self.search_generic(
            "SELECT version::text FROM new_sources WHERE source in ($1, $2);",
            package,
            version,
            PkgType::Source,
        )?;

        self.write_cache("new", package, version, &info)?;
        Ok(info)
    }

    pub fn search_generic(
        &mut self,
        query: &str,
        package: &str,
        version: &Version,
        pkg_type: PkgType,
    ) -> Result<PkgInfo, Error> {
        let mut info = PkgInfo {
            status: PkgStatus::NotFound,
            version: String::new(),
        };
        let package = package.replace('_', "-");
        let package = package.to_lowercase();
        let semver_version = if version.major == 0 {
            if version.minor == 0 {
                format!("{}.{}.{}", version.major, version.minor, version.patch)
            } else {
                format!("{}.{}", version.major, version.minor)
            }
        } else {
            format!("{}", version.major)
        };
        let names: &[&str] = if pkg_type == PkgType::Binary {
            &[
                &format!("librust-{package}-dev")[..],
                &format!("librust-{package}-{}-dev", semver_version),
            ]
        } else {
            &[
                &format!("rust-{package}"),
                &format!("rust-{package}-{}", semver_version),
            ]
        };
        let rows = self.sock.run_query(query, names)?;

        let version = version.to_string();
        let version = VersionReq::parse(&version)?;
        let semver_version = VersionReq::parse(&semver_version)?;
        for row in &rows {
            let debversion: &str = row
                .first()
                .expect("Each SQL result row should have one entry");

            let debversion = match debversion.find('-') {
                Some(idx) => debversion.split_at(idx).0,
                _ => debversion,
            };

            //println!("{:?} ({:?}) => {:?}", debversion, version, is_compatible(debversion, &version));

            if is_compatible(debversion, &version)? {
                info.version = debversion.to_string();
                info.status = PkgStatus::Found;
                debug!("{package} {:?}", info);
                return Ok(info);
            } else if is_compatible(debversion, &semver_version)? {
                info.version = debversion.to_string();
                info.status = PkgStatus::Compatible;
            } else if info.status == PkgStatus::NotFound {
                info.version = debversion.to_string();
                info.status = PkgStatus::Outdated;
            } else if info.status == PkgStatus::Outdated {
                if let (Ok(existing), Ok(ours)) = (
                    parse_deb_version(&info.version),
                    parse_deb_version(debversion),
                ) {
                    if existing < ours {
                        info.version = debversion.to_string();
                    }
                }
            }
        }

        debug!("{package} {:?}", info);
        Ok(info)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::db::{is_compatible, Connection, PkgStatus, PkgType};
    use anyhow::anyhow;
    use semver::{Version, VersionReq};

    use super::Client;

    /// SQL queries followed by their parameters
    type MockedQuery<'a> = Vec<&'a str>;
    /// Mocked SQL query results
    type ResultRows<'a> = Vec<Vec<&'a str>>;

    struct MockClient<'a> {
        responses: HashMap<MockedQuery<'a>, ResultRows<'a>>,
    }

    impl Client for MockClient<'_> {
        fn run_query(
            &mut self,
            query: &str,
            params: &[&str],
        ) -> anyhow::Result<Vec<Vec<String>>, anyhow::Error> {
            let mut key = vec![query];
            key.extend_from_slice(params);
            self.responses
                .get(&key)
                .map(|v| {
                    v.iter()
                        .map(|row| row.iter().map(|s| s.to_string()).collect())
                        .collect()
                })
                .ok_or(anyhow!(
                    "Unmocked SQL query: {query}, with parameters: [{}]",
                    params.join(", ")
                ))
        }
    }

    fn mock_connection<'a>(
        mocked_responses: &'a [(&str, Vec<&str>, ResultRows<'a>)],
    ) -> Connection<MockClient<'a>> {
        let responses = mocked_responses
            .iter()
            .map(|(query, params, rows)| {
                let mut key = vec![*query];
                for param in params.iter() {
                    key.push(param);
                }
                let value = rows.iter().map(|arr| arr.to_vec()).collect();
                (key, value)
            })
            .collect();
        let mock_client = MockClient { responses };
        let cache_dir =
            tempfile::tempdir().expect("could not create a temporary directory for the cache");

        Connection {
            sock: mock_client,
            cache_dir: cache_dir.into_path(),
        }
    }

    #[test]
    fn is_compatible_with_tilde() {
        assert!(is_compatible(
            "1.0.0~alpha.9",
            &VersionReq::parse("1.0.0-alpha.9").unwrap()
        )
        .unwrap());
    }

    #[test]
    fn is_compatible_with_plus() {
        assert!(is_compatible("4+20231122+dfsg", &VersionReq::parse("4.0.0").unwrap()).unwrap());
    }

    #[test]
    fn is_compatible_follows_semver() {
        assert!(is_compatible("0.1.1", &VersionReq::parse("0.1.0").unwrap()).unwrap());
        assert!(!is_compatible("0.1.0", &VersionReq::parse("0.1.1").unwrap()).unwrap());
        assert!(is_compatible("1.1.0", &VersionReq::parse("1").unwrap()).unwrap());
    }

    #[test]
    fn find_via_lib_package_name() {
        // crate "usvg" is not packaged from the "resvg" source package, not "rust-usvg"
        let mocked_responses = &[
            (
                "SELECT version::text FROM sources WHERE source in ($1, $2) AND release='sid';",
                vec!["rust-usvg", "rust-usvg-0.45"],
                vec![],
            ),
            (
                "SELECT version::text FROM packages WHERE package in ($1, $2) AND release='sid';",
                vec!["librust-usvg-dev", "librust-usvg-0.45-dev"],
                vec![vec!["0.45.0-2"]],
            ),
        ][..];
        let mut db = mock_connection(mocked_responses);
        let info = db
            .search("usvg", &Version::parse("0.45.0").unwrap(), true)
            .unwrap();
        assert_eq!(info.status, PkgStatus::Found);
        assert_eq!(info.version, "0.45.0");
    }

    #[test]
    fn find_via_source_name() {
        // crate "vivid" only provides a binary, no lib
        let mocked_responses = &[
            (
                "SELECT version::text FROM sources WHERE source in ($1, $2) AND release='sid';",
                vec!["rust-vivid", "rust-vivid-0.9"],
                vec![vec!["0.9.0-3"]],
            ),
            (
                "SELECT version::text FROM packages WHERE package in ($1, $2) AND release='sid';",
                vec!["librust-vivid-dev", "librust-vivid-0.9-dev"],
                vec![],
            ),
        ][..];
        let mut db = mock_connection(mocked_responses);
        let info = db
            .search("vivid", &Version::parse("0.9.0").unwrap(), true)
            .unwrap();
        assert_eq!(info.status, PkgStatus::Found);
        assert_eq!(info.version, "0.9.0");
    }

    #[test]
    fn check_version_reqs() {
        // Debian bullseye has rust-serde v1.0.106 and shouldn't be updated anymore
        let query =
            "SELECT version::text FROM sources WHERE source in ($1, $2) AND release='bullseye';";
        let mocked_responses = &[
            (
                query,
                vec!["rust-serde", "rust-serde-1"],
                vec![vec!["1.0.106-1"]],
            ),
            (
                query,
                vec!["rust-serde", "rust-serde-2"],
                vec![vec!["1.0.106-1"]],
            ),
            (query, vec!["rust-notacrate", "rust-notacrate-1"], vec![]),
        ][..];
        let mut db = mock_connection(mocked_responses);
        let info = db
            .search_generic(
                query,
                "serde",
                &Version::parse("1.0.100").unwrap(),
                PkgType::Source,
            )
            .unwrap();
        assert_eq!(info.status, PkgStatus::Found);
        assert_eq!(info.version, "1.0.106");
        let info = db
            .search_generic(
                query,
                "serde",
                &Version::parse("1.0.150").unwrap(),
                PkgType::Source,
            )
            .unwrap();
        assert_eq!(info.status, PkgStatus::Compatible);
        let info = db
            .search_generic(
                query,
                "serde",
                &Version::parse("2.0.0").unwrap(),
                PkgType::Source,
            )
            .unwrap();
        assert_eq!(info.status, PkgStatus::Outdated);
        let info = db
            .search_generic(
                query,
                "notacrate",
                &Version::parse("1.0.0").unwrap(),
                PkgType::Source,
            )
            .unwrap();
        assert_eq!(info.status, PkgStatus::NotFound);
    }

    #[test]
    fn check_zerover_version_reqs() {
        // Debian bookworm has rust-zoxide v0.4.3 and shouldn't be updated anymore
        let query =
            "SELECT version::text FROM sources WHERE source in ($1, $2) AND release='bookworm';";
        let mocked_responses = &[
            (
                query,
                vec!["rust-zoxide", "rust-zoxide-0.4"],
                vec![vec!["0.4.3-5"]],
            ),
            (
                query,
                vec!["rust-zoxide", "rust-zoxide-0.5"],
                vec![vec!["0.4.3-5"]],
            ),
        ][..];
        let mut db = mock_connection(mocked_responses);
        let info = db
            .search_generic(
                query,
                "zoxide",
                &Version::parse("0.4.1").unwrap(),
                PkgType::Source,
            )
            .unwrap();
        assert_eq!(info.status, PkgStatus::Found);
        assert_eq!(info.version, "0.4.3");
        let info = db
            .search_generic(
                query,
                "zoxide",
                &Version::parse("0.4.5").unwrap(),
                PkgType::Source,
            )
            .unwrap();
        assert_eq!(info.status, PkgStatus::Compatible);
        let info = db
            .search_generic(
                query,
                "zoxide",
                &Version::parse("0.5.0").unwrap(),
                PkgType::Source,
            )
            .unwrap();
        assert_eq!(info.status, PkgStatus::Outdated);
    }
}
