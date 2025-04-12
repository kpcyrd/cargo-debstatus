use crate::args::Args;
use crate::debian::Pkg;
use crate::errors::*;
use cargo_metadata::{DependencyKind, Metadata, PackageId};
use petgraph::graph::NodeIndex;
use petgraph::stable_graph::StableGraph;
use petgraph::visit::Dfs;
use std::collections::{HashMap, HashSet};

pub struct Graph {
    pub graph: StableGraph<Pkg, DependencyKind>,
    pub nodes: HashMap<PackageId, NodeIndex>,
    pub roots: Vec<PackageId>,
}

pub fn build(args: &Args, metadata: Metadata) -> Result<Graph, Error> {
    let resolve = metadata.resolve.unwrap();

    let mut graph = Graph {
        graph: StableGraph::new(),
        nodes: HashMap::new(),
        roots: metadata.workspace_members,
    };

    for package in metadata.packages {
        let id = package.id.clone();
        let index = graph.graph.add_node(Pkg::new(package));
        graph.nodes.insert(id, index);
    }

    for node in resolve.nodes {
        if node.deps.len() != node.dependencies.len() {
            return Err(anyhow!("cargo tree requires cargo 1.41 or newer"));
        }

        let from = graph.nodes[&node.id];
        for dep in node.deps {
            if dep.dep_kinds.is_empty() {
                return Err(anyhow!("cargo tree requires cargo 1.41 or newer"));
            }

            // https://github.com/rust-lang/cargo/issues/7752
            let mut kinds = vec![];
            for kind in dep.dep_kinds {
                if !kinds.contains(&kind.kind) {
                    kinds.push(kind.kind);
                }
            }

            let to = graph.nodes[&dep.pkg];
            for kind in kinds {
                if args.no_dev_dependencies && kind == DependencyKind::Development {
                    continue;
                }

                graph.graph.add_edge(from, to, kind);
            }
        }
    }

    if let Some(included) = &args.included {
        // only keep roots included on the command line
        let included: Vec<&str> = included.split(",").collect();
        let included_roots: Vec<PackageId> = resolve_roots(&graph, &included);
        graph.roots.retain(|root| included_roots.contains(root));

        // optionally collapse workspace
        if args.collapse_workspace {
            collapse_workspace(&mut graph);
        }
    } else {
        // optionally collapse workspace
        if args.collapse_workspace {
            collapse_workspace(&mut graph);
        }

        // prune roots excluded on the command line
        if let Some(excluded) = &args.excluded {
            let excluded: Vec<&str> = excluded.split(",").collect();
            let excluded_roots: Vec<PackageId> = resolve_roots(&graph, &excluded);
            graph.roots.retain(|root| !excluded_roots.contains(root));
        }
    }

    // prune nodes not reachable from the root packages (directionally)
    let mut dfs = Dfs::empty(&graph.graph);
    graph.roots.iter().for_each(|root| {
        dfs.move_to(graph.nodes[root]);
        while dfs.next(&graph.graph).is_some() {}
    });

    let g = &mut graph.graph;
    graph.nodes.retain(|_, idx| {
        if !dfs.discovered.contains(idx.index()) {
            g.remove_node(*idx);
            false
        } else {
            true
        }
    });

    Ok(graph)
}

// prune roots reachable from other roots (directionally), that is,
// do not count as roots workspace members which are dependencies
// of other workspace members
fn collapse_workspace(graph: &mut Graph) {
    let mut droots = HashSet::new();
    for root in &graph.roots {
        let mut dfs = Dfs::new(&graph.graph, graph.nodes[root]);
        while dfs.next(&graph.graph).is_some() {}
        droots.extend(
            graph.roots.iter().filter(|&droot| {
                droot != root && dfs.discovered.contains(graph.nodes[droot].index())
            }),
        );
    }
    let disc: Vec<PackageId> = droots.iter().map(|&package| package.clone()).collect();
    graph.roots.retain(|root| !disc.contains(root));
}

fn resolve_roots(graph: &Graph, roots: &[&str]) -> Vec<PackageId> {
    graph
        .roots
        .iter()
        .filter(|root| {
            roots.contains(
                &graph
                    .graph
                    .node_weight(graph.nodes[root])
                    .unwrap()
                    .name
                    .as_str(),
            )
        })
        .cloned()
        .collect()
}
