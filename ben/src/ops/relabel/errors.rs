use std::io;
use thiserror::Error;

/// Errors produced by BEN relabeling operations.
#[derive(Debug, Error)]
pub enum RelabelError {
    #[error(
        "node permutation map must cover a contiguous range of new indices \
         (max index: {max_key}, but {missing} entries are missing)"
    )]
    NonContiguousMap { max_key: usize, missing: usize },

    #[error(
        "node permutation map length {map_len} does not match assignment length {assignment_len}"
    )]
    LengthMismatch {
        map_len: usize,
        assignment_len: usize,
    },

    #[error(
        "node permutation map references old index {old_idx}, \
         but assignment length is {assignment_len}"
    )]
    OldIndexOutOfRange {
        old_idx: usize,
        assignment_len: usize,
    },

    #[error(
        "node permutation map references old index {old_idx} more than once; \
         a permutation must use each old index exactly once"
    )]
    DuplicateOldIndex { old_idx: usize },

    #[error("IO error: {0}")]
    Io(#[from] io::Error),
}

impl From<RelabelError> for io::Error {
    fn from(e: RelabelError) -> Self {
        match e {
            RelabelError::Io(e) => e,
            other => io::Error::new(io::ErrorKind::InvalidInput, other),
        }
    }
}
