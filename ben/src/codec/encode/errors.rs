use std::io;
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum BenEncodeError {
    #[error("Encountered a repeated sample when encoding.")]
    RepeatedSample,
}

impl From<BenEncodeError> for io::Error {
    fn from(error: BenEncodeError) -> Self {
        io::Error::new(io::ErrorKind::Other, error)
    }
}
