//! JSON graph helpers used by relabeling workflows.

use crate::progress;
use serde_json::{json, Value};
use std::cmp::Ordering;
use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Result, Write};
use std::result::Result as StdResult;

/// Topology-based graph ordering methods supported by `reben`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphOrderingMethod {
    /// Order nodes using a recursive nested-dissection heuristic.
    NestedDissection,
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
        GraphOrderingMethod::NestedDissection => nested_dissection_order(&graph),
        GraphOrderingMethod::ReverseCuthillMckee => reverse_cuthill_mckee_order(&graph),
    };

    reorder_graph(graph, order, writer)
}

fn parse_node_id(node: &Value) -> io::Result<usize> {
    node["id"].as_u64().map(|v| v as usize).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Node id is not an unsigned integer: {}", node["id"]),
        )
    })
}

fn parse_link_id(link: &Value) -> io::Result<usize> {
    link["id"].as_u64().map(|v| v as usize).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Edge target id is not an unsigned integer: {}", link["id"]),
        )
    })
}

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

fn reverse_cuthill_mckee_order(graph: &GraphJson) -> Vec<usize> {
    let mut order = Vec::with_capacity(graph.nodes.len());
    let degrees = graph
        .adjacency_indices
        .iter()
        .map(Vec::len)
        .collect::<Vec<_>>();

    for component in connected_components(graph) {
        let component_set = component.iter().copied().collect::<std::collections::HashSet<_>>();
        let start = component
            .iter()
            .copied()
            .min_by_key(|&node| (degrees[node], graph.node_ids[node]))
            .unwrap();

        let mut visited = HashMap::new();
        let mut queue = VecDeque::from([start]);
        visited.insert(start, true);
        let mut component_order = Vec::with_capacity(component.len());

        while let Some(node) = queue.pop_front() {
            component_order.push(node);
            let mut neighbors = graph.adjacency_indices[node]
                .iter()
                .copied()
                .filter(|neighbor| component_set.contains(neighbor) && !visited.contains_key(neighbor))
                .collect::<Vec<_>>();
            neighbors.sort_by_key(|&neighbor| (degrees[neighbor], graph.node_ids[neighbor]));
            for neighbor in neighbors {
                visited.insert(neighbor, true);
                queue.push_back(neighbor);
            }
        }

        component_order.reverse();
        order.extend(component_order);
    }

    order
}

fn nested_dissection_order(graph: &GraphJson) -> Vec<usize> {
    let mut order = Vec::with_capacity(graph.nodes.len());

    for component in connected_components(graph) {
        order.extend(nested_dissection_component(graph, component));
    }

    order
}

fn nested_dissection_component(graph: &GraphJson, component: Vec<usize>) -> Vec<usize> {
    if component.len() <= 8 {
        let mut base = component;
        base.sort_by_key(|&node| graph.node_ids[node]);
        return base;
    }

    let component_mask = subset_mask(graph.nodes.len(), &component);
    let start = component
        .iter()
        .copied()
        .min_by_key(|&node| (graph.adjacency_indices[node].len(), graph.node_ids[node]))
        .unwrap();
    let a = farthest_node_in_subset(graph, start, &component_mask);
    let (b, dist_from_a) = farthest_node_with_distances(graph, a, &component_mask);
    let dist_from_b = bfs_distances(graph, b, &component_mask);

    let Some(max_dist) = dist_from_a.iter().flatten().copied().max() else {
        let mut base = component;
        base.sort_by_key(|&node| graph.node_ids[node]);
        return base;
    };

    let separator_target = max_dist / 2;
    let mut separator = component
        .iter()
        .copied()
        .filter(|&node| dist_from_a[node] == Some(separator_target))
        .collect::<Vec<_>>();

    if separator.is_empty() {
        let best_delta = component
            .iter()
            .filter_map(|&node| Some((node, dist_from_a[node]?, dist_from_b[node]?)))
            .map(|(_, da, db)| da.abs_diff(db))
            .min()
            .unwrap_or(0);
        separator = component
            .iter()
            .copied()
            .filter(|&node| {
                matches!(
                    (dist_from_a[node], dist_from_b[node]),
                    (Some(da), Some(db)) if da.abs_diff(db) == best_delta
                )
            })
            .collect();
    }

    separator.sort_by_key(|&node| graph.node_ids[node]);
    let separator_mask = subset_mask(graph.nodes.len(), &separator);
    let remaining = component
        .iter()
        .copied()
        .filter(|node| !separator_mask[*node])
        .collect::<Vec<_>>();
    let mut subcomponents = connected_components_in_subset(graph, &remaining);
    if subcomponents.len() <= 1 {
        let mut fallback = component;
        fallback.sort_by_key(|&node| graph.node_ids[node]);
        return fallback;
    }

    subcomponents.sort_by_key(|part| {
        part.iter()
            .filter_map(|&node| dist_from_a[node])
            .min()
            .unwrap_or(usize::MAX)
    });

    let mut order = Vec::with_capacity(component.len());
    for subcomponent in subcomponents {
        order.extend(nested_dissection_component(graph, subcomponent));
    }
    order.extend(separator);
    order
}

fn subset_mask(size: usize, nodes: &[usize]) -> Vec<bool> {
    let mut mask = vec![false; size];
    for &node in nodes {
        mask[node] = true;
    }
    mask
}

fn bfs_distances(graph: &GraphJson, start: usize, allowed: &[bool]) -> Vec<Option<usize>> {
    let mut distances = vec![None; graph.nodes.len()];
    let mut queue = VecDeque::from([start]);
    distances[start] = Some(0);

    while let Some(node) = queue.pop_front() {
        let distance = distances[node].unwrap();
        for &neighbor in &graph.adjacency_indices[node] {
            if allowed[neighbor] && distances[neighbor].is_none() {
                distances[neighbor] = Some(distance + 1);
                queue.push_back(neighbor);
            }
        }
    }

    distances
}

fn farthest_node_in_subset(graph: &GraphJson, start: usize, allowed: &[bool]) -> usize {
    farthest_node_with_distances(graph, start, allowed).0
}

fn farthest_node_with_distances(
    graph: &GraphJson,
    start: usize,
    allowed: &[bool],
) -> (usize, Vec<Option<usize>>) {
    let distances = bfs_distances(graph, start, allowed);
    let farthest = distances
        .iter()
        .enumerate()
        .filter(|(idx, distance)| allowed[*idx] && distance.is_some())
        .max_by_key(|(idx, distance)| (distance.unwrap(), graph.node_ids[*idx]))
        .map(|(idx, _)| idx)
        .unwrap_or(start);
    (farthest, distances)
}

fn connected_components_in_subset(graph: &GraphJson, nodes: &[usize]) -> Vec<Vec<usize>> {
    let allowed = subset_mask(graph.nodes.len(), nodes);
    let mut seen = vec![false; graph.nodes.len()];
    let mut components = Vec::new();

    for &start in nodes {
        if seen[start] {
            continue;
        }
        let mut queue = VecDeque::from([start]);
        let mut component = Vec::new();
        seen[start] = true;

        while let Some(node) = queue.pop_front() {
            component.push(node);
            for &neighbor in &graph.adjacency_indices[node] {
                if allowed[neighbor] && !seen[neighbor] {
                    seen[neighbor] = true;
                    queue.push_back(neighbor);
                }
            }
        }

        component.sort_by_key(|&node| graph.node_ids[node]);
        components.push(component);
    }

    components
}

#[cfg(test)]
mod tests;
