#![allow(clippy::too_many_arguments)]

use crate::args::{Args, Charset};
use crate::debian::Pkg;
use crate::errors::*;
use crate::format::{self, Pattern};
use crate::graph::Graph;
use cargo_metadata::{DependencyKind, PackageId};
use petgraph::visit::EdgeRef;
use petgraph::EdgeDirection;
use semver::Version;
use std::collections::{HashMap, HashSet};
use std::io::Write;

#[derive(Clone, Copy)]
enum Prefix {
    None,
    Indent,
    Depth,
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

pub fn print<W: Write>(args: &Args, graph: &Graph, writer: &mut W) -> Result<(), Error> {
    let format = Pattern::new(&args.format)?;

    let direction = if args.invert || args.duplicates {
        EdgeDirection::Incoming
    } else {
        EdgeDirection::Outgoing
    };

    let symbols = match args.charset {
        Charset::Utf8 => &UTF8_SYMBOLS,
        Charset::Ascii => &ASCII_SYMBOLS,
    };

    let prefix = if args.prefix_depth {
        Prefix::Depth
    } else if args.no_indent {
        Prefix::None
    } else {
        Prefix::Indent
    };

    if args.duplicates {
        for (i, package) in find_duplicates(graph).iter().enumerate() {
            if i != 0 {
                writeln!(writer)?;
            }

            let root = &graph.graph[graph.nodes[*package]];
            print_tree(
                graph, root, &format, direction, symbols, prefix, args.all, args.json, writer,
            )?;
        }
    } else {
        let root = match &args.package {
            Some(package) => find_package(package, graph)?,
            None => graph.root.as_ref().ok_or_else(|| {
                anyhow!("this command requires running against an actual package in this workspace")
            })?,
        };
        let root = &graph.graph[graph.nodes[root]];

        print_tree(
            graph, root, &format, direction, symbols, prefix, args.all, args.json, writer,
        )?;
    }

    Ok(())
}

fn find_package<'a>(package: &str, graph: &'a Graph) -> Result<&'a PackageId, Error> {
    let mut it = package.split(':');
    let name = it.next().unwrap();
    let version = it
        .next()
        .map(Version::parse)
        .transpose()
        .context("error parsing package version")?;

    let mut candidates = vec![];
    for idx in graph.graph.node_indices() {
        let package = &graph.graph[idx];
        if package.name != name {
            continue;
        }

        if let Some(version) = &version {
            if package.version != *version {
                continue;
            }
        }

        candidates.push(package);
    }

    if candidates.is_empty() {
        Err(anyhow!("no crates found for package `{}`", package))
    } else if candidates.len() > 1 {
        let specs = candidates
            .iter()
            .map(|p| format!("{}:{}", p.name, p.version))
            .collect::<Vec<_>>()
            .join(", ");
        Err(anyhow!(
            "multiple crates found for package `{}`: {}",
            package,
            specs,
        ))
    } else {
        Ok(&candidates[0].id)
    }
}

fn find_duplicates(graph: &Graph) -> Vec<&PackageId> {
    let mut packages = HashMap::new();

    for idx in graph.graph.node_indices() {
        let package = &graph.graph[idx];
        packages
            .entry(&package.name)
            .or_insert_with(Vec::new)
            .push(&package.id);
    }

    let mut duplicates = vec![];
    for ids in packages.values() {
        if ids.len() > 1 {
            duplicates.extend(ids.iter().cloned());
        }
    }

    duplicates.sort();
    duplicates
}

fn print_tree<'a, W: Write>(
    graph: &'a Graph,
    root: &'a Pkg,
    format: &Pattern,
    direction: EdgeDirection,
    symbols: &Symbols,
    prefix: Prefix,
    all: bool,
    json: bool,
    writer: &mut W,
) -> Result<(), Error> {
    let mut visited_deps = HashSet::new();
    let mut levels_continue = vec![];

    print_package(
        graph,
        root,
        format,
        direction,
        symbols,
        prefix,
        all,
        json,
        &mut visited_deps,
        &mut levels_continue,
        writer,
    )
}

fn print_package<'a, W: Write>(
    graph: &'a Graph,
    package: &'a Pkg,
    format: &Pattern,
    direction: EdgeDirection,
    symbols: &Symbols,
    prefix: Prefix,
    all: bool,
    json: bool,
    visited_deps: &mut HashSet<&'a PackageId>,
    levels_continue: &mut Vec<bool>,
    writer: &mut W,
) -> Result<(), Error> {
    let treeline = {
        let mut line = "".to_string();
        line.push_str(&format!(" {} ", &package.packaging_status()));
        match prefix {
            Prefix::Depth => line.push_str(&format!("{}", levels_continue.len())),
            Prefix::Indent => {
                if let Some((last_continues, rest)) = levels_continue.split_last() {
                    for continues in rest {
                        let c = if *continues { symbols.down } else { " " };
                        line.push_str(&format!("{c}   "));
                    }

                    let c = if *last_continues {
                        symbols.tee
                    } else {
                        symbols.ell
                    };
                    line.push_str(&format!("{0}{1}{1} ", c, symbols.right));
                }
            }
            Prefix::None => {}
        }
        line
    };

    if json {
        writeln!(
            writer,
            "{}",
            format::json::display(package, levels_continue.len())?
        )?;
    } else {
        let pkg_status_s = format::human::display(format, package)?;
        writeln!(writer, "{treeline}{pkg_status_s}")?;
    }

    if !all && !package.show_dependencies() && !levels_continue.is_empty()
        || !visited_deps.insert(&package.id)
    {
        return Ok(());
    }

    for kind in &[
        DependencyKind::Normal,
        DependencyKind::Build,
        DependencyKind::Development,
    ] {
        print_dependencies(
            graph,
            package,
            format,
            direction,
            symbols,
            prefix,
            all,
            json,
            visited_deps,
            levels_continue,
            *kind,
            writer,
        )?;
    }

    Ok(())
}

fn print_dependencies<'a, W: Write>(
    graph: &'a Graph,
    package: &'a Pkg,
    format: &Pattern,
    direction: EdgeDirection,
    symbols: &Symbols,
    prefix: Prefix,
    all: bool,
    json: bool,
    visited_deps: &mut HashSet<&'a PackageId>,
    levels_continue: &mut Vec<bool>,
    kind: DependencyKind,
    writer: &mut W,
) -> Result<(), Error> {
    let idx = graph.nodes[&package.id];
    let mut deps = vec![];
    for edge in graph.graph.edges_directed(idx, direction) {
        if *edge.weight() != kind {
            continue;
        }

        let dep = match direction {
            EdgeDirection::Incoming => &graph.graph[edge.source()],
            EdgeDirection::Outgoing => &graph.graph[edge.target()],
        };
        deps.push(dep);
    }

    if deps.is_empty() {
        return Ok(());
    }

    // ensure a consistent output ordering
    deps.sort_by_key(|p| &p.id);

    if !json {
        let name = match kind {
            DependencyKind::Normal => None,
            DependencyKind::Build => Some("[build-dependencies]"),
            DependencyKind::Development => Some("[dev-dependencies]"),
            _ => unreachable!(),
        };

        if let Prefix::Indent = prefix {
            if let Some(name) = name {
                // start with padding used by packaging status icons
                write!(writer, "    ")?;

                // print tree graph parts
                for continues in &**levels_continue {
                    let c = if *continues { symbols.down } else { " " };
                    write!(writer, "{c}   ")?;
                }

                // print the actual texts
                writeln!(writer, "{name}")?;
            }
        }
    }

    let mut it = deps.iter().peekable();
    while let Some(dependency) = it.next() {
        levels_continue.push(it.peek().is_some());
        print_package(
            graph,
            dependency,
            format,
            direction,
            symbols,
            prefix,
            all,
            json,
            visited_deps,
            levels_continue,
            writer,
        )?;
        levels_continue.pop();
    }

    Ok(())
}

#[cfg(test)]
mod tests {

    use anyhow::Error;
    use cargo_metadata::Metadata;
    use clap::Parser;

    use super::print;
    use crate::{args::Args, graph};

    #[test]
    fn print_tree_without_dependency_loop() -> Result<(), Error> {
        let args = Args::parse_from(["debstatus"]);
        let metadata: Metadata = serde_json::from_str(include_str!(
            "../tests/data/cargo_metadata_without_loop.json"
        ))?;
        let graph = graph::build(&args, metadata)?;
        let mut buffer = Vec::new();

        print(&args, &graph, &mut buffer)?;

        let expected = r#" 🔴 cargotest v0.1.0 (/tmp/cargotest)
 🔴 └── crossbeam-channel v0.5.15
 🔴     └── crossbeam-utils v0.8.21
"#;
        assert_eq!(String::from_utf8(buffer)?, expected);
        Ok(())
    }

    #[test]
    fn print_tree_with_dependency_loop() -> Result<(), Error> {
        let args = Args::parse_from(["debstatus"]);
        let metadata: Metadata =
            serde_json::from_str(include_str!("../tests/data/cargo_metadata_with_loop.json"))?;
        let graph = graph::build(&args, metadata)?;
        let mut buffer = Vec::new();

        print(&args, &graph, &mut buffer)?;

        let expected = r#" 🔴 cargotest v0.1.0 (/tmp/cargotest)
 🔴 └── crossbeam-channel v0.5.15
 🔴     └── crossbeam-utils v0.8.21
 🔴         └── crossbeam-channel v0.5.15
"#;
        assert_eq!(String::from_utf8(buffer)?, expected);
        Ok(())
    }
}
