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
/// Args:
///     graph (GraphInput): The graph: a live ``networkx.Graph`` (subclasses such as
///         ``gerrychain.Graph`` count), or NetworkX adjacency JSON as a ``dict``/``list``, raw
///         JSON ``bytes``, a file-like object with ``.read()``, or a ``str``/``os.PathLike``
///         path to a JSON file (a plain ``str`` is a *path* here).
///     sort (SortMethod, optional): The ordering: ``"mlc"`` (multi-level clustering),
///         ``"rcm"`` (reverse Cuthill-McKee), or ``"key"`` (sort by the node attribute named
///         in ``key``). Default is ``"mlc"``.
///     key (str | None, optional): Node attribute to sort by (e.g. ``key="GEOID"``, or the
///         special ``key="id"`` for the NetworkX node id); required with (and only valid
///         with) ``sort="key"``. Default is ``None``.
///
/// Returns:
///     tuple[networkx.Graph, NodePermutationMap]: The reordered graph (a live NetworkX graph,
///     matching ``BendlEncoder.add_graph`` / ``BendlDecoder.read_graph``) and the parsed
///     permutation map, whose required ``node_permutation_old_to_new`` field maps original
///     zero-based node positions to their new positions (the on-disk
///     ``node_permutation_map.json`` convention).
///
/// Raises:
///     ValueError: If ``sort`` / ``key`` is invalid.
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
