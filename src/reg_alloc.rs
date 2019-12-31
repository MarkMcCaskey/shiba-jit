//! Functions and types for register allocation.
//!
//! The code here is generic and may be used with any architecture.

use crate::ir::*;
use petgraph::{
    graph::NodeIndex,
    stable_graph::StableGraph,
    visit::{depth_first_search, DfsEvent},
    Directed,
};
use std::collections::*;

pub struct GraphData {
    pub index_map: BTreeMap<BasicBlockIndex, NodeIndex>,
    /// how deep from the root each node is, used to compute real back-edges
    pub depth_map: BTreeMap<NodeIndex, u32>,
    pub graph: StableGraph<BasicBlockIndex, (), Directed>,
    pub reduced_graph: StableGraph<BasicBlockIndex, (), Directed>,
}

impl GraphData {
    /// Returns the "transitive closure" of reachibilty on the reduced graph,
    /// that is the set of all nodes that are reachable without back-edges
    pub fn compute_reduced_reachability_and_back_edges(
        &self,
    ) -> (
        BTreeMap<NodeIndex, BTreeSet<NodeIndex>>,
        BTreeMap<NodeIndex, BTreeSet<NodeIndex>>,
    ) {
        let mut rr_out = BTreeMap::new();
        let mut back_edge_out = BTreeMap::new();

        for node_idx in self.reduced_graph.node_indices() {
            let mut connected_nodes: BTreeSet<NodeIndex> = BTreeSet::new();
            let mut back_edge_targets: BTreeSet<NodeIndex> = BTreeSet::new();
            depth_first_search(&self.reduced_graph, Some(node_idx), |event| match event {
                // TODO: it looks like DfsEvents will let us avoid computing the reduced graph
                DfsEvent::Discover(n, _) => {
                    connected_nodes.insert(n);
                }
                DfsEvent::CrossForwardEdge(s, d)
                | DfsEvent::TreeEdge(s, d)
                | DfsEvent::BackEdge(s, d) => {
                    // manually check back-edges, this back edge is relative to
                    // the start of the dfs which if not the root, is wrong
                    if self.depth_map[&s] > self.depth_map[&d] {
                        back_edge_targets.insert(d);
                    }
                }
                _ => (),
            });
            rr_out.insert(node_idx, connected_nodes);
            back_edge_out.insert(node_idx, back_edge_targets);
        }

        (rr_out, back_edge_out)
    }
}

pub fn compute_graph(bbm: &BasicBlockManager) -> GraphData {
    let mut graph = StableGraph::new();
    let mut node_lookup: BTreeMap<BasicBlockIndex, NodeIndex> = BTreeMap::new();
    for (bbi, _bb) in bbm.iterate_basic_blocks() {
        let ni = graph.add_node(bbi);
        node_lookup.insert(bbi, ni);
    }
    for (bbi, bb) in bbm.iterate_basic_blocks() {
        let ni = node_lookup[&bbi];
        for parent in bb.iter_parents() {
            let parent_ni = node_lookup[parent];
            // update to avoid duplicates
            graph.update_edge(parent_ni, ni, ());
        }
        for exit in bb.iter_exits() {
            let exit_ni = node_lookup[exit];
            // update to avoid duplicates
            graph.update_edge(ni, exit_ni, ());
        }
    }
    println!("{:?}", petgraph::dot::Dot::new(&graph));

    let start_ni = node_lookup[&bbm.start];
    let (reduced_graph, depth_map) = compute_reduced_graph_and_depth_map(&graph, start_ni);

    println!("{:?}", petgraph::dot::Dot::new(&reduced_graph));

    GraphData {
        index_map: node_lookup,
        depth_map,
        graph,
        reduced_graph,
    }
}

/// Creates a copy of the graph with the back-edges removed
pub fn compute_reduced_graph_and_depth_map(
    graph: &StableGraph<BasicBlockIndex, (), Directed>,
    start: NodeIndex,
) -> (
    StableGraph<BasicBlockIndex, (), Directed>,
    BTreeMap<NodeIndex, u32>,
) {
    let mut reduced_graph = graph.clone();
    let mut stack = VecDeque::new();
    let mut seen: BTreeMap<NodeIndex, u32> = BTreeMap::new();
    seen.insert(start, 0);
    // second num is generation, we use this to detect back-references
    stack.push_back((start, 0));

    while let Some((node, generation)) = stack.pop_front() {
        for neigh in graph.neighbors(node) {
            if seen.contains_key(&neigh) {
                if seen[&neigh] < generation {
                    let edge = reduced_graph.find_edge(node, neigh).unwrap();
                    reduced_graph.remove_edge(edge);
                }
            } else {
                seen.insert(neigh, generation);
                stack.push_back((neigh, generation + 1))
            }
        }
    }

    (reduced_graph, seen)
}
