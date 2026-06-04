use super::helpers::{reorder_graph_to_bytes, require_reorder};
use crate::common::{networkx_graph_from_bytes, parse_graph_input};
use pyo3::exceptions::PyException;
use pyo3::prelude::*;

/// Parse JSON bytes into a Python object (used for the permutation map).
fn json_loads(py: Python<'_>, bytes: &[u8]) -> PyResult<Py<PyAny>> {
    let json_mod = py.import("json")?;
    let text = std::str::from_utf8(bytes)
        .map_err(|e| PyException::new_err(format!("reordered output is not valid UTF-8: {e}")))?;
    Ok(json_mod.call_method1("loads", (text,))?.into())
}

/// Reorder a NetworkX adjacency-format graph and return `(reordered_graph, node_permutation_map)`.
///
/// `reordered_graph` is a live NetworkX graph (matching `BendlEncoder.add_graph` /
/// `BendlDecoder.read_graph`); `node_permutation_map` is the parsed map JSON.
///
/// `sort` selects the ordering: `"mlc"` (multi-level clustering), `"rcm"` (reverse Cuthill-McKee),
/// or `"key"` to sort by a node attribute named via `key` (e.g. `key="GEOID"`, or the special
/// `key="id"` for the NetworkX node id). The permutation map matches the on-disk
/// `node_permutation_map.json` convention (a `node_permutation_old_to_new` object).
#[pyfunction]
#[pyo3(signature = (graph, sort = Some("mlc".to_string()), key = None))]
#[pyo3(text_signature = "(graph, sort='mlc', key=None)")]
pub fn graph_reorder<'py>(
    py: Python<'py>,
    graph: Bound<'py, PyAny>,
    sort: Option<String>,
    key: Option<String>,
) -> PyResult<(Py<PyAny>, Py<PyAny>)> {
    let plan = require_reorder(sort.as_deref(), key.as_deref())?;
    let graph_bytes = parse_graph_input(py, &graph)?;
    let (reordered_bytes, map_bytes) = reorder_graph_to_bytes(&graph_bytes, &plan)?;
    Ok((
        networkx_graph_from_bytes(py, &reordered_bytes)?,
        json_loads(py, &map_bytes)?,
    ))
}
