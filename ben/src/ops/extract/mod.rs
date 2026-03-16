//! Sample extraction helpers for BEN and XBEN streams.

use crate::codec::decode::decode_ben32_line;
use crate::io::reader::{BenDecoder, XBenDecoder};
use serde_json::Error as SerdeError;
use std::fmt;
use std::io::Cursor;
use std::io::{self, Read};

#[derive(Debug)]
/// Error categories returned when extracting an individual sample from a file.
pub enum SampleErrorKind {
    InvalidSampleNumber,
    SampleNotFound { sample_number: usize },
    IoError(io::Error),
    JsonError(SerdeError),
}

#[derive(Debug)]
/// Error returned by sample extraction helpers.
pub struct SampleError {
    /// The underlying extraction failure category.
    pub kind: SampleErrorKind,
}

impl SampleError {
    /// Wrap a plain I/O error as a [`SampleError`].
    ///
    /// # Arguments
    ///
    /// * `error` - The underlying I/O error.
    ///
    /// # Returns
    ///
    /// Returns a new [`SampleError`] with [`SampleErrorKind::IoError`].
    pub fn new_io_error(error: io::Error) -> Self {
        SampleError {
            kind: SampleErrorKind::IoError(error),
        }
    }
}

impl fmt::Display for SampleError {
    /// Format the sample extraction error for display.
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match &self.kind {
            SampleErrorKind::InvalidSampleNumber => {
                write!(
                    f,
                    "Invalid sample number. Sample number must be greater than 0"
                )
            }
            SampleErrorKind::SampleNotFound { sample_number } => {
                write!(
                    f,
                    "Sample number not found in file. Failed to find sample '{}'. Last sample seems to be '{}'",
                    sample_number,
                    sample_number - 1
                )
            }
            SampleErrorKind::IoError(e) => write!(f, "IO Error: {}", e),
            SampleErrorKind::JsonError(e) => write!(f, "JSON Error: {}", e),
        }
    }
}

impl std::error::Error for SampleError {
    /// Return the underlying source error when one exists.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.kind {
            SampleErrorKind::InvalidSampleNumber => None,
            SampleErrorKind::SampleNotFound { .. } => None,
            SampleErrorKind::IoError(e) => Some(e),
            SampleErrorKind::JsonError(e) => Some(e),
        }
    }
}

impl From<io::Error> for SampleError {
    /// Wrap a plain I/O error as a sample extraction error.
    fn from(error: io::Error) -> Self {
        SampleError::new_io_error(error)
    }
}

impl From<SerdeError> for SampleError {
    /// Wrap a JSON parsing error as a sample extraction error.
    fn from(error: SerdeError) -> Self {
        SampleError {
            kind: SampleErrorKind::JsonError(error),
        }
    }
}

/// Extract a single 1-based sample from an uncompressed BEN stream.
///
/// # Arguments
///
/// * `reader` - The input BEN stream, including its 17-byte banner.
/// * `sample_number` - The 1-based sample index to retrieve.
///
/// # Returns
///
/// Returns the decoded assignment vector for the requested sample.
pub fn extract_assignment_ben<R: Read>(
    mut reader: R,
    sample_number: usize,
) -> Result<Vec<u16>, SampleError> {
    if sample_number == 0 {
        return Err(SampleError {
            kind: SampleErrorKind::InvalidSampleNumber,
        });
    }

    let mut current_sample = 1;
    let inner_decoder = BenDecoder::new(&mut reader).expect("Failed to create XBenDecoder");
    for record in inner_decoder {
        let (assignment, count) = record.map_err(SampleError::new_io_error)?;
        if current_sample == sample_number || current_sample + count as usize > sample_number {
            return Ok(assignment);
        }
        current_sample += count as usize;
    }

    Err(SampleError {
        kind: SampleErrorKind::SampleNotFound {
            sample_number: current_sample,
        },
    })
}

/// Extract a single 1-based sample from an XBEN stream.
///
/// # Arguments
///
/// * `reader` - The compressed XBEN input stream.
/// * `sample_number` - The 1-based sample index to retrieve.
///
/// # Returns
///
/// Returns the decoded assignment vector for the requested sample.
pub fn extract_assignment_xben<R: Read>(
    mut reader: R,
    sample_number: usize,
) -> Result<Vec<u16>, SampleError> {
    if sample_number == 0 {
        return Err(SampleError {
            kind: SampleErrorKind::InvalidSampleNumber,
        });
    }

    let inner_decoder = XBenDecoder::new(&mut reader).expect("Failed to create XBenDecoder");
    let variant = inner_decoder.variant;
    let frame_iterator = inner_decoder.into_frames();

    let mut current_sample = 1;
    for frame in frame_iterator {
        let frame = frame.map_err(SampleError::new_io_error)?;
        if current_sample == sample_number || current_sample + frame.1 as usize > sample_number {
            match decode_ben32_line(Cursor::new(&frame.0), variant) {
                Ok((assignment, _)) => return Ok(assignment),
                Err(e) => return Err(SampleError::new_io_error(e)),
            };
        }
        current_sample += frame.1 as usize;
    }

    Err(SampleError {
        kind: SampleErrorKind::SampleNotFound {
            sample_number: current_sample,
        },
    })
}

#[cfg(test)]
mod tests;
