use super::super::errors::NxPetgraphError;
use super::super::nx_formats::{NxAdjEntry, NxGraphAdjFormat, NxNode};
use super::{PetxGraph, PetxNode};
use petgraph::graph::{Graph, NodeIndex};
use petgraph::visit::{EdgeRef, IntoNodeReferences};
use petgraph::{Directed, Undirected};
use std::collections::{HashMap, HashSet};

/// Convert an [`NxNode`] into a [`PetxNode`].
///
/// The node's `id` field is moved into the attribute map under the reserved key `"__networkx_id__"`
/// so it can be recovered later.
///
/// # Arguments
///
/// * `nx_node` - The NetworkX node to convert. Consumed by this function.
///
/// # Returns
///
/// A [`PetxNode`] whose `attrs` map contains all original attributes plus `"__networkx_id__"`.
pub(in crate::json::graph) fn nx_node_to_petx_node(nx_node: NxNode) -> PetxNode {
    let mut attrs = nx_node.attrs;
    attrs.insert("__networkx_id__".to_string(), nx_node.id);
    PetxNode { attrs }
}

/// Convert a [`PetxNode`] back into an [`NxNode`].
///
/// The `"__networkx_id__"` entry is removed from the attribute map and placed back into the `id`
/// field.
///
/// # Arguments
///
/// * `petx_node` - The petgraph node to convert.
///
/// # Returns
///
/// An [`NxNode`] with the original id and attributes restored.
///
/// # Errors
///
/// Returns [`NxPetgraphError::Other`] if the node has no `"__networkx_id__"` attribute.
pub(in crate::json::graph) fn petx_node_to_nx_node(
    petx_node: &PetxNode,
) -> Result<NxNode, NxPetgraphError> {
    let mut attrs = petx_node.attrs.clone();
    let id = attrs.remove("__networkx_id__").ok_or_else(|| {
        NxPetgraphError::Other("missing __networkx_id__ on petgraph node".to_string())
    })?;

    Ok(NxNode { id, attrs })
}

/// Build a [`PetxGraph`] from a parsed [`NxGraphAdjFormat`].
///
/// Nodes are added in order and edges are extracted from the adjacency lists. For undirected
/// graphs, duplicate `(u,v)` / `(v,u)` entries are deduplicated so each edge is stored only once.
///
/// # Arguments
///
/// * `nx_graph` - The parsed NetworkX graph. Consumed by this function.
/// * `is_directed` - Whether the target graph should be directed. Must match `nx_graph.directed`.
///
/// # Returns
///
/// A [`PetxGraph<Ty>`] with the same topology and attributes.
///
/// # Errors
///
/// Returns an [`NxPetgraphError`] if:
/// * `nx_graph.directed` does not match `is_directed`.
/// * The `nodes` and `adjacency` arrays differ in length.
/// * A node id appears more than once.
/// * An adjacency entry references a node id not present in `nodes`.
fn build_petgraph_from_networkx<Ty>(
    nx_graph: NxGraphAdjFormat,
    is_directed: bool,
) -> Result<PetxGraph<Ty>, NxPetgraphError>
where
    Ty: petgraph::EdgeType,
{
    if nx_graph.directed != is_directed {
        return Err(NxPetgraphError::DirectednessMismatch {
            expected_directed: is_directed,
            found_directed: nx_graph.directed,
        });
    }

    if nx_graph.nodes.len() != nx_graph.adjacency.len() {
        return Err(NxPetgraphError::NodeAdjacencyLengthMismatch {
            n_nodes: nx_graph.nodes.len(),
            n_adjacency_items: nx_graph.adjacency.len(),
        });
    }

    let NxGraphAdjFormat {
        directed: _,
        multigraph: _,
        graph: graph_attrs,
        nodes,
        adjacency,
    } = nx_graph;

    let mut graph = Graph::<PetxNode, NxAdjEntry, Ty>::with_capacity(nodes.len(), 0);
    let mut node_id_to_index: HashMap<serde_json::Value, NodeIndex> =
        HashMap::with_capacity(nodes.len());

    for node in nodes {
        if node_id_to_index.contains_key(&node.id) {
            return Err(NxPetgraphError::DuplicateNodeId(node.id));
        }

        let node_id = node.id.clone();
        let petx_node = nx_node_to_petx_node(node);
        let index = graph.add_node(petx_node);
        node_id_to_index.insert(node_id, index);
    }

    // NetworkX adjacency format is a list of adjacency lists, where the i-th adjacency list
    // corresponds to the i-th node in the nodes list.
    //
    // For undirected graphs, the format may contain both (u, v) and (v, u), so we track
    // canonicalized edge endpoint pairs and only add each undirected edge once.
    let mut seen_undirected_edges: HashSet<(String, String, Option<String>)> = HashSet::new();

    for (source_idx_orig, neighbors) in adjacency.into_iter().enumerate() {
        let source_idx = NodeIndex::new(source_idx_orig);
        // Adjacency length was validated against nodes length above.
        let source_node = graph
            .node_weight(source_idx)
            .expect("adjacency length validated against nodes length");

        // __networkx_id__ is always inserted by nx_node_to_petx_node.
        let source_id = source_node
            .attrs
            .get("__networkx_id__")
            .expect("__networkx_id__ always set by nx_node_to_petx_node");

        // serde_json::Value is always serializable.
        let source_key =
            serde_json::to_string(source_id).expect("serde_json::Value always serializes");

        for edge in neighbors {
            let target_id = &edge.id;
            let target_idx = node_id_to_index
                .get(target_id)
                .ok_or_else(|| NxPetgraphError::MissingNeighborNode(target_id.clone()))?;

            if is_directed {
                graph.add_edge(source_idx, *target_idx, edge);
            } else {
                // serde_json::Value is always serializable.
                let target_key =
                    serde_json::to_string(target_id).expect("serde_json::Value always serializes");

                let edge_key_str = edge
                    .key
                    .as_ref()
                    .and_then(|key| serde_json::to_string(key).ok());

                let canonical = if source_key <= target_key {
                    (source_key.clone(), target_key, edge_key_str)
                } else {
                    (target_key, source_key.clone(), edge_key_str)
                };

                if seen_undirected_edges.insert(canonical) {
                    graph.add_edge(source_idx, *target_idx, edge);
                }
            }
        }
    }

    Ok(PetxGraph { graph_attrs, graph })
}

/// Check whether a graph contains parallel (multi) edges.
///
/// Two edges are considered parallel if they connect the same pair of endpoints. For undirected
/// graphs, `(u,v)` and `(v,u)` are the same pair.
///
/// # Arguments
///
/// * `graph` - The petgraph graph to inspect.
///
/// # Returns
///
/// `true` if any pair of nodes is connected by more than one edge.
pub(in crate::json::graph) fn graph_has_parallel_edges<Ty>(
    graph: &Graph<PetxNode, NxAdjEntry, Ty>,
) -> bool
where
    Ty: petgraph::EdgeType,
{
    let mut seen_endpoint_pairs: HashSet<(usize, usize)> = HashSet::new();

    for edge_ref in graph.edge_references() {
        let source_idx = edge_ref.source().index();
        let target_idx = edge_ref.target().index();

        let endpoint_pair = if graph.is_directed() || source_idx <= target_idx {
            (source_idx, target_idx)
        } else {
            (target_idx, source_idx)
        };

        if !seen_endpoint_pairs.insert(endpoint_pair) {
            return true;
        }
    }

    false
}

/// Convert a [`PetxGraph`] back into an [`NxGraphAdjFormat`].
///
/// Nodes are emitted in petgraph index order. For undirected graphs, each edge appears in both
/// endpoints' adjacency lists (except self-loops, which appear only once). The `multigraph` flag is
/// set automatically based on whether parallel edges exist.
///
/// # Arguments
///
/// * `petx_graph` - The petgraph-backed graph to convert.
/// * `is_directed` - Whether the output should be marked as directed.
///
/// # Returns
///
/// An [`NxGraphAdjFormat`] ready for JSON serialization.
///
/// # Errors
///
/// Returns [`NxPetgraphError::Other`] if any node is missing its `"__networkx_id__"` attribute.
fn construct_networkx_from_petgraph<Ty>(
    petx_graph: &PetxGraph<Ty>,
    is_directed: bool,
) -> Result<NxGraphAdjFormat, NxPetgraphError>
where
    Ty: petgraph::EdgeType,
{
    let graph = &petx_graph.graph;
    let graph_attrs = petx_graph.graph_attrs.clone();
    let mut nodes = Vec::with_capacity(graph.node_count());
    let mut adjacency = vec![Vec::<NxAdjEntry>::new(); graph.node_count()];

    for (_, node) in graph.node_references() {
        nodes.push(petx_node_to_nx_node(node)?);
    }

    for edge_ref in graph.edge_references() {
        let source_idx = edge_ref.source().index();
        let target_idx = edge_ref.target().index();
        let mut adj_data = edge_ref.weight().clone();

        adj_data.id = nodes[target_idx].id.clone();
        adjacency[source_idx].push(adj_data.clone());

        if !is_directed && source_idx != target_idx {
            let mut reverse_adj_data = adj_data;
            reverse_adj_data.id = nodes[source_idx].id.clone();
            adjacency[target_idx].push(reverse_adj_data);
        }
    }

    Ok(NxGraphAdjFormat {
        directed: is_directed,
        multigraph: graph_has_parallel_edges(graph),
        graph: graph_attrs,
        nodes,
        adjacency,
    })
}

impl TryFrom<NxGraphAdjFormat> for PetxGraph<Directed> {
    type Error = NxPetgraphError;

    fn try_from(nx_graph: NxGraphAdjFormat) -> Result<Self, Self::Error> {
        build_petgraph_from_networkx::<Directed>(nx_graph, true)
    }
}

impl TryFrom<NxGraphAdjFormat> for PetxGraph<Undirected> {
    type Error = NxPetgraphError;

    fn try_from(nx_graph: NxGraphAdjFormat) -> Result<Self, Self::Error> {
        build_petgraph_from_networkx::<Undirected>(nx_graph, false)
    }
}

impl TryFrom<&PetxGraph<Directed>> for NxGraphAdjFormat {
    type Error = NxPetgraphError;

    fn try_from(petx_graph: &PetxGraph<Directed>) -> Result<Self, Self::Error> {
        construct_networkx_from_petgraph(petx_graph, true)
    }
}

impl TryFrom<&PetxGraph<Undirected>> for NxGraphAdjFormat {
    type Error = NxPetgraphError;

    fn try_from(petx_graph: &PetxGraph<Undirected>) -> Result<Self, Self::Error> {
        construct_networkx_from_petgraph(petx_graph, false)
    }
}
