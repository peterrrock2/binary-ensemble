mod nx_convert;
mod permutation;
mod sort;

use super::nx_formats::NxAdjEntry;
use petgraph::graph::Graph;
#[cfg(test)]
use petgraph::graph::{DiGraph, UnGraph};
#[cfg(test)]
use petgraph::{Directed, Undirected};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// A single node in a [`PetxGraph`].
///
/// All NetworkX node attributes are stored in `attrs`, including the original
/// node id under the reserved key `"__networkx_id__"`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct PetxNode {
    /// Node attributes. Always contains `"__networkx_id__"` holding the
    /// original (or current) NetworkX node id as a [`Value`].
    pub attrs: BTreeMap<String, Value>,
}

/// A petgraph-backed graph that mirrors a NetworkX adjacency-format graph.
///
/// The type parameter `Ty` is either [`Directed`] or [`Undirected`] and
/// determines the edge semantics of the underlying [`Graph`].
///
/// Graph-level attributes (the `"graph"` array in the NetworkX JSON) are
/// stored alongside the petgraph [`Graph`] so they survive roundtrips.
#[derive(Debug, Clone)]
pub(crate) struct PetxGraph<Ty>
where
    Ty: petgraph::EdgeType,
{
    /// Graph-level key/value attributes from the NetworkX JSON `"graph"` field.
    pub graph_attrs: Vec<(String, Value)>,
    /// The underlying petgraph graph. Nodes carry [`PetxNode`] weights and
    /// edges carry [`NxAdjEntry`] weights.
    pub graph: Graph<PetxNode, NxAdjEntry, Ty>,
}

/// Convenience alias for a directed [`PetxGraph`].
#[cfg(test)]
pub(crate) type PetxDiGraph = PetxGraph<Directed>;
/// Convenience alias for an undirected [`PetxGraph`].
#[cfg(test)]
pub(crate) type PetxUnGraph = PetxGraph<Undirected>;
/// Convenience alias for the inner directed petgraph type.
#[cfg(test)]
pub(crate) type PetxDiInnerGraph = DiGraph<PetxNode, NxAdjEntry>;
/// Convenience alias for the inner undirected petgraph type.
#[cfg(test)]
pub(crate) type PetxUnInnerGraph = UnGraph<PetxNode, NxAdjEntry>;

pub(in crate::json::graph) use permutation::apply_permutation;
pub(in crate::json::graph) use sort::sort_by_key;

#[cfg(test)]
pub(in crate::json::graph) use nx_convert::{
    graph_has_parallel_edges, nx_node_to_petx_node, petx_node_to_nx_node,
};
