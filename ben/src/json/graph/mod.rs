//! JSON graph helpers used by relabeling workflows.

use crate::progress;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Result, Write};
use std::result::Result as StdResult;

/// Sorts a JSON-formatted NetworkX graph file by a key.
pub fn sort_json_file_by_key<R: Read, W: Write>(
    reader: R,
    mut writer: W,
    key: &str,
) -> Result<HashMap<usize, usize>> {
    tracing::trace!("Loading JSON file...");
    let mut data: Value = serde_json::from_reader(reader).unwrap();

    tracing::trace!("Sorting JSON file by key: {}", key);
    if let Some(nodes) = data["nodes"].as_array_mut() {
        nodes.sort_by(|a, b| {
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
        });
    }

    let mut node_map = HashMap::new();
    let mut rev_node_map = HashMap::new();
    if let Some(nodes) = data["nodes"].as_array_mut() {
        for (i, node) in nodes.iter_mut().enumerate() {
            progress!("Relabeling node: {}\r", i + 1);
            node_map.insert(node["id"].to_string().parse::<usize>().unwrap(), i);
            rev_node_map.insert(i, node["id"].to_string().parse::<usize>().unwrap());
            node["id"] = json!(i);
        }
    }
    tracing::trace!("");

    let mut edge_array = Vec::new();
    if let Some(edges) = data["adjacency"].as_array() {
        for i in 0..edges.len() {
            progress!("Relabeling edge: {}\r", i + 1);
            let edge_list_location =
                rev_node_map[&data["nodes"][i]["id"].to_string().parse::<usize>().unwrap()];
            let mut new_edge_lst = edges[edge_list_location].as_array().unwrap().clone();
            for link in &mut new_edge_lst {
                let new = node_map[&link["id"].to_string().parse::<usize>().unwrap()];
                link["id"] = json!(new);
            }
            edge_array.push(new_edge_lst);
        }
    }
    tracing::trace!("");

    data["adjacency"] = json!(edge_array);

    tracing::trace!("Writing new json to file...");
    writer.write_all(serde_json::to_string(&data).unwrap().as_bytes())?;

    Ok(node_map)
}

#[cfg(test)]
mod tests;
