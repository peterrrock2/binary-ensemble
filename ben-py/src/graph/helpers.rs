use binary_ensemble::json::graph::{
    sort_json_file_by_key, sort_json_file_by_ordering, GraphOrderingMethod,
};
use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use serde_json::json;
use std::io::Cursor;

/// How a `preprocess_method` / graph-utility method string maps onto reben's reordering machinery.
enum Reorder {
    /// A topology-based ordering algorithm, paired with its canonical kebab-case name.
    Ordering(GraphOrderingMethod, &'static str),
    /// A node-attribute key sort (e.g. `"geoid"`, or the special `"id"` for the NetworkX node id).
    Key(String),
}

fn classify(method: &str) -> Reorder {
    match method {
        "mlc" | "multi-level-cluster" => Reorder::Ordering(
            GraphOrderingMethod::MultiLevelCluster,
            "multi-level-cluster",
        ),
        "rcm" | "reverse-cuthill-mckee" => Reorder::Ordering(
            GraphOrderingMethod::ReverseCuthillMckee,
            "reverse-cuthill-mckee",
        ),
        other => Reorder::Key(other.to_string()),
    }
}

/// Reorder a NetworkX adjacency-format graph and emit a `node_permutation_map.json` payload.
///
/// Returns `(reordered_graph_bytes, node_permutation_map_bytes)`. The permutation map is a JSON
/// object carrying the required `node_permutation_old_to_new` field (original zero-based node
/// positions → new positions) plus an optional `key` or `ordering_method` recording how the order
/// was produced. The reben file-path fields (`input_file` / `output_file`) are omitted, since the
/// Python graph utilities have no such paths.
pub fn reorder_graph_to_bytes(graph_bytes: &[u8], method: &str) -> PyResult<(Vec<u8>, Vec<u8>)> {
    let mut reordered = Vec::new();
    let (map, key_field, ordering_field) = match classify(method) {
        Reorder::Ordering(ordering, name) => {
            let map =
                sort_json_file_by_ordering(Cursor::new(graph_bytes), &mut reordered, ordering)
                    .map_err(|e| PyException::new_err(format!("Failed to reorder graph: {e}")))?;
            (map, None::<String>, Some(name))
        }
        Reorder::Key(key) => {
            let map = sort_json_file_by_key(Cursor::new(graph_bytes), &mut reordered, &key)
                .map_err(|e| PyException::new_err(format!("Failed to reorder graph: {e}")))?;
            (map, Some(key), None)
        }
    };

    let map_json = json!({
        "key": key_field,
        "ordering_method": ordering_field,
        "node_permutation_old_to_new": map,
    });
    let map_bytes = serde_json::to_vec(&map_json)
        .map_err(|e| PyException::new_err(format!("Failed to serialize permutation map: {e}")))?;

    Ok((reordered, map_bytes))
}
