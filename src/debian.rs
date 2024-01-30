use crate::db::{Connection, PkgStatus};
use crate::errors::*;
use crate::graph::Graph;
use cargo_metadata::{Package, PackageId, Source};
use indicatif::ProgressBar;
use indicatif::ProgressStyle;
use semver::Version;
use std::path::PathBuf;
use std::thread;

const QUERY_THREADS: usize = 24;

#[derive(Debug, Clone)]
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

pub enum PackagingProgress {
    Available,
    AvailableInNew,
    NeedsUpdate,
    Missing,
}

use std::fmt;

impl fmt::Display for PackagingProgress {
    //! Generate icons to display the packaging progress.
    //! They should all take the same width when printed in a terminal
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let icon = match self {
            PackagingProgress::Available => "  ",
            PackagingProgress::AvailableInNew => " N",
            PackagingProgress::NeedsUpdate => "⌛",
            PackagingProgress::Missing => "🔴",
        };
        write!(f, "{}", icon)
    }
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

    pub fn show_dependencies(&self) -> bool {
        if !self.in_debian() {
            return true;
        }

        if let Some(deb) = &self.debinfo {
            !deb.exact_match && (deb.outdated || !deb.compatible)
        } else {
            true
        }
    }

    pub fn packaging_status(&self) -> PackagingProgress {
        if let Some(deb) = &self.debinfo {
            if deb.in_unstable {
                if deb.compatible {
                    // Available at an older yet compatible version
                    PackagingProgress::Available
                } else if deb.outdated {
                    PackagingProgress::NeedsUpdate
                } else {
                    PackagingProgress::Available
                }
            } else if deb.in_new {
                if deb.compatible {
                    PackagingProgress::AvailableInNew
                } else if deb.outdated {
                    // Outdated; in the NEW queue
                    PackagingProgress::NeedsUpdate
                } else {
                    PackagingProgress::AvailableInNew
                }
            } else if deb.outdated {
                PackagingProgress::NeedsUpdate
            } else {
                PackagingProgress::Missing
            }
        } else {
            PackagingProgress::Missing
        }
    }
}

#[derive(Debug, Clone)]
pub struct DebianInfo {
    pub in_unstable: bool,
    pub in_new: bool,
    pub outdated: bool,
    pub compatible: bool,
    pub exact_match: bool,
    pub version: String,
}

fn run_task(db: &mut Connection, pkg: Pkg) -> Result<DebianInfo> {
    let mut deb = DebianInfo {
        in_unstable: false,
        in_new: false,
        outdated: false,
        compatible: false,
        exact_match: false,
        version: String::new(),
    };

    let mut info = db.search(&pkg.name, &pkg.version).unwrap();
    if info.status == PkgStatus::NotFound {
        info = db.search_new(&pkg.name, &pkg.version).unwrap();
        if info.status != PkgStatus::NotFound {
            deb.in_new = true;
            deb.version = info.version;
        }
    } else {
        deb.in_unstable = true;
        deb.version = info.version;
    }

    match info.status {
        PkgStatus::Outdated => deb.outdated = true,
        PkgStatus::Compatible => deb.compatible = true,
        PkgStatus::Found => deb.exact_match = true,
        _ => (),
    }

    Ok(deb)
}

pub fn populate(graph: &mut Graph) -> Result<(), Error> {
    let (task_tx, task_rx) = crossbeam_channel::unbounded();
    let (return_tx, return_rx) = crossbeam_channel::unbounded();

    info!("Creating thread-pool");
    for _ in 0..QUERY_THREADS {
        let task_rx = task_rx.clone();
        let return_tx = return_tx.clone();

        thread::spawn(move || {
            let mut db = match Connection::new() {
                Ok(db) => db,
                Err(err) => {
                    return_tx.send(Err(err)).unwrap();
                    return;
                }
            };

            for (idx, pkg) in task_rx {
                let deb = run_task(&mut db, pkg);
                if return_tx.send(Ok((idx, deb))).is_err() {
                    break;
                }
            }
        });
    }

    info!("Getting node indices");
    let idxs = graph.graph.node_indices().collect::<Vec<_>>();
    let jobs = idxs.len();
    debug!("Found node indices: {}", jobs);

    for idx in idxs {
        if let Some(pkg) = graph.graph.node_weight_mut(idx) {
            debug!("Adding job for {:?}: {:?}", idx, pkg);
            let pkg = pkg.clone();
            task_tx.send((idx, pkg)).unwrap();
        }
    }

    info!("Processing debian results");

    let pb = ProgressBar::new(jobs as u64)
        .with_style(
            ProgressStyle::default_bar()
                .template("[{pos:.green}/{len:.green}] {prefix:.bold} {wide_bar}")?,
        )
        .with_prefix("Resolving debian packages");
    pb.tick();

    for result in return_rx.iter().take(jobs) {
        let result = result.context("A worker crashed")?;

        let idx = result.0;
        let deb = result.1?;

        if let Some(pkg) = graph.graph.node_weight_mut(idx) {
            pkg.debinfo = Some(deb);
        }
        pb.inc(1);
    }

    pb.finish_and_clear();

    Ok(())
}
