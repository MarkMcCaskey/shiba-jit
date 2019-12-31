//! Functions and types for register allocation.
//!
//! The code here is generic and may be used with any architecture.
//!
//! The liveness querying is from and/or inspired by
//! ["Fast Liveness Checking for SSA-Form Programs"][paper] by
//! Benoit Boissinot, Sebastian Hack, Daniel Grund, Benoît Dupont de Dinechin,
//! and Fabrice Rastello.
//!
//! [paper]: https://dl.acm.org/doi/10.1145/1356058.1356064

use crate::ir::*;
use petgraph::{
    algo::dominators::{simple_fast, Dominators},
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
    pub root: NodeIndex,
}

pub struct GraphQuery {
    graph_data: GraphData,
    dominators: Dominators<NodeIndex>,
    reduced_reachability: BTreeMap<NodeIndex, BTreeSet<NodeIndex>>,
    back_edges: BTreeMap<NodeIndex, BTreeSet<NodeIndex>>,
    /// Map showing where a register is used
    use_map: BTreeMap<RegisterIndex, BTreeSet<NodeIndex>>,
    /// Map showing where a register was defined
    define_map: BTreeMap<RegisterIndex, NodeIndex>,
}

impl GraphQuery {
    pub fn new(graph_data: GraphData, bbm: &BasicBlockManager) -> Self {
        let (reduced_reachability, back_edges) =
            graph_data.compute_reduced_reachability_and_back_edges();
        let dominators = simple_fast(&graph_data.graph, graph_data.root);
        let mut use_map: BTreeMap<RegisterIndex, BTreeSet<NodeIndex>> = BTreeMap::new();
        let mut define_map: BTreeMap<RegisterIndex, NodeIndex> = BTreeMap::new();
        for (idx, block) in bbm.iterate_basic_blocks() {
            let ni = graph_data.index_map[&idx];
            for reg_idx in block.iter_used_registers() {
                let ent = use_map.entry(*reg_idx).or_default();
                ent.insert(ni);
            }
            for reg_idx in block.iter_defined_registers() {
                let result = define_map.insert(*reg_idx, ni);
                assert_eq!(result, None);
            }
        }
        Self {
            graph_data,
            dominators,
            reduced_reachability,
            back_edges,
            use_map,
            define_map,
        }
    }

    pub fn is_live_in(&self, idx: RegisterIndex) -> bool {
        let ni = self.define_map[&idx];
        let strict_dominators = self
            .dominators
            .strict_dominators(ni)
            .unwrap()
            .collect::<BTreeSet<_>>();
        let uses_set = &self.use_map[&idx];
        for t in self.back_edges[&ni].intersection(&strict_dominators) {
            if self.reduced_reachability[&t]
                .intersection(&uses_set)
                .count()
                != 0
            {
                return true;
            }
        }
        false
    }

    pub fn is_live_out(&self, idx: RegisterIndex, node: BasicBlockIndex) -> bool {
        let ni = self.define_map[&idx];
        let node_ni = self.graph_data.index_map[&node];
        if self.define_map[&idx] == node_ni {
            return self.use_map[&idx].iter().filter(|n| **n != node_ni).count() != 0;
        }
        // can avoid allocation here
        let registers_node_dominates_node = self
            .dominators
            .strict_dominators(node_ni)
            .unwrap()
            .any(|e| e == ni);
        if registers_node_dominates_node {
            let strict_dominators = self
                .dominators
                .strict_dominators(ni)
                .unwrap()
                .collect::<BTreeSet<_>>();
            for t in self.back_edges[&ni].intersection(&strict_dominators) {
                let mut u = self.use_map[&idx].clone();
                if *t == node_ni && !self.back_edges[&node_ni].contains(&node_ni) {
                    u.remove(&node_ni);
                }
                if self.reduced_reachability[&t].intersection(&u).count() != 0 {
                    return true;
                }
            }
        }
        false
    }
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
        root: start_ni,
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
