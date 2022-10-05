use crate::db::{Connection, CrateStatus};
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

    pub debinfo: Option<CrateStatus>,
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
            deb.in_debian()
        } else {
            false
        }
    }
}

fn run_task(db: &mut Connection, pkg: Pkg) -> Result<CrateStatus> {
    let mut result = db.search(&pkg.name, &pkg.version.to_string()).unwrap();

    if result == CrateStatus::Missing {
        result = db.search_new(&pkg.name, &pkg.version.to_string()).unwrap();
    }

    Ok(result)
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
                .template("[{pos:.green}/{len:.green}] {prefix:.bold} {wide_bar}"),
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
