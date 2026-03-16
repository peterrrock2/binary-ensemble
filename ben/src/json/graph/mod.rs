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
    /// Order nodes using the Fiedler-vector style spectral ordering.
    Spectral,
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
        GraphOrderingMethod::Spectral => spectral_order(&graph),
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

fn spectral_order(graph: &GraphJson) -> Vec<usize> {
    let mut order = Vec::with_capacity(graph.nodes.len());
    let degrees = graph
        .adjacency_indices
        .iter()
        .map(Vec::len)
        .collect::<Vec<_>>();

    for component in connected_components(graph) {
        if component.len() <= 2 {
            let mut tiny = component.clone();
            tiny.sort_by_key(|&node| graph.node_ids[node]);
            order.extend(tiny);
            continue;
        }

        let local_index = component
            .iter()
            .enumerate()
            .map(|(idx, &node)| (node, idx))
            .collect::<HashMap<_, _>>();
        let max_degree = component
            .iter()
            .map(|&node| degrees[node])
            .max()
            .unwrap_or(0) as f64;

        let mut x = component
            .iter()
            .map(|&node| pseudo_random_seed(graph.node_ids[node]))
            .collect::<Vec<_>>();
        orthogonalize_to_constant(&mut x);
        normalize(&mut x);

        if x.iter().all(|value| value.abs() < 1e-12) {
            for (idx, value) in x.iter_mut().enumerate() {
                *value = idx as f64;
            }
            orthogonalize_to_constant(&mut x);
            normalize(&mut x);
        }

        let mut y = vec![0.0; component.len()];
        for _ in 0..128 {
            for (local_idx, &node) in component.iter().enumerate() {
                let degree = degrees[node] as f64;
                let neighbor_sum = graph.adjacency_indices[node]
                    .iter()
                    .filter_map(|neighbor| local_index.get(neighbor).copied())
                    .map(|neighbor_local| x[neighbor_local])
                    .sum::<f64>();
                y[local_idx] = neighbor_sum + (max_degree - degree) * x[local_idx];
            }

            orthogonalize_to_constant(&mut y);
            normalize(&mut y);

            let diff = x
                .iter()
                .zip(&y)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0_f64, f64::max);
            x.copy_from_slice(&y);
            if diff < 1e-10 {
                break;
            }
        }

        let mut component_order = component
            .iter()
            .enumerate()
            .map(|(local_idx, &node)| (node, x[local_idx]))
            .collect::<Vec<_>>();
        component_order.sort_by(|(a_node, a_val), (b_node, b_val)| {
            a_val
                .partial_cmp(b_val)
                .unwrap_or(Ordering::Equal)
                .then_with(|| graph.node_ids[*a_node].cmp(&graph.node_ids[*b_node]))
        });
        order.extend(component_order.into_iter().map(|(node, _)| node));
    }

    order
}

fn pseudo_random_seed(node_id: usize) -> f64 {
    let raw = node_id.wrapping_mul(1_103_515_245).wrapping_add(12_345) % 1_000;
    raw as f64 / 500.0 - 1.0
}

fn orthogonalize_to_constant(values: &mut [f64]) {
    if values.is_empty() {
        return;
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    for value in values {
        *value -= mean;
    }
}

fn normalize(values: &mut [f64]) {
    let norm = values.iter().map(|value| value * value).sum::<f64>().sqrt();
    if norm > 0.0 {
        for value in values {
            *value /= norm;
        }
    }
}

#[cfg(test)]
mod tests;
