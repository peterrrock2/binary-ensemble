use binary_ensemble::json::graph::{
    sort_json_file_by_key, sort_json_file_by_ordering, GraphOrderingMethod,
};
use pyo3::exceptions::{PyException, PyValueError};
use pyo3::prelude::*;
use serde_json::json;
use std::io::Cursor;

/// A resolved reordering, derived from the `sort` / `key` arguments.
pub enum ReorderPlan {
    /// A topology-based ordering algorithm, paired with its canonical kebab-case name.
    Ordering(GraphOrderingMethod, &'static str),
    /// A node-attribute key sort (e.g. `"GEOID"`, or the special `"id"` for the NetworkX node id).
    Key(String),
}

/// Resolve the `(sort, key)` argument pair into a reordering plan, or `None` to store the graph
/// as-is (`sort=None`).
///
/// `sort` selects the method: `"mlc"` / `"rcm"` reorder by graph topology, `"key"` sorts by a node
/// attribute (which must be named via `key`), and `None` means no reordering. `key` is only valid
/// with `sort="key"`.
pub fn resolve_reorder(sort: Option<&str>, key: Option<&str>) -> PyResult<Option<ReorderPlan>> {
    match sort {
        None => {
            if key.is_some() {
                return Err(PyValueError::new_err(
                    "key=... requires sort='key'; pass sort='key' to sort by a node attribute",
                ));
            }
            Ok(None)
        }
        Some("mlc") | Some("multi-level-cluster") => {
            reject_key(key, "mlc")?;
            Ok(Some(ReorderPlan::Ordering(
                GraphOrderingMethod::MultiLevelCluster,
                "multi-level-cluster",
            )))
        }
        Some("rcm") | Some("reverse-cuthill-mckee") => {
            reject_key(key, "rcm")?;
            Ok(Some(ReorderPlan::Ordering(
                GraphOrderingMethod::ReverseCuthillMckee,
                "reverse-cuthill-mckee",
            )))
        }
        Some("key") => {
            let key = key.ok_or_else(|| {
                PyValueError::new_err(
                    "sort='key' requires key=... (the node attribute to sort by, e.g. 'GEOID')",
                )
            })?;
            Ok(Some(ReorderPlan::Key(key.to_string())))
        }
        Some(other) => Err(PyValueError::new_err(format!(
            "unknown sort {other:?}; use 'mlc', 'rcm', 'key', or None"
        ))),
    }
}

fn reject_key(key: Option<&str>, sort: &str) -> PyResult<()> {
    if key.is_some() {
        return Err(PyValueError::new_err(format!(
            "key=... is only valid with sort='key', not sort='{sort}'"
        )));
    }
    Ok(())
}

/// Resolve `(sort, key)` and require an actual reordering (used by callers that have no "store raw"
/// path, e.g. the standalone reorder utility and `relabel_bundle`).
pub fn require_reorder(sort: Option<&str>, key: Option<&str>) -> PyResult<ReorderPlan> {
    resolve_reorder(sort, key)?.ok_or_else(|| {
        PyValueError::new_err("sort=None has nothing to reorder; pass sort='mlc', 'rcm', or 'key'")
    })
}

/// Reorder a NetworkX adjacency-format graph per `plan` and emit a `node_permutation_map.json`
/// payload.
///
/// Returns `(reordered_graph_bytes, node_permutation_map_bytes)`. The permutation map is a JSON
/// object carrying the required `node_permutation_old_to_new` field (original zero-based node
/// positions → new positions) plus an optional `key` or `ordering_method` recording how the order
/// was produced. The reben file-path fields (`input_file` / `output_file`) are omitted, since the
/// Python graph utilities have no such paths.
pub fn reorder_graph_to_bytes(
    graph_bytes: &[u8],
    plan: &ReorderPlan,
) -> PyResult<(Vec<u8>, Vec<u8>)> {
    let mut reordered = Vec::new();
    let (map, key_field, ordering_field) = match plan {
        ReorderPlan::Ordering(ordering, name) => {
            let map =
                sort_json_file_by_ordering(Cursor::new(graph_bytes), &mut reordered, *ordering)
                    .map_err(|e| PyException::new_err(format!("Failed to reorder graph: {e}")))?;
            (map, None::<String>, Some(*name))
        }
        ReorderPlan::Key(key) => {
            let map = sort_json_file_by_key(Cursor::new(graph_bytes), &mut reordered, key)
                .map_err(|e| PyException::new_err(format!("Failed to reorder graph: {e}")))?;
            (map, Some(key.clone()), None)
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
