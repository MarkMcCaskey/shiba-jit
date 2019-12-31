//! Functions and types for register allocation.
//!
//! The code here is generic and may be used with any architecture.

use crate::ir::*;
use petgraph::{graph::NodeIndex, stable_graph::StableGraph, Directed};
use std::collections::*;

pub struct GraphData {
    pub unreduced_index_map: BTreeMap<BasicBlockIndex, NodeIndex>,
    pub graph: StableGraph<BasicBlockIndex, (), Directed>,
    pub reduced_graph: StableGraph<BasicBlockIndex, (), Directed>,
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
    let reduced_graph = compute_reduced_graph(&graph, start_ni);

    println!("{:?}", petgraph::dot::Dot::new(&reduced_graph));

    GraphData {
        unreduced_index_map: node_lookup,
        graph,
        reduced_graph,
    }
}

/// Creates a copy of the graph with the back-edges removed
pub fn compute_reduced_graph(
    graph: &StableGraph<BasicBlockIndex, (), Directed>,
    start: NodeIndex,
) -> StableGraph<BasicBlockIndex, (), Directed> {
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

    reduced_graph
}
