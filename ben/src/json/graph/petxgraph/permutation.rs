use super::super::nx_formats::NxAdjEntry;
use super::{PetxGraph, PetxNode};
use petgraph::graph::{Graph, NodeIndex};
use petgraph::visit::{EdgeRef, NodeIndexable};
use serde_json::Value;

/// Apply a node permutation to a `PetxGraph`, returning a new graph with nodes reordered.
///
/// Arguments:
///
/// - `petx_graph`: The input graph to permute.
/// - `order`: A permutation where `order[new_index]` is the `NodeIndex` of the node that should
///   occupy position `new_index` in the output graph. Must be a valid permutation of the graph's
///   node indices.
///
/// Returns:
///
/// - A new `PetxGraph` with nodes in the specified order and edges remapped to the new indices.
///   Edge attributes (including `key` and `attrs`) are preserved; the `NxAdjEntry::id` field is
///   left as-is since `construct_networkx_from_petgraph` overwrites it on export.
pub(in crate::json::graph) fn apply_permutation<Ty>(
    petx_graph: &PetxGraph<Ty>,
    order: &[NodeIndex],
) -> PetxGraph<Ty>
where
    Ty: petgraph::EdgeType,
{
    let graph = &petx_graph.graph;

    // Build old-to-new index mapping.
    let mut old_to_new = vec![NodeIndex::new(0); graph.node_bound()];
    for (new_idx, &old_idx) in order.iter().enumerate() {
        old_to_new[old_idx.index()] = NodeIndex::new(new_idx);
    }

    let mut new_graph =
        Graph::<PetxNode, NxAdjEntry, Ty>::with_capacity(graph.node_count(), graph.edge_count());

    for &old_idx in order {
        new_graph.add_node(graph[old_idx].clone());
    }

    for edge_ref in graph.edge_references() {
        let new_src = old_to_new[edge_ref.source().index()];
        let new_tgt = old_to_new[edge_ref.target().index()];
        new_graph.add_edge(new_src, new_tgt, edge_ref.weight().clone());
    }

    // Relabel __networkx_id__ to match new positions.
    for node_idx in new_graph.node_indices() {
        new_graph[node_idx].attrs.insert(
            "__networkx_id__".to_string(),
            Value::from(node_idx.index() as u64),
        );
    }

    PetxGraph {
        graph_attrs: petx_graph.graph_attrs.clone(),
        graph: new_graph,
    }
}
