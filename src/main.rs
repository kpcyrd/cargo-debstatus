#![allow(unused_imports)]
extern crate cargo;
extern crate env_logger;
extern crate failure;
extern crate petgraph;
extern crate semver;
extern crate colored;
extern crate postgres;
extern crate serde_json;
extern crate dirs;

#[macro_use]
extern crate structopt;
#[macro_use]
extern crate serde_derive;

use cargo::core::Registry;
use cargo::core::Dependency;
use cargo::core::SourceId;
use semver::Version;

use cargo::core::dependency::Kind;
use cargo::core::manifest::ManifestMetadata;
use cargo::core::package::PackageSet;
use cargo::core::registry::PackageRegistry;
use cargo::core::resolver::Method;
use cargo::core::shell::Shell;
use cargo::core::{Package, PackageId, Resolve, Workspace};
use cargo::ops;
use cargo::util::{self, important_paths, CargoResult, Cfg, Rustc};
use cargo::{CliResult, Config};
use colored::Colorize;
use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;
use petgraph::EdgeDirection;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::str::{self, FromStr};
use structopt::clap::AppSettings;
use structopt::StructOpt;

use db::Connection;
mod db;

#[derive(StructOpt)]
#[structopt(bin_name = "cargo")]
enum Opts {
    #[structopt(
        name = "debstatus",
        raw(
            setting = "AppSettings::UnifiedHelpMessage",
            setting = "AppSettings::DeriveDisplayOrder",
            setting = "AppSettings::DontCollapseArgsInUsage"
        )
    )]
    /// Visualize the dependency graph for debian packaging
    DebStatus(Args),
}

#[derive(StructOpt)]
struct Args {
    #[structopt(long = "package", short = "p", value_name = "SPEC")]
    /// Package to be used as the root of the tree
    package: Option<String>,
    #[structopt(long = "target", value_name = "TARGET")]
    /// Set the target triple
    target: Option<String>,
    /// Directory for all generated artifacts
    #[structopt(long = "target-dir", value_name = "DIRECTORY", parse(from_os_str))]
    target_dir: Option<PathBuf>,
    #[structopt(long = "manifest-path", value_name = "PATH", parse(from_os_str))]
    /// Path to Cargo.toml
    manifest_path: Option<PathBuf>,
    #[structopt(long = "no-indent")]
    /// Display the dependencies as a list (rather than a tree)
    no_indent: bool,
    #[structopt(long = "prefix-depth")]
    /// Display the dependencies as a list (rather than a tree), but prefixed with the depth
    prefix_depth: bool,
    #[structopt(long = "all", short = "a")]
    /// Don't truncate dependencies that have already been displayed
    all: bool,
    #[structopt(long = "charset", value_name = "CHARSET", default_value = "utf8")]
    /// Character set to use in output: utf8, ascii
    charset: Charset,
    #[structopt(long = "verbose", short = "v", parse(from_occurrences))]
    /// Use verbose output (-vv very verbose/build.rs output)
    verbose: u32,
    #[structopt(long = "quiet", short = "q")]
    /// No output printed to stdout other than the tree
    quiet: Option<bool>,
    #[structopt(long = "color", value_name = "WHEN")]
    /// Coloring: auto, always, never
    color: Option<String>,
    #[structopt(long = "frozen")]
    /// Require Cargo.lock and cache are up to date
    frozen: bool,
    #[structopt(long = "locked")]
    /// Require Cargo.lock is up to date
    locked: bool,
    #[structopt(short = "Z", value_name = "FLAG")]
    /// Unstable (nightly-only) flags to Cargo
    unstable_flags: Vec<String>,
}

enum Charset {
    Utf8,
    Ascii,
}

#[derive(Clone, Copy)]
enum Prefix {
    None,
    Indent,
    Depth,
}

impl FromStr for Charset {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Charset, &'static str> {
        match s {
            "utf8" => Ok(Charset::Utf8),
            "ascii" => Ok(Charset::Ascii),
            _ => Err("invalid charset"),
        }
    }
}

struct Symbols {
    down: &'static str,
    tee: &'static str,
    ell: &'static str,
    right: &'static str,
}

static UTF8_SYMBOLS: Symbols = Symbols {
    down: "│",
    tee: "├",
    ell: "└",
    right: "─",
};

static ASCII_SYMBOLS: Symbols = Symbols {
    down: "|",
    tee: "|",
    ell: "`",
    right: "-",
};

fn main() {
    env_logger::init();

    let mut config = match Config::default() {
        Ok(cfg) => cfg,
        Err(e) => {
            let mut shell = Shell::new();
            cargo::exit_with_error(e.into(), &mut shell)
        }
    };

    let Opts::DebStatus(args) = Opts::from_args();

    if let Err(e) = real_main(args, &mut config) {
        let mut shell = Shell::new();
        cargo::exit_with_error(e.into(), &mut shell)
    }
}

fn real_main(args: Args, config: &mut Config) -> CliResult {
    config.configure(
        args.verbose,
        args.quiet,
        &args.color,
        args.frozen,
        args.locked,
        &args.target_dir,
        &args.unstable_flags,
    )?;

    let workspace = workspace(config, args.manifest_path)?;
    let package = workspace.current()?;
    let mut registry = registry(config, &package)?;
    let (packages, resolve) = resolve(
        &mut registry,
        &workspace,
    )?;
    let ids = packages.package_ids().cloned().collect::<Vec<_>>();

    let db = Connection::new()?;
    let debian = find_in_debian(&config, &db, &ids)?;
    // println!("debian: {:?}", debian);

    let outdated = find_outdated(&mut registry, &config, &ids)?;
    // println!("outdated: {:?}", outdated);

    let packages = registry.get(&ids)?;

    let root = match args.package {
        Some(ref pkg) => resolve.query(pkg)?,
        None => package.package_id(),
    };

    let rustc = config.rustc(Some(&workspace))?;

    let cfgs = get_cfgs(&rustc, &args.target)?;
    let graph = build_graph(
        &resolve,
        &packages,
        package.package_id(),
        None,
        cfgs.as_ref().map(|r| &**r),
    )?;

    let direction = EdgeDirection::Outgoing;

    let symbols = match args.charset {
        Charset::Ascii => &ASCII_SYMBOLS,
        Charset::Utf8 => &UTF8_SYMBOLS,
    };

    let prefix = if args.prefix_depth {
        Prefix::Depth
    } else if args.no_indent {
        Prefix::None
    } else {
        Prefix::Indent
    };

    print_tree(root, &graph, &outdated, &debian, direction, symbols, prefix, args.all);

    Ok(())
}

fn find_outdated(registry: &mut PackageRegistry, config: &Config, ids: &[PackageId]) -> CargoResult<HashSet<String>> {
    let crates_io = SourceId::crates_io(config)?;

    let mut outdated = HashSet::new();

    for id in ids {
        if id.source_id().is_registry() {
            let latest_version = find_latest_version(registry, &crates_io, &id.name())?;

            if *id.version() != latest_version {
                // println!("outdated: {:?} {} -> {}", id.name(), id.version(), latest_version);
                outdated.insert(id.to_string());
            }
        }
    }

    Ok(outdated)
}

fn find_latest_version(registry: &mut PackageRegistry, crates_io: &SourceId, name: &str) -> CargoResult<Version> {
    let versions = registry.query_vec(&Dependency::parse_no_deprecated(name, None, &crates_io)?, false)?;
    let empty = Version::from_str("0.0.0").unwrap();
    let latest_version = versions.iter()
                           .filter(|x| !x.version().is_prerelease())
                           .map(|x| x.version())
                           .max().unwrap_or(&empty);

    Ok(latest_version.to_owned())
}

fn find_in_debian(config: &Config, sock: &Connection, ids: &[PackageId]) -> CargoResult<(HashSet<String>, HashSet<String>)> {
    let mut sid = HashSet::new();
    let mut new = HashSet::new();

    for id in ids {
        if id.source_id().is_registry() {
            let name = id.name();
            let version = id.version().to_string();

            if sock.search(&config, &name, &version)? {
                sid.insert(id.to_string());
            } else if sock.search_new(&config, &name, &version)? {
                new.insert(id.to_string());
            }
        }
    }

    Ok((sid, new))
}

fn get_cfgs(rustc: &Rustc, target: &Option<String>) -> CargoResult<Option<Vec<Cfg>>> {
    let mut process = util::process(&rustc.path);
    process.arg("--print=cfg").env_remove("RUST_LOG");
    if let Some(ref s) = *target {
        process.arg("--target").arg(s);
    }

    let output = match process.exec_with_output() {
        Ok(output) => output,
        Err(_) => return Ok(None),
    };
    let output = str::from_utf8(&output.stdout).unwrap();
    let lines = output.lines();
    Ok(Some(
        lines.map(Cfg::from_str).collect::<CargoResult<Vec<_>>>()?,
    ))
}

fn workspace(config: &Config, manifest_path: Option<PathBuf>) -> CargoResult<Workspace> {
    let root = match manifest_path {
        Some(path) => path,
        None => important_paths::find_root_manifest_for_wd(config.cwd())?,
    };
    Workspace::new(&root, config)
}

fn registry<'a>(config: &'a Config, package: &Package) -> CargoResult<PackageRegistry<'a>> {
    let mut registry = PackageRegistry::new(config)?;
    registry.add_sources(&[package.package_id().source_id().clone()])?;
    Ok(registry)
}

fn resolve<'a, 'cfg>(
    registry: &mut PackageRegistry<'cfg>,
    workspace: &'a Workspace<'cfg>,
) -> CargoResult<(PackageSet<'a>, Resolve)> {
    let (packages, resolve) = ops::resolve_ws(workspace)?;

    let method = Method::Required {
        dev_deps: false,
        features: &[],
        all_features: true,
        uses_default_features: true,
    };

    let resolve = ops::resolve_with_previous(
        registry,
        workspace,
        method,
        Some(&resolve),
        None,
        &[],
        true,
        true,
    )?;
    Ok((packages, resolve))
}

struct Node<'a> {
    id: &'a PackageId,
}

struct Graph<'a> {
    graph: petgraph::Graph<Node<'a>, Kind>,
    nodes: HashMap<&'a PackageId, NodeIndex>,
}

fn build_graph<'a>(
    resolve: &'a Resolve,
    packages: &'a PackageSet,
    root: &'a PackageId,
    target: Option<&str>,
    cfgs: Option<&[Cfg]>,
) -> CargoResult<Graph<'a>> {
    let mut graph = Graph {
        graph: petgraph::Graph::new(),
        nodes: HashMap::new(),
    };
    let node = Node {
        id: root,
    };
    graph.nodes.insert(root, graph.graph.add_node(node));

    let mut pending = vec![root];

    while let Some(pkg_id) = pending.pop() {
        let idx = graph.nodes[&pkg_id];
        let pkg = packages.get_one(pkg_id)?;

        for raw_dep_id in resolve.deps_not_replaced(pkg_id) {
            let it = pkg
                .dependencies()
                .iter()
                .filter(|d| d.matches_id(raw_dep_id))
                .filter(|d| {
                    d.platform()
                        .and_then(|p| target.map(|t| p.matches(t, cfgs)))
                        .unwrap_or(true)
                });
            let dep_id = match resolve.replacement(raw_dep_id) {
                Some(id) => id,
                None => raw_dep_id,
            };
            for dep in it {
                let dep_idx = match graph.nodes.entry(dep_id) {
                    Entry::Occupied(e) => *e.get(),
                    Entry::Vacant(e) => {
                        pending.push(dep_id);
                        let node = Node {
                            id: dep_id,
                        };
                        *e.insert(graph.graph.add_node(node))
                    }
                };
                graph.graph.add_edge(idx, dep_idx, dep.kind());
            }
        }
    }

    Ok(graph)
}

fn print_tree<'a>(
    package: &'a PackageId,
    graph: &Graph<'a>,
    outdated: &HashSet<String>,
    debian: &(HashSet<String>, HashSet<String>),
    direction: EdgeDirection,
    symbols: &Symbols,
    prefix: Prefix,
    all: bool,
) {
    let mut levels_continue = vec![];

    let node = &graph.graph[graph.nodes[&package]];
    print_dependency(
        node,
        &graph,
        outdated,
        debian,
        direction,
        symbols,
        &mut levels_continue,
        prefix,
        all,
    );
}

fn print_dependency<'a>(
    package: &Node<'a>,
    graph: &Graph<'a>,
    outdated: &HashSet<String>,
    debian: &(HashSet<String>, HashSet<String>),
    direction: EdgeDirection,
    symbols: &Symbols,
    levels_continue: &mut Vec<bool>,
    prefix: Prefix,
    all: bool,
) {
    match prefix {
        Prefix::Depth => print!("{} ", levels_continue.len()),
        Prefix::Indent => {
            if let Some((&last_continues, rest)) = levels_continue.split_last() {
                for &continues in rest {
                    let c = if continues { symbols.down } else { " " };
                    print!("{}   ", c);
                }

                let c = if last_continues {
                    symbols.tee
                } else {
                    symbols.ell
                };
                print!("{0}{1}{1} ", c, symbols.right);
            }
        }
        Prefix::None => (),
    }

    let fmt = package.id.to_string();

    if debian.0.contains(&fmt) {
        println!("{} (in debian)", fmt.green());
        // TODO: option to display the whole tree
        return;
    } else if debian.1.contains(&fmt) {
        println!("{} (in debian NEW queue)", fmt.blue());
        // TODO: option to display the whole tree
        return;
    } else if outdated.contains(&fmt) {
        println!("{} (outdated)", fmt.yellow());
    } else {
        println!("{}", fmt);
    }

    let mut normal = vec![];
    for edge in graph
        .graph
        .edges_directed(graph.nodes[&package.id], direction)
    {
        let dep = match direction {
            EdgeDirection::Incoming => &graph.graph[edge.source()],
            EdgeDirection::Outgoing => &graph.graph[edge.target()],
        };
        match *edge.weight() {
            Kind::Normal => normal.push(dep),
            Kind::Build => normal.push(dep),
            Kind::Development => (),
        }
    }

    print_dependency_kind(
        normal,
        graph,
        outdated,
        debian,
        direction,
        symbols,
        levels_continue,
        prefix,
        all,
    );
}

fn print_dependency_kind<'a>(
    mut deps: Vec<&Node<'a>>,
    graph: &Graph<'a>,
    outdated: &HashSet<String>,
    debian: &(HashSet<String>, HashSet<String>),
    direction: EdgeDirection,
    symbols: &Symbols,
    levels_continue: &mut Vec<bool>,
    prefix: Prefix,
    all: bool,
) {
    if deps.is_empty() {
        return;
    }

    // Resolve uses Hash data types internally but we want consistent output ordering
    deps.sort_by_key(|n| n.id);

    let mut it = deps.iter().peekable();
    while let Some(dependency) = it.next() {
        levels_continue.push(it.peek().is_some());
        print_dependency(
            dependency,
            graph,
            outdated,
            debian,
            direction,
            symbols,
            levels_continue,
            prefix,
            all,
        );
        levels_continue.pop();
    }
}
