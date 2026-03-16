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
    /// Order nodes using a minimum-linear-arrangement heuristic.
    MinimumLinearArrangement,
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
        GraphOrderingMethod::MinimumLinearArrangement => minimum_linear_arrangement_order(&graph),
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

    for component in connected_components(graph) {
        order.extend(reverse_cuthill_mckee_component(graph, &component));
    }

    order
}

fn reverse_cuthill_mckee_component(graph: &GraphJson, component: &[usize]) -> Vec<usize> {
    let degrees = graph
        .adjacency_indices
        .iter()
        .map(Vec::len)
        .collect::<Vec<_>>();
    let component_set = component.iter().copied().collect::<std::collections::HashSet<_>>();
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

fn minimum_linear_arrangement_order(graph: &GraphJson) -> Vec<usize> {
    let mut order = Vec::with_capacity(graph.nodes.len());

    for component in connected_components(graph) {
        order.extend(minimum_linear_arrangement_component(graph, &component));
    }

    order
}

fn minimum_linear_arrangement_component(graph: &GraphJson, component: &[usize]) -> Vec<usize> {
    if component.len() <= 2 {
        return component.to_vec();
    }

    let component_mask = subset_mask(graph.nodes.len(), component);
    let mut order = reverse_cuthill_mckee_component(graph, component);

    for _ in 0..8 {
        let positions = positions_for_order(graph.nodes.len(), &order);
        order.sort_by(|&a, &b| {
            let a_score = barycenter_score(graph, a, &positions, &component_mask);
            let b_score = barycenter_score(graph, b, &positions, &component_mask);
            a_score
                .partial_cmp(&b_score)
                .unwrap_or(Ordering::Equal)
                .then_with(|| graph.node_ids[a].cmp(&graph.node_ids[b]))
        });
        local_adjacent_improvement(graph, &mut order, &component_mask);
    }

    order
}

fn subset_mask(size: usize, nodes: &[usize]) -> Vec<bool> {
    let mut mask = vec![false; size];
    for &node in nodes {
        mask[node] = true;
    }
    mask
}

fn positions_for_order(size: usize, order: &[usize]) -> Vec<usize> {
    let mut positions = vec![usize::MAX; size];
    for (idx, &node) in order.iter().enumerate() {
        positions[node] = idx;
    }
    positions
}

fn barycenter_score(
    graph: &GraphJson,
    node: usize,
    positions: &[usize],
    component_mask: &[bool],
) -> f64 {
    let mut sum = 0.0;
    let mut count = 0.0;
    for &neighbor in &graph.adjacency_indices[node] {
        if component_mask[neighbor] {
            sum += positions[neighbor] as f64;
            count += 1.0;
        }
    }

    if count == 0.0 {
        positions[node] as f64
    } else {
        sum / count
    }
}

fn local_adjacent_improvement(graph: &GraphJson, order: &mut [usize], component_mask: &[bool]) {
    if order.len() < 2 {
        return;
    }

    let mut improved = true;
    while improved {
        improved = false;
        let mut positions = positions_for_order(graph.nodes.len(), order);
        for idx in 0..order.len() - 1 {
            let current_cost = node_span_cost(graph, order[idx], &positions, component_mask)
                + node_span_cost(graph, order[idx + 1], &positions, component_mask);

            order.swap(idx, idx + 1);
            positions[order[idx]] = idx;
            positions[order[idx + 1]] = idx + 1;

            let swapped_cost = node_span_cost(graph, order[idx], &positions, component_mask)
                + node_span_cost(graph, order[idx + 1], &positions, component_mask);

            if swapped_cost <= current_cost {
                improved = swapped_cost < current_cost;
            } else {
                order.swap(idx, idx + 1);
                positions[order[idx]] = idx;
                positions[order[idx + 1]] = idx + 1;
            }
        }
    }
}

fn node_span_cost(
    graph: &GraphJson,
    node: usize,
    positions: &[usize],
    component_mask: &[bool],
) -> usize {
    graph.adjacency_indices[node]
        .iter()
        .copied()
        .filter(|&neighbor| component_mask[neighbor])
        .map(|neighbor| positions[node].abs_diff(positions[neighbor]))
        .sum()
}

#[cfg(test)]
mod tests;
