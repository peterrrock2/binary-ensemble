use std::io;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EncodeError {
    #[error("line {line}: JSON parse error: {source}")]
    JsonParse {
        line: usize,
        #[source]
        source: serde_json::Error,
    },

    #[error("line {line}: `assignment` field missing or not an array")]
    MissingAssignment { line: usize },

    #[error("line {line}: value `{value}` cannot be represented as u16")]
    InvalidAssignmentValue { line: usize, value: u64 },

    #[error("TwoDelta transition involves more than two distinct district ids")]
    TwoDeltaTooManyIds,

    #[error("TwoDelta received identical assignment to previous frame")]
    TwoDeltaIdentical,

    #[error("Encoders require equal-length assignment vectors, got {prev_len} vs {new_len}")]
    LengthMismatch { prev_len: usize, new_len: usize },

    #[error("TwoDelta delta_pair hint provided without corresponding masks")]
    TwoDeltaHintWithoutMasks,

    #[error("TwoDelta pair hint has identical values for both ids (value: {value})")]
    TwoDeltaIdenticalPairHint { value: u16 },

    #[error("TwoDelta mask for id {id} is missing from the position map")]
    TwoDeltaMissingMask { id: u16 },

    #[error("TwoDelta mask for id {id} is empty")]
    TwoDeltaEmptyMask { id: u16 },

    #[error("TwoDelta mask referenced position {pos} whose value {actual} is outside the pair ({a}, {b})")]
    TwoDeltaMaskOutOfPair {
        pos: usize,
        actual: u16,
        a: u16,
        b: u16,
    },

    #[error("XZ encoder initialization failed: {0}")]
    XzInit(#[source] xz2::stream::Error),

    #[error(transparent)]
    Io(#[from] io::Error),
}

impl From<EncodeError> for io::Error {
    fn from(e: EncodeError) -> Self {
        match e {
            EncodeError::Io(e) => e,
            other => io::Error::new(io::ErrorKind::InvalidData, other),
        }
    }
}
