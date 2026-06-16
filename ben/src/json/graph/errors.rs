use serde_json::Value;
use std::fmt;

/// Errors that can occur when converting between [`NxGraphAdjFormat`] and [`PetxGraph`].
#[derive(Debug)]
pub(crate) enum NxPetgraphError {
    /// The `directed` flag on the input does not match the target graph type.
    DirectednessMismatch {
        /// The directedness expected by the target type.
        expected_directed: bool,
        /// The directedness found in the input data.
        found_directed: bool,
    },
    /// The `nodes` and `adjacency` arrays have different lengths.
    NodeAdjacencyLengthMismatch {
        /// Number of entries in the `nodes` array.
        n_nodes: usize,
        /// Number of entries in the `adjacency` array.
        n_adjacency_items: usize,
    },
    /// A node id appears more than once in the `nodes` array.
    DuplicateNodeId(Value),
    /// An adjacency entry references a node id not present in `nodes`.
    MissingNeighborNode(Value),
    /// A catch-all for other conversion errors.
    Other(String),
}

impl fmt::Display for NxPetgraphError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DirectednessMismatch {
                expected_directed,
                found_directed,
            } => write!(
                formatter,
                "directedness mismatch: expected_directed={}, found_directed={}",
                expected_directed, found_directed
            ),
            Self::NodeAdjacencyLengthMismatch {
                n_nodes: nodes,
                n_adjacency_items: adjacency,
            } => write!(
                formatter,
                "nodes/adjacency length mismatch: {} nodes but {} adjacency lists",
                nodes, adjacency
            ),
            Self::DuplicateNodeId(id) => {
                write!(formatter, "duplicate node id in NetworkX data: {}", id)
            }
            Self::MissingNeighborNode(id) => {
                write!(formatter, "adjacency references unknown node id: {}", id)
            }
            Self::Other(msg) => write!(formatter, "{}", msg),
        }
    }
}

impl std::error::Error for NxPetgraphError {}
