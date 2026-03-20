use crate::BenVariant;
use std::io;
use thiserror::Error;

/// Errors produced while decoding BEN or XBEN streams.
#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("TwoDelta run-length vector exhausted after {run_idx} runs \
             before position {pos} was covered")]
    TwoDeltaRunsExhausted { run_idx: usize, pos: usize },

    #[error("unknown XBEN frame tag byte {tag:#04x}")]
    XBenUnknownFrameTag { tag: u8 },

    #[error("truncated XBEN stream: partial frame at end of input")]
    XBenTruncated,

    #[error("TwoDelta frame encountered before an initial full-assignment frame")]
    TwoDeltaNoAnchorFrame,

    #[error(
        "unexpected TwoDelta frame in a non-TwoDelta BEN stream (variant: {variant:?})"
    )]
    UnexpectedTwoDeltaFrame { variant: BenVariant },

    #[error("IO error: {0}")]
    Io(#[from] io::Error),
}

impl From<DecodeError> for io::Error {
    fn from(e: DecodeError) -> Self {
        match e {
            DecodeError::Io(e) => e,
            other => io::Error::new(io::ErrorKind::InvalidData, other),
        }
    }
}
