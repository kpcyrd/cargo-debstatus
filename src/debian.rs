use crate::db::Connection;
use crate::graph::Graph;
use anyhow::Error;
use cargo_metadata::{Package, PackageId, Source};
use semver::Version;
use std::path::PathBuf;

pub struct Pkg {
    pub id: PackageId,
    pub name: String,
    pub version: Version,
    pub source: Option<Source>,
    pub manifest_path: PathBuf,
    pub license: Option<String>,
    pub repository: Option<String>,

    pub debinfo: Option<DebianInfo>,
}

impl Pkg {
    pub fn new(pkg: Package) -> Pkg {
        Pkg {
            id: pkg.id,
            name: pkg.name,
            version: pkg.version,
            source: pkg.source,
            manifest_path: pkg.manifest_path.into(),
            license: pkg.license,
            repository: pkg.repository,

            debinfo: None,
        }
    }

    pub fn in_debian(&self) -> bool {
        if let Some(deb) = &self.debinfo {
            deb.in_unstable || deb.in_new
        } else {
            false
        }
    }
}

pub struct DebianInfo {
    pub in_unstable: bool,
    pub in_new: bool,
    pub outdated: bool,
}

pub fn populate(graph: &mut Graph) -> Result<(), Error> {
    let idxs = graph.graph.node_indices().collect::<Vec<_>>();

    let mut db = Connection::new()?;

    for idx in idxs {
        if let Some(mut pkg) = graph.graph.node_weight_mut(idx) {
            let mut deb = DebianInfo {
                in_unstable: false,
                in_new: false,
                outdated: false,
            };

            if db.search(&pkg.name, &pkg.version.to_string())? {
                deb.in_unstable = true;
            } else if db.search_new(&pkg.name, &pkg.version.to_string())? {
                deb.in_new = true;
            }

            // TODO: outdated is missing

            pkg.debinfo = Some(deb);
        }
    }

    Ok(())
}
