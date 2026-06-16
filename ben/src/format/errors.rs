use std::io;
use thiserror::Error;

/// Errors produced while parsing or validating a BEN file header/banner.
#[derive(Debug, Error)]
pub enum FormatError {
    #[error(
        "unrecognized BEN banner (got \"{}\" = {actual:?}; expected one of \
         \"STANDARD BEN FILE\", \"MKVCHAIN BEN FILE\", or \"TWODELTA BEN FILE\"). \
         If the decoded text looks like JSON or plain text, the input is likely \
         not a BEN file.",
        .actual.escape_ascii()
    )]
    UnknownBanner { actual: Vec<u8> },

    #[error("IO error reading banner: {0}")]
    Io(#[from] io::Error),
}

impl From<FormatError> for io::Error {
    fn from(e: FormatError) -> Self {
        match e {
            FormatError::Io(e) => e,
            other => io::Error::new(io::ErrorKind::InvalidData, other),
        }
    }
}
