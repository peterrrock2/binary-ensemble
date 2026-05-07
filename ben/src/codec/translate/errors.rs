use std::io;
use thiserror::Error;

/// Errors produced while translating between BEN and ben32 wire formats.
#[derive(Debug, Error)]
pub enum TranslateError {
    #[error("ben32 frame payload length {len} is not a multiple of 4")]
    Ben32BadLength { len: usize },

    #[error(
        "ben32 frame missing 4-byte zero end-of-line sentinel at offset {offset} \
         (got {actual:?})"
    )]
    Ben32MissingTerminator { actual: [u8; 4], offset: usize },

    #[error(
        "TwoDelta BEN streams cannot be translated to ben32; \
         use XZAssignmentWriter/BenStreamReader for TwoDelta compressed I/O"
    )]
    TwoDeltaUnsupported,

    #[error("IO error: {0}")]
    Io(#[from] io::Error),
}

impl From<TranslateError> for io::Error {
    fn from(e: TranslateError) -> Self {
        match e {
            TranslateError::Io(e) => e,
            TranslateError::TwoDeltaUnsupported => {
                io::Error::new(io::ErrorKind::Unsupported, e.to_string())
            }
            other => io::Error::new(io::ErrorKind::InvalidData, other),
        }
    }
}
