//! This module provides some utility functions for working with assignments
//! and RLE encoding. It also provides a function to sort a JSON file by a key
//! so as to make the BEN encoding more efficient.

use super::{log, logln};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Result, Write};
use std::result::Result as StdResult;

/// Convert a vector of assignments to a run-length encoded (RLE) vector.
///
/// # Arguments
///
/// * `assign_vec` - A vector of assignments to convert to RLE.
///
/// # Returns
///
/// A vector of tuples where the first element is the value and the second element is
/// the length of the run.
pub fn assign_to_rle(assign_vec: Vec<u16>) -> Vec<(u16, u16)> {
    let mut prev_assign: u16 = 0;
    let mut count: u16 = 0;
    let mut first = true;
    let mut rle_vec: Vec<(u16, u16)> = Vec::new();

    for assign in assign_vec {
        if first {
            prev_assign = assign;
            count = 1;
            first = false;
            continue;
        }
        if assign == prev_assign {
            count += 1;
        } else {
            rle_vec.push((prev_assign, count));
            // Reset for next run
            prev_assign = assign;
            count = 1;
        }
    }

    // Handle the last run
    if count > 0 {
        rle_vec.push((prev_assign, count));
    }
    rle_vec
}

/// Convert a run-length encoded (RLE) vector to a vector of assignments.
///
/// # Arguments
///
/// * `rle_vec` - A vector of tuples where the first element is the value and the second element is
/// the length of the run.
///
/// # Returns
///
/// A vector of assignments.
pub fn rle_to_vec(rle_vec: Vec<(u16, u16)>) -> Vec<u16> {
    let mut output_vec: Vec<u16> = Vec::new();
    for (val, len) in rle_vec {
        for _ in 0..len {
            output_vec.push(val);
        }
    }
    output_vec
}

/// Sorts a JSON-formatted NetworkX graph file by a key.
/// This function will sort the nodes in the graph by the key provided and
/// then relabel the nodes in the graph from 0 to n-1 where n is the number
/// of nodes in the graph. It will also relabel the edges in the graph to
/// match the new node labels.
///
/// # Arguments
///
/// * `reader` - A reader for the JSON file to sort.
/// * `writer` - A writer for the new JSON file.
/// * `key` - The key to sort the nodes by.
///
/// # Returns
///
/// A Result containing a HashMap from the old node labels to the new node labels.
pub fn sort_json_file_by_key<R: Read, W: Write>(
    reader: R,
    mut writer: W,
    key: &str,
) -> Result<HashMap<usize, usize>> {
    logln!("Loading JSON file...");
    let mut data: Value = serde_json::from_reader(reader).unwrap();

    logln!("Sorting JSON file by key: {}", key);
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
            log!("Relabeling node: {}\r", i + 1);
            node_map.insert(node["id"].to_string().parse::<usize>().unwrap(), i);
            rev_node_map.insert(i, node["id"].to_string().parse::<usize>().unwrap());
            node["id"] = json!(i);
        }
    }
    logln!();

    let mut edge_array = Vec::new();
    if let Some(edges) = data["adjacency"].as_array() {
        for i in 0..edges.len() {
            log!("Relabeling edge: {}\r", i + 1);
            let edge_list_location =
                rev_node_map[&data["nodes"][i]["id"].to_string().parse::<usize>().unwrap()];
            let mut new_edge_lst = edges[edge_list_location].as_array().unwrap().clone();
            for link in new_edge_lst.iter_mut() {
                let new = node_map[&link["id"].to_string().parse::<usize>().unwrap()];
                link["id"] = json!(new);
            }
            edge_array.push(new_edge_lst);
        }
    }
    logln!();

    data["adjacency"] = json!(edge_array);

    logln!("Writing new json to file...");
    writer.write_all(serde_json::to_string(&data).unwrap().as_bytes())?;

    Ok(node_map)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_assign_to_rle() {
        let assign_vec: Vec<u16> = vec![1, 1, 1, 2, 2, 3];

        let result: Vec<(u16, u16)> = vec![(1, 3), (2, 2), (3, 1)];

        assert_eq!(assign_to_rle(assign_vec), result);
    }

    #[test]
    fn test_rle_to_vec() {
        let rle_vec: Vec<(u16, u16)> = vec![(1, 3), (2, 2), (3, 1)];

        let result: Vec<u16> = vec![1, 1, 1, 2, 2, 3];

        assert_eq!(rle_to_vec(rle_vec), result);
    }

    #[test]
    fn test_relabel_small_file() {
        //
        // 6 -- 7 -- 8
        // |    |    |
        // 3 -- 4 -- 5
        // |    |    |
        // 0 -- 1 -- 2
        //
        let input = r#"{
    "adjacency": [
        [ { "id": 3 }, { "id": 1 } ],
        [ { "id": 0 }, { "id": 4 }, { "id": 2 } ],
        [ { "id": 1 }, { "id": 5 } ],
        [ { "id": 0 }, { "id": 6 }, { "id": 4 } ],
        [ { "id": 1 }, { "id": 3 }, { "id": 7 }, { "id": 5 } ],
        [ { "id": 2 }, { "id": 4 }, { "id": 8 } ],
        [ { "id": 3 }, { "id": 7 } ],
        [ { "id": 4 }, { "id": 6 }, { "id": 8 } ],
        [ { "id": 5 }, { "id": 7 } ]
    ],
    "directed": false,
    "graph": [],
    "multigraph": false,
    "nodes": [
        {
            "TOTPOP": 1,
            "boundary_nodes": true,
            "boundary_perim": 1,
            "GEOID20": "20258288005",
            "id": 0
        },
        {
            "TOTPOP": 1,
            "boundary_nodes": true,
            "boundary_perim": 1,
            "GEOID20": "20258288004",
            "id": 1
        },
        {
            "TOTPOP": 1,
            "boundary_nodes": true,
            "boundary_perim": 1,
            "GEOID20": "20258288003",
            "id": 2
        },
        {
            "TOTPOP": 1,
            "boundary_nodes": true,
            "boundary_perim": 1,
            "GEOID20": "20258288006",
            "id": 3
        },
        {
            "TOTPOP": 1,
            "boundary_nodes": false,
            "boundary_perim": 0,
            "GEOID20": "20258288001",
            "id": 4
        },
        {
            "TOTPOP": 1,
            "boundary_nodes": true,
            "boundary_perim": 1,
            "GEOID20": "20258288002",
            "id": 5
        },
        {
            "TOTPOP": 1,
            "boundary_nodes": true,
            "boundary_perim": 1,
            "GEOID20": "20258288007",
            "id": 6
        },
        {
            "TOTPOP": 1,
            "boundary_nodes": true,
            "boundary_perim": 1,
            "GEOID20": "20258288008",
            "id": 7
        },
        {
            "TOTPOP": 1,
            "boundary_nodes": true,
            "boundary_perim": 1,
            "GEOID20": "20258288009",
            "id": 8
        }
    ]
}
"#;

        let reader = input.as_bytes();

        let mut output = Vec::new();
        let writer = &mut output;

        let key = "GEOID20";

        let _ = sort_json_file_by_key(reader, writer, key).unwrap();

        //
        // 6 -- 7 -- 8
        // |    |    |
        // 5 -- 0 -- 1
        // |    |    |
        // 4 -- 3 -- 2
        //
        let expected_output = r#"{
    "adjacency": [
        [ { "id": 3 }, { "id": 5 }, { "id": 7 }, { "id": 1 } ],
        [ { "id": 2 }, { "id": 0 }, { "id": 8 } ], [ { "id": 3 }, { "id": 1 } ],
        [ { "id": 4 }, { "id": 0 }, { "id": 2 } ],
        [ { "id": 5 }, { "id": 3 } ],
        [ { "id": 4 }, { "id": 6 }, { "id": 0 } ],
        [ { "id": 5 }, { "id": 7 } ],
        [ { "id": 0 }, { "id": 6 }, { "id": 8 } ],
        [ { "id": 1 }, { "id": 7 } ]
    ],
    "directed": false,
    "graph": [],
    "multigraph": false,
    "nodes": [
        {
            "GEOID20": "20258288001",
            "TOTPOP": 1,
            "boundary_nodes": false,
            "boundary_perim": 0,
            "id": 0
        },
        {
            "GEOID20": "20258288002",
            "TOTPOP": 1,
            "boundary_nodes": true,
            "boundary_perim": 1,
            "id": 1
        },
        {
            "GEOID20": "20258288003",
            "TOTPOP": 1,
            "boundary_nodes": true,
            "boundary_perim": 1,
            "id": 2
        },
        {
            "GEOID20": "20258288004",
            "TOTPOP": 1,
            "boundary_nodes": true,
            "boundary_perim": 1,
            "id": 3
        },
        {
            "GEOID20": "20258288005",
            "TOTPOP": 1,
            "boundary_nodes": true,
            "boundary_perim": 1,
            "id": 4
        },
        {
            "GEOID20": "20258288006",
            "TOTPOP": 1,
            "boundary_nodes": true,
            "boundary_perim": 1,
            "id": 5
        },
        {
            "GEOID20": "20258288007",
            "TOTPOP": 1,
            "boundary_nodes": true,
            "boundary_perim": 1,
            "id": 6
        },
        {
            "GEOID20": "20258288008",
            "TOTPOP": 1,
            "boundary_nodes": true,
            "boundary_perim": 1,
            "id": 7
        },
        {
            "GEOID20": "20258288009",
            "TOTPOP": 1,
            "boundary_nodes": true,
            "boundary_perim": 1,
            "id": 8
        }
    ]
}
"#;

        logln!();
        let output_json: Value = serde_json::from_slice(&output).unwrap();
        let expected_output_json: Value = serde_json::from_str(expected_output).unwrap();

        assert_eq!(output_json, expected_output_json);
    }
}
