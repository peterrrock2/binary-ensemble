use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// A NetworkX graph in adjacency-format JSON.
///
/// This is the Rust representation of the JSON produced by
/// `networkx.adjacency_data()`. All fields use `#[serde(default)]` so that
/// inputs which omit optional keys (e.g. `"directed"`) still deserialize
/// successfully.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct NxGraphAdjFormat {
    /// Whether the graph is directed. Defaults to `false`.
    #[serde(default)]
    pub directed: bool,
    /// Whether the graph allows parallel edges. Defaults to `false`.
    #[serde(default)]
    pub multigraph: bool,
    /// Graph-level attributes as key/value pairs.
    #[serde(default)]
    pub graph: Vec<(String, Value)>,

    /// The list of nodes, each carrying an `id` and arbitrary attributes.
    #[serde(default)]
    pub nodes: Vec<NxNode>,
    /// Adjacency lists parallel to `nodes`. `adjacency[i]` lists the
    /// neighbors (and edge attributes) of `nodes[i]`.
    #[serde(default)]
    pub adjacency: Vec<Vec<NxAdjEntry>>,
}

/// A single node in a [`NxGraphAdjFormat`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct NxNode {
    /// The node identifier. May be an integer or a string.
    #[serde(rename = "id")]
    pub id: Value,

    /// All remaining node attributes (flattened from the JSON object).
    #[serde(flatten)]
    pub attrs: BTreeMap<String, Value>,
}

/// A single entry in a node's adjacency list, representing one edge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct NxAdjEntry {
    /// The id of the neighbor node this edge points to.
    #[serde(rename = "id")]
    pub id: Value,

    /// The edge key, present only in multigraphs. Omitted from JSON when
    /// `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<Value>,

    /// All remaining edge attributes (flattened from the JSON object).
    #[serde(flatten)]
    pub attrs: BTreeMap<String, Value>,
}
