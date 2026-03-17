//! JSON graph helpers used by relabeling workflows.

use crate::progress;
use serde_json::{json, Value};
use std::cmp::{Ordering, Reverse};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{self, Read, Result, Write};
use std::result::Result as StdResult;

/// Topology-based graph ordering methods supported by `reben`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphOrderingMethod {
    /// Order nodes using recursive multilevel clustering.
    MultiLevelCluster,
    /// Order nodes using Reverse Cuthill-McKee.
    ReverseCuthillMckee,
}

#[derive(Clone)]
struct GraphJson {
    data: Value,
    nodes: Vec<Value>,
    adjacency: Vec<Vec<Value>>,
    node_ids: Vec<usize>,
    adjacency_indices: Vec<Vec<usize>>,
}

impl GraphJson {
    /// Deserialize a NetworkX node-link JSON graph from a reader.
    ///
    /// # Arguments
    ///
    /// * `reader` - A source implementing `Read` that provides the JSON data.
    ///
    /// # Returns
    ///
    /// Returns a parsed `GraphJson` with precomputed node ids and adjacency indices.
    fn from_reader<R: Read>(reader: R) -> io::Result<Self> {
        let data: Value = serde_json::from_reader(reader)?;
        let nodes = data["nodes"].as_array().cloned().unwrap_or_default();
        let adjacency = data["adjacency"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .map(|row| row.as_array().cloned().unwrap_or_default())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let node_ids = nodes
            .iter()
            .map(parse_node_id)
            .collect::<io::Result<Vec<_>>>()?;
        let id_to_index = node_ids
            .iter()
            .enumerate()
            .map(|(idx, &id)| (id, idx))
            .collect::<HashMap<_, _>>();
        let adjacency_indices = adjacency
            .iter()
            .map(|row| {
                row.iter()
                    .map(|link| {
                        let id = parse_link_id(link)?;
                        id_to_index.get(&id).copied().ok_or_else(|| {
                            io::Error::new(
                                io::ErrorKind::InvalidData,
                                format!("Adjacency references unknown node id {id}"),
                            )
                        })
                    })
                    .collect::<io::Result<Vec<_>>>()
            })
            .collect::<io::Result<Vec<_>>>()?;

        Ok(Self {
            data,
            nodes,
            adjacency,
            node_ids,
            adjacency_indices,
        })
    }
}

/// Sorts a JSON-formatted NetworkX graph file by a key.
///
/// # Arguments
///
/// * `reader` - The source JSON graph in the NetworkX node-link style used by
///   the relabeling workflow.
/// * `writer` - The destination for the sorted JSON graph.
/// * `key` - The node attribute used to determine the new ordering.
///
/// # Returns
///
/// Returns a map from the original node id to the new node id.
pub fn sort_json_file_by_key<R: Read, W: Write>(
    reader: R,
    writer: W,
    key: &str,
) -> Result<HashMap<usize, usize>> {
    tracing::trace!("Loading JSON file...");
    let graph = GraphJson::from_reader(reader)?;
    let mut order: Vec<usize> = (0..graph.nodes.len()).collect();

    tracing::trace!("Sorting JSON file by key: {}", key);
    order.sort_by(|&a, &b| compare_node_key(&graph.nodes[a], &graph.nodes[b], key));

    reorder_graph(graph, order, writer)
}

/// Reorder a JSON-formatted NetworkX graph file using a topology-based method.
///
/// # Arguments
///
/// * `reader` - The source JSON graph in the NetworkX node-link style used by
///   the relabeling workflow.
/// * `writer` - The destination for the reordered JSON graph.
/// * `method` - The topology-based ordering algorithm to apply.
///
/// # Returns
///
/// Returns a map from the original node id to the new node id.
pub fn sort_json_file_by_ordering<R: Read, W: Write>(
    reader: R,
    writer: W,
    method: GraphOrderingMethod,
) -> Result<HashMap<usize, usize>> {
    tracing::trace!("Loading JSON file...");
    let graph = GraphJson::from_reader(reader)?;
    tracing::trace!("Sorting JSON file by ordering method: {:?}", method);

    let order = match method {
        GraphOrderingMethod::MultiLevelCluster => multi_level_cluster_order(&graph),
        GraphOrderingMethod::ReverseCuthillMckee => reverse_cuthill_mckee_order(&graph),
    };

    reorder_graph(graph, order, writer)
}

/// Extract the `id` field from a node JSON value as a `usize`.
///
/// # Arguments
///
/// * `node` - A JSON value representing a graph node.
///
/// # Returns
///
/// Returns the node id, or an error if the field is missing or not an unsigned integer.
fn parse_node_id(node: &Value) -> io::Result<usize> {
    node["id"].as_u64().map(|v| v as usize).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Node id is not an unsigned integer: {}", node["id"]),
        )
    })
}

/// Extract the `id` field from an adjacency link JSON value as a `usize`.
///
/// # Arguments
///
/// * `link` - A JSON value representing an adjacency link (edge target).
///
/// # Returns
///
/// Returns the target node id, or an error if the field is missing or not an unsigned integer.
fn parse_link_id(link: &Value) -> io::Result<usize> {
    link["id"].as_u64().map(|v| v as usize).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Edge target id is not an unsigned integer: {}", link["id"]),
        )
    })
}

/// Compare two nodes by a named attribute, using numeric ordering when possible.
///
/// # Arguments
///
/// * `a` - The first node JSON value.
/// * `b` - The second node JSON value.
/// * `key` - The attribute name to compare.
///
/// # Returns
///
/// Returns the ordering between the two nodes based on the attribute value.
fn compare_node_key(a: &Value, b: &Value, key: &str) -> Ordering {
    let extract_value = |val: &Value| -> StdResult<u64, String> {
        match &val[key] {
            Value::String(s) => s.parse::<u64>().map_err(|_| s.clone()),
            Value::Number(n) => n.as_u64().ok_or_else(|| n.to_string()),
            _ => Err(val[key].to_string()),
        }
    };

    match (extract_value(a), extract_value(b)) {
        (Ok(a_num), Ok(b_num)) => a_num.cmp(&b_num),
        (Err(a_str), Err(b_str)) => a_str.cmp(&b_str),
        (Err(a_str), Ok(b_num)) => a_str.cmp(&b_num.to_string()),
        (Ok(a_num), Err(b_str)) => a_num.to_string().cmp(&b_str),
    }
}

/// Apply a permutation to a graph and write the relabeled JSON to a writer.
///
/// # Arguments
///
/// * `graph` - The parsed graph to reorder.
/// * `order` - A permutation where `order[new_index]` gives the old index.
/// * `writer` - The destination for the reordered JSON output.
///
/// # Returns
///
/// Returns a map from original node id to new node id.
fn reorder_graph<W: Write>(
    mut graph: GraphJson,
    order: Vec<usize>,
    mut writer: W,
) -> io::Result<HashMap<usize, usize>> {
    let mut old_id_to_new = HashMap::with_capacity(order.len());
    let mut new_nodes = Vec::with_capacity(order.len());
    let mut new_adjacency = Vec::with_capacity(order.len());

    for (new_idx, &old_idx) in order.iter().enumerate() {
        progress!("Relabeling node: {}\r", new_idx + 1);
        old_id_to_new.insert(graph.node_ids[old_idx], new_idx);
    }
    tracing::trace!("");

    for (new_idx, &old_idx) in order.iter().enumerate() {
        let mut node = graph.nodes[old_idx].clone();
        node["id"] = json!(new_idx);
        new_nodes.push(node);
    }

    for (new_idx, &old_idx) in order.iter().enumerate() {
        progress!("Relabeling edge: {}\r", new_idx + 1);
        let mut new_edge_lst = graph.adjacency[old_idx].clone();
        for link in &mut new_edge_lst {
            let old_neighbor_id = parse_link_id(link)?;
            let new_neighbor = old_id_to_new[&old_neighbor_id];
            link["id"] = json!(new_neighbor);
        }
        new_adjacency.push(Value::Array(new_edge_lst));
    }
    tracing::trace!("");

    graph.data["nodes"] = Value::Array(new_nodes);
    graph.data["adjacency"] = Value::Array(new_adjacency);

    tracing::trace!("Writing new json to file...");
    let rendered = serde_json::to_string(&graph.data)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    writer.write_all(rendered.as_bytes())?;

    Ok(old_id_to_new)
}

/// Find connected components of a graph using breadth-first search.
///
/// # Arguments
///
/// * `graph` - The parsed graph to decompose.
///
/// # Returns
///
/// Returns a list of components, each a vector of node indices, sorted by
/// smallest original node id.
fn connected_components(graph: &GraphJson) -> Vec<Vec<usize>> {
    let n = graph.nodes.len();
    let mut seen = vec![false; n];
    let mut components = Vec::new();

    for start in 0..n {
        if seen[start] {
            continue;
        }
        let mut queue = VecDeque::from([start]);
        let mut component = Vec::new();
        seen[start] = true;

        while let Some(node) = queue.pop_front() {
            component.push(node);
            for &neighbor in &graph.adjacency_indices[node] {
                if !seen[neighbor] {
                    seen[neighbor] = true;
                    queue.push_back(neighbor);
                }
            }
        }

        components.push(component);
    }

    components.sort_by_key(|component| graph.node_ids[component[0]]);
    components
}

/// Compute a Reverse Cuthill-McKee ordering for the entire graph.
///
/// # Arguments
///
/// * `graph` - The parsed graph to order.
///
/// # Returns
///
/// Returns a permutation of node indices that reduces bandwidth.
fn reverse_cuthill_mckee_order(graph: &GraphJson) -> Vec<usize> {
    let mut order = Vec::with_capacity(graph.nodes.len());

    for component in connected_components(graph) {
        order.extend(reverse_cuthill_mckee_component(graph, &component));
    }

    order
}

/// Compute a Reverse Cuthill-McKee ordering for a single connected component.
///
/// # Arguments
///
/// * `graph` - The parsed graph.
/// * `component` - The node indices belonging to the component.
///
/// # Returns
///
/// Returns a reversed BFS ordering of the component starting from the
/// minimum-degree node.
fn reverse_cuthill_mckee_component(graph: &GraphJson, component: &[usize]) -> Vec<usize> {
    let degrees = graph
        .adjacency_indices
        .iter()
        .map(Vec::len)
        .collect::<Vec<_>>();
    let component_set = component
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    let start = component
        .iter()
        .copied()
        .min_by_key(|&node| (degrees[node], graph.node_ids[node]))
        .unwrap();

    let mut visited = vec![false; graph.nodes.len()];
    let mut queue = VecDeque::from([start]);
    visited[start] = true;
    let mut component_order = Vec::with_capacity(component.len());

    while let Some(node) = queue.pop_front() {
        component_order.push(node);
        let mut neighbors = graph.adjacency_indices[node]
            .iter()
            .copied()
            .filter(|neighbor| component_set.contains(neighbor) && !visited[*neighbor])
            .collect::<Vec<_>>();
        neighbors.sort_by_key(|&neighbor| (degrees[neighbor], graph.node_ids[neighbor]));
        for neighbor in neighbors {
            visited[neighbor] = true;
            queue.push_back(neighbor);
        }
    }

    component_order.reverse();
    component_order
}

/// Compute a multilevel cluster ordering for the entire graph.
///
/// # Arguments
///
/// * `graph` - The parsed graph to order.
///
/// # Returns
///
/// Returns a permutation of node indices produced by recursive multilevel
/// clustering.
fn multi_level_cluster_order(graph: &GraphJson) -> Vec<usize> {
    multilevel_cluster_order_generic(&graph.adjacency_indices, &graph.node_ids)
}

fn subset_mask(size: usize, nodes: &[usize]) -> Vec<bool> {
    let mut mask = vec![false; size];
    for &node in nodes {
        mask[node] = true;
    }
    mask
}

/// Find connected components of a generic adjacency list using breadth-first search.
///
/// # Arguments
///
/// * `adjacency` - The adjacency list for each node.
/// * `labels` - Node labels used to sort components by smallest label.
///
/// # Returns
///
/// Returns a list of components, each a vector of node indices, sorted by
/// minimum label.
fn connected_components_generic(adjacency: &[Vec<usize>], labels: &[usize]) -> Vec<Vec<usize>> {
    let mut seen = vec![false; adjacency.len()];
    let mut components = Vec::new();

    for start in 0..adjacency.len() {
        if seen[start] {
            continue;
        }
        let mut queue = VecDeque::from([start]);
        let mut component = Vec::new();
        seen[start] = true;

        while let Some(node) = queue.pop_front() {
            component.push(node);
            for &neighbor in &adjacency[node] {
                if !seen[neighbor] {
                    seen[neighbor] = true;
                    queue.push_back(neighbor);
                }
            }
        }

        components.push(component);
    }

    components.sort_by_key(|component| {
        component
            .iter()
            .map(|&node| labels[node])
            .min()
            .unwrap_or(usize::MAX)
    });
    components
}

/// Compute a Reverse Cuthill-McKee ordering for a component of a generic graph.
///
/// # Arguments
///
/// * `adjacency` - The adjacency list for each node.
/// * `labels` - Node labels used for tie-breaking.
/// * `component` - The node indices belonging to the component.
///
/// # Returns
///
/// Returns a reversed BFS ordering of the component starting from the
/// minimum-degree node.
fn rcm_component_generic(
    adjacency: &[Vec<usize>],
    labels: &[usize],
    component: &[usize],
) -> Vec<usize> {
    let component_mask = subset_mask(adjacency.len(), component);
    let local_degree = local_degree_in_subset(adjacency, &component_mask, component);
    let start = component
        .iter()
        .copied()
        .min_by_key(|&node| (local_degree[node], labels[node]))
        .unwrap();

    let mut visited = vec![false; adjacency.len()];
    let mut queue = VecDeque::from([start]);
    let mut order = Vec::with_capacity(component.len());
    visited[start] = true;

    while let Some(node) = queue.pop_front() {
        order.push(node);
        let mut neighbors = adjacency[node]
            .iter()
            .copied()
            .filter(|&neighbor| component_mask[neighbor] && !visited[neighbor])
            .collect::<Vec<_>>();
        neighbors.sort_by_key(|&neighbor| (local_degree[neighbor], labels[neighbor]));
        for neighbor in neighbors {
            visited[neighbor] = true;
            queue.push_back(neighbor);
        }
    }

    order.reverse();
    order
}

/// Compute a multilevel cluster ordering for a generic graph.
///
/// # Arguments
///
/// * `adjacency` - The adjacency list for each node.
/// * `labels` - Node labels used for tie-breaking and component sorting.
///
/// # Returns
///
/// Returns a permutation of node indices produced by recursive multilevel
/// clustering across all connected components.
fn multilevel_cluster_order_generic(adjacency: &[Vec<usize>], labels: &[usize]) -> Vec<usize> {
    let mut order = Vec::with_capacity(adjacency.len());
    for component in connected_components_generic(adjacency, labels) {
        order.extend(multilevel_cluster_component_generic(
            adjacency, labels, &component,
        ));
    }
    order
}

/// Compute a multilevel cluster ordering for a single component of a generic graph.
///
/// # Arguments
///
/// * `adjacency` - The adjacency list for each node.
/// * `labels` - Node labels used for tie-breaking.
/// * `component` - The node indices belonging to the component.
///
/// # Returns
///
/// Returns an ordering that recursively partitions the component into clusters,
/// orders each cluster with RCM, builds a coarse graph of clusters, and recurses.
fn multilevel_cluster_component_generic(
    adjacency: &[Vec<usize>],
    labels: &[usize],
    component: &[usize],
) -> Vec<usize> {
    if component.len() <= 8 {
        return rcm_component_generic(adjacency, labels, component);
    }

    let clusters = greedy_cluster_partition(adjacency, labels, component, 6);
    if clusters.len() <= 1 || clusters.len() == component.len() {
        return rcm_component_generic(adjacency, labels, component);
    }

    let cluster_orders = clusters
        .iter()
        .map(|cluster| rcm_component_generic(adjacency, labels, cluster))
        .collect::<Vec<_>>();
    let (coarse_adjacency, coarse_labels) = build_coarse_graph(adjacency, labels, &clusters);
    let coarse_order = multilevel_cluster_order_generic(&coarse_adjacency, &coarse_labels);

    let mut order = Vec::with_capacity(component.len());
    for cluster_idx in coarse_order {
        order.extend(cluster_orders[cluster_idx].iter().copied());
    }
    order
}

/// Partition a component into small clusters using a greedy seed-expansion strategy.
///
/// # Arguments
///
/// * `adjacency` - The adjacency list for each node.
/// * `labels` - Node labels used for tie-breaking.
/// * `component` - The node indices to partition.
/// * `max_cluster_size` - The maximum number of nodes per cluster.
///
/// # Returns
///
/// Returns a list of clusters, each a vector of node indices.
fn greedy_cluster_partition(
    adjacency: &[Vec<usize>],
    labels: &[usize],
    component: &[usize],
    max_cluster_size: usize,
) -> Vec<Vec<usize>> {
    let component_mask = subset_mask(adjacency.len(), component);
    let local_degree = local_degree_in_subset(adjacency, &component_mask, component);
    let mut assigned = vec![false; adjacency.len()];
    let mut unassigned = component.to_vec();
    unassigned.sort_by_key(|&node| (local_degree[node], labels[node]));
    let mut remaining = unassigned.len();
    let mut clusters = Vec::new();
    let mut seed_marks = vec![0usize; adjacency.len()];
    let mut mark_epoch = 1usize;

    while remaining > 0 {
        let seed = unassigned
            .iter()
            .copied()
            .find(|&node| !assigned[node])
            .unwrap();

        let mut cluster = vec![seed];
        assigned[seed] = true;
        remaining -= 1;
        for &neighbor in &adjacency[seed] {
            if component_mask[neighbor] {
                seed_marks[neighbor] = mark_epoch;
            }
        }

        let mut candidates = adjacency[seed]
            .iter()
            .copied()
            .filter(|&neighbor| component_mask[neighbor] && !assigned[neighbor])
            .collect::<Vec<_>>();
        candidates.sort_by_key(|&neighbor| {
            let shared = adjacency[neighbor]
                .iter()
                .filter(|&&next| component_mask[next] && seed_marks[next] == mark_epoch)
                .count();
            (Reverse(shared), local_degree[neighbor], labels[neighbor])
        });

        for neighbor in candidates
            .into_iter()
            .take(max_cluster_size.saturating_sub(1))
        {
            assigned[neighbor] = true;
            remaining -= 1;
            cluster.push(neighbor);
        }

        mark_epoch = mark_epoch.wrapping_add(1);
        if mark_epoch == 0 {
            seed_marks.fill(0);
            mark_epoch = 1;
        }

        clusters.push(cluster);
    }

    clusters
}

/// Compute the degree of each node restricted to a subset of the graph.
///
/// # Arguments
///
/// * `adjacency` - The adjacency list for each node.
/// * `subset_mask` - A boolean mask indicating which nodes belong to the subset.
/// * `subset` - The node indices in the subset.
///
/// # Returns
///
/// Returns a vector indexed by node where each entry is the number of neighbors
/// within the subset.
fn local_degree_in_subset(
    adjacency: &[Vec<usize>],
    subset_mask: &[bool],
    subset: &[usize],
) -> Vec<usize> {
    let mut local_degree = vec![0usize; adjacency.len()];
    for &node in subset {
        local_degree[node] = adjacency[node]
            .iter()
            .filter(|&&neighbor| subset_mask[neighbor])
            .count();
    }
    local_degree
}

/// Build a coarse graph where each cluster is contracted into a single node.
///
/// # Arguments
///
/// * `adjacency` - The adjacency list of the original graph.
/// * `labels` - Node labels of the original graph.
/// * `clusters` - The cluster partition of the original graph.
///
/// # Returns
///
/// Returns a tuple of the coarse adjacency list and coarse labels, where each
/// coarse label is the minimum original label in the cluster.
fn build_coarse_graph(
    adjacency: &[Vec<usize>],
    labels: &[usize],
    clusters: &[Vec<usize>],
) -> (Vec<Vec<usize>>, Vec<usize>) {
    let mut cluster_of = vec![usize::MAX; adjacency.len()];
    for (cluster_idx, cluster) in clusters.iter().enumerate() {
        for &node in cluster {
            cluster_of[node] = cluster_idx;
        }
    }

    let mut coarse_sets = vec![HashSet::new(); clusters.len()];
    for (cluster_idx, cluster) in clusters.iter().enumerate() {
        for &node in cluster {
            for &neighbor in &adjacency[node] {
                let neighbor_cluster = cluster_of[neighbor];
                if neighbor_cluster != cluster_idx && neighbor_cluster != usize::MAX {
                    coarse_sets[cluster_idx].insert(neighbor_cluster);
                }
            }
        }
    }

    let coarse_adjacency = coarse_sets
        .into_iter()
        .map(|neighbors| {
            let mut neighbors = neighbors.into_iter().collect::<Vec<_>>();
            neighbors.sort_unstable();
            neighbors
        })
        .collect::<Vec<_>>();
    let coarse_labels = clusters
        .iter()
        .map(|cluster| {
            cluster
                .iter()
                .map(|&node| labels[node])
                .min()
                .unwrap_or(usize::MAX)
        })
        .collect::<Vec<_>>();

    (coarse_adjacency, coarse_labels)
}

#[cfg(test)]
mod tests;
