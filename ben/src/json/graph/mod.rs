//! JSON graph helpers used by relabeling workflows.

use std::collections::HashMap;
use std::io::{self, Error, ErrorKind, Read, Result, Write};

mod errors;
mod mlc;
mod nx_formats;
mod petxgraph;
mod rcm;

use errors::NxPetgraphError;
use nx_formats::NxGraphAdjFormat;
use petgraph::graph::NodeIndex;
use petgraph::{Directed, Undirected};
use petxgraph::PetxGraph;

/// Topology-based graph ordering methods supported by `reben`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphOrderingMethod {
    /// Order nodes using recursive multilevel clustering.
    MultiLevelCluster,
    /// Order nodes using Reverse Cuthill-McKee.
    ReverseCuthillMckee,
}

/// Sorts a JSON-formatted NetworkX graph file by a node attribute.
///
/// Reads a NetworkX adjacency-format JSON graph, reorders nodes so that they
/// are sorted by the given attribute key, and writes the reordered graph back
/// as JSON.
///
/// # Arguments
///
/// * `reader` - A source of JSON bytes in NetworkX adjacency format.
/// * `writer` - Destination for the reordered JSON output.
/// * `key` - The node attribute name to sort by. Use `"id"` to sort by the
///   NetworkX node id.
///
/// # Returns
///
/// A map from each original node id to its new (post-sort) node id.
pub fn sort_json_file_by_key<R: Read, W: Write>(
    reader: R,
    writer: W,
    key: &str,
) -> Result<HashMap<usize, usize>> {
    tracing::trace!("Loading JSON file...");
    let nx_graph: NxGraphAdjFormat = serde_json::from_reader(reader)?;
    let original_ids = extract_usize_ids(&nx_graph)?;

    tracing::trace!("Sorting JSON file by key: {}", key);
    let (result, order) = if nx_graph.directed {
        let mut petx: PetxGraph<Directed> = nx_graph.try_into().map_err(nx_err)?;
        let order = petxgraph::sort_by_key(&mut petx, key);
        let result: NxGraphAdjFormat = (&petx).try_into().map_err(nx_err)?;
        (result, order)
    } else {
        let mut petx: PetxGraph<Undirected> = nx_graph.try_into().map_err(nx_err)?;
        let order = petxgraph::sort_by_key(&mut petx, key);
        let result: NxGraphAdjFormat = (&petx).try_into().map_err(nx_err)?;
        (result, order)
    };

    write_nx_graph(writer, &result)?;
    Ok(build_id_mapping(&original_ids, &order))
}

/// Reorder a JSON-formatted NetworkX graph file using a topology-based method.
///
/// Reads a NetworkX adjacency-format JSON graph, reorders nodes using the
/// specified graph ordering algorithm, and writes the reordered graph back
/// as JSON.
///
/// # Arguments
///
/// * `reader` - A source of JSON bytes in NetworkX adjacency format.
/// * `writer` - Destination for the reordered JSON output.
/// * `method` - The topology-based ordering algorithm to apply.
///
/// # Returns
///
/// A map from each original node id to its new (post-sort) node id.
pub fn sort_json_file_by_ordering<R: Read, W: Write>(
    reader: R,
    writer: W,
    method: GraphOrderingMethod,
) -> Result<HashMap<usize, usize>> {
    tracing::trace!("Loading JSON file...");
    let nx_graph: NxGraphAdjFormat = serde_json::from_reader(reader)?;
    let original_ids = extract_usize_ids(&nx_graph)?;

    tracing::trace!("Sorting JSON file by ordering method: {:?}", method);
    let (result, order) = if nx_graph.directed {
        let mut petx: PetxGraph<Directed> = nx_graph.try_into().map_err(nx_err)?;
        let order = run_ordering_method(&mut petx, method);
        let result: NxGraphAdjFormat = (&petx).try_into().map_err(nx_err)?;
        (result, order)
    } else {
        let mut petx: PetxGraph<Undirected> = nx_graph.try_into().map_err(nx_err)?;
        let order = run_ordering_method(&mut petx, method);
        let result: NxGraphAdjFormat = (&petx).try_into().map_err(nx_err)?;
        (result, order)
    };

    write_nx_graph(writer, &result)?;
    Ok(build_id_mapping(&original_ids, &order))
}

/// Dispatch to the appropriate ordering algorithm.
///
/// # Arguments
///
/// * `petx` - The graph to reorder in place.
/// * `method` - Which ordering algorithm to run.
///
/// # Returns
///
/// The permutation that was applied: `order[new_index]` is the `NodeIndex`
/// the node occupied before reordering.
fn run_ordering_method<Ty: petgraph::EdgeType>(
    petx: &mut PetxGraph<Ty>,
    method: GraphOrderingMethod,
) -> Vec<NodeIndex> {
    match method {
        GraphOrderingMethod::MultiLevelCluster => mlc::apply_multi_level_clustering(petx),
        GraphOrderingMethod::ReverseCuthillMckee => rcm::apply_reverse_cuthill_mckee(petx),
    }
}

/// Extract the integer node ids from an [`NxGraphAdjFormat`] in order.
///
/// # Arguments
///
/// * `nx_graph` - The parsed NetworkX graph whose node ids are extracted.
///
/// # Returns
///
/// A vector of `usize` ids in the same order as `nx_graph.nodes`.
///
/// # Errors
///
/// Returns an error if any node id is not a non-negative integer.
fn extract_usize_ids(nx_graph: &NxGraphAdjFormat) -> io::Result<Vec<usize>> {
    nx_graph
        .nodes
        .iter()
        .map(|n| {
            n.id.as_u64().map(|v| v as usize).ok_or_else(|| {
                Error::new(
                    ErrorKind::InvalidData,
                    format!("Node id is not an unsigned integer: {}", n.id),
                )
            })
        })
        .collect()
}

/// Build a mapping from original node ids to new positional ids.
///
/// # Arguments
///
/// * `original_ids` - The node ids before reordering, indexed by the old
///   node position.
/// * `order` - The permutation that was applied: `order[new_index]` is the
///   old `NodeIndex`.
///
/// # Returns
///
/// A map where `mapping[original_id] == new_positional_id`.
fn build_id_mapping(original_ids: &[usize], order: &[NodeIndex]) -> HashMap<usize, usize> {
    let mut mapping = HashMap::with_capacity(order.len());
    for (new_idx, &old_node_idx) in order.iter().enumerate() {
        mapping.insert(original_ids[old_node_idx.index()], new_idx);
    }
    mapping
}

/// Serialize an [`NxGraphAdjFormat`] to JSON and write it to the given writer.
///
/// # Arguments
///
/// * `writer` - Destination for the JSON bytes.
/// * `nx_graph` - The graph to serialize.
///
/// # Returns
///
/// `Ok(())` on success, or an I/O error if serialization or writing fails.
fn write_nx_graph<W: Write>(mut writer: W, nx_graph: &NxGraphAdjFormat) -> io::Result<()> {
    let rendered =
        serde_json::to_string(nx_graph).map_err(|err| Error::new(ErrorKind::InvalidData, err))?;
    writer.write_all(rendered.as_bytes())
}

/// Convert an [`NxPetgraphError`] into a [`std::io::Error`] with
/// [`ErrorKind::InvalidData`].
///
/// # Arguments
///
/// * `e` - The conversion error to wrap.
///
/// # Returns
///
/// An `io::Error` carrying `e` as its inner cause.
fn nx_err(e: NxPetgraphError) -> Error {
    Error::new(ErrorKind::InvalidData, e)
}

#[cfg(test)]
mod tests;
