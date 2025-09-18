use crate::debian::PackagingProgress;
use cargo_metadata::DependencyKind;
use clap::ValueEnum;
use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;
use std::collections::{HashMap, HashSet};
use std::fmt::Display;

use crate::debian::Pkg;
use crate::graph::Graph;

#[derive(ValueEnum, Clone, Default, Debug, PartialEq, Eq)]
pub enum DependencyFilter {
    /// Show all dependencies (default)
    #[default]
    All,
    /// Only show missing dependencies, which require going through the NEW queue.
    /// Missing dependencies of crates that are newer in Debian are ignored.
    Missing,
}

impl Display for DependencyFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            DependencyFilter::All => "all",
            DependencyFilter::Missing => "missing",
        })
    }
}

impl DependencyFilter {
    /// Run the filter on a graph, mutating it.
    pub fn run(&self, graph: &mut Graph) {
        match self {
            DependencyFilter::All => (),
            DependencyFilter::Missing => {
                let mut visited = HashSet::new();
                let mut cache = HashMap::new();
                for node_index in graph.graph.node_indices() {
                    has_missing_dependency(graph, node_index, &mut visited, &mut cache);
                }

                graph.graph.retain_edges(|graph, edge| {
                    (*graph)
                        .edge_endpoints(edge)
                        .is_some_and(|(source, target)| {
                            if let (Some(&a), Some(&b)) = (cache.get(&source), cache.get(&target)) {
                                a && b
                            } else {
                                false
                            }
                        })
                });
            }
        }
    }
}

fn has_missing_dependency(
    graph: &Graph,
    node_index: NodeIndex<u32>,
    visited: &mut HashSet<NodeIndex<u32>>,
    cache: &mut HashMap<NodeIndex<u32>, bool>,
) -> bool {
    if let Some(result) = cache.get(&node_index) {
        *result
    } else if visited.contains(&node_index) {
        // dependency loop: we don't recurse, to avoid a stack overflow.
        false
    } else {
        visited.insert(node_index);
        let edges = graph
            .graph
            .edges_directed(node_index, petgraph::Direction::Outgoing);
        let package: &Pkg = &graph.graph[node_index];
        let mut missing_dep_found = !package.in_debian();
        for edge in edges {
            let edge_kind = graph
                .graph
                .edge_weight(edge.id())
                .unwrap_or(&DependencyKind::Unknown);
            if ![
                DependencyKind::Build,
                DependencyKind::Development,
                DependencyKind::Build,
            ]
            .contains(edge_kind)
            {
                continue;
            }
            let dep_has_missing_dep = has_missing_dependency(graph, edge.target(), visited, cache);
            missing_dep_found = missing_dep_found || dep_has_missing_dep;
        }
        // If the package is newer in debian, ignore any missing dependencies of it,
        // because there is no point packaging the dependencies of an older version of it.
        if matches!(package.packaging_status(), PackagingProgress::NeedsPatching) {
            missing_dep_found = false;
        }
        cache.insert(node_index, missing_dep_found);
        missing_dep_found
    }
}
