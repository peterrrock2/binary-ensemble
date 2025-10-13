//! Module documentation.
//!
//! This module provides functionality for extracting single assignment
//! vectors from a BEN file.
use serde_json::Error as SerdeError;
use std::fmt::{self};
use std::io::Cursor;
use std::io::{self, Read};

use super::{decode_ben32_line, decode_ben_line, rle_to_vec, BenDecoder, XBenDecoder};

/// Types of errors that can occur during the extraction of assignments.
#[derive(Debug)]
pub enum SampleErrorKind {
    /// Indicates the sample number is invalid. All sample numbers must be greater than 0.
    InvalidSampleNumber,
    /// Indicates the sample number was not found in the file. The last sample number is provided.
    SampleNotFound { sample_number: usize },
    /// Wrapper for IO errors.
    IoError(io::Error),
    /// Wrapper for JSON errors.
    JsonError(SerdeError),
}

/// Error type for the extraction of assignments.
#[derive(Debug)]
pub struct SampleError {
    pub kind: SampleErrorKind,
}

impl SampleError {
    /// Create a new error from an IO error.
    ///
    /// # Arguments
    ///
    /// * `error` - The IO error to wrap.
    pub fn new_io_error(error: io::Error) -> Self {
        SampleError {
            kind: SampleErrorKind::IoError(error),
        }
    }
}

impl fmt::Display for SampleError {
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
                    "Sample number not found in file. \
                    Failed to find sample '{}'. \
                    Last sample seems to be '{}'",
                    sample_number,
                    sample_number - 1
                )
            }
            SampleErrorKind::IoError(e) => {
                write!(f, "IO Error: {}", e)
            }
            SampleErrorKind::JsonError(e) => {
                write!(f, "JSON Error: {}", e)
            }
        }
    }
}

impl std::error::Error for SampleError {
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
    fn from(error: io::Error) -> Self {
        SampleError::new_io_error(error)
    }
}

impl From<SerdeError> for SampleError {
    fn from(error: SerdeError) -> Self {
        SampleError {
            kind: SampleErrorKind::JsonError(error),
        }
    }
}

/// Extracts a single assignment from a binary-encoded data stream.
///
/// # Arguments
///
/// * `reader` - The reader to extract the assignment from.
/// * `sample_number` - The sample number to extract.
///
/// # Returns
///
/// This function returns a `Result` containing a `Vec<u16>` of the assignment if successful,
/// or a `SampleError` if an error occurred.
///
/// # Example
///
/// ```no_run
/// use ben::decode::read::extract_assignment_ben;
/// use std::{fs::File, io::BufReader};
///
/// let file = File::open("data.jsonl.ben").unwrap();
/// let reader = BufReader::new(file);
/// let sample_number = 2;
///
/// let result = extract_assignment_ben(reader, sample_number);
/// match result {
///     Ok(assignment) => {
///         eprintln!("Extracted assignment: {:?}", assignment);
///     }
///     Err(e) => {
///         eprintln!("Error: {}", e);
///     }
/// }
/// ```
///
/// # Errors
///
/// This function can return a `SampleError` if an error occurs during the extraction process.
/// The error can be one of the following:
/// * `InvalidSampleNumber` - The sample number is invalid. All sample numbers must be greater than 0.
/// * `SampleNotFound` - The sample number was not found in the file. The last sample number is provided.
/// * `IoError` - An IO error occurred during the extraction process.
/// * `JsonError` - A JSON error occurred during the extraction process.
pub fn extract_assignment_ben<R: Read>(
    mut reader: R,
    sample_number: usize,
) -> Result<Vec<u16>, SampleError> {
    if sample_number == 0 {
        return Err(SampleError {
            kind: SampleErrorKind::InvalidSampleNumber,
        });
    }

    let inner_decoder = BenDecoder::new(&mut reader).expect("Failed to create XBenDecoder");
    let frame_iterator = inner_decoder.into_frames();

    let mut current_sample = 1;
    for frame in frame_iterator {
        let frame = frame.map_err(SampleError::new_io_error)?;
        if current_sample == sample_number || current_sample + frame.count as usize > sample_number
        {
            match decode_ben_line(
                Cursor::new(&frame.raw_data),
                frame.max_val_bits,
                frame.max_len_bits,
                frame.n_bytes,
            ) {
                Ok(assignment_rle) => return Ok(rle_to_vec(assignment_rle)),
                Err(e) => return Err(SampleError::new_io_error(e)),
            };
        }
        current_sample += frame.count as usize;
    }

    Err(SampleError {
        kind: SampleErrorKind::SampleNotFound {
            sample_number: current_sample,
        },
    })
}

/// Extracts a single assignment from a binary-encoded data stream.
///
/// # Arguments
///
/// * `reader` - The reader to extract the assignment from.
/// * `sample_number` - The sample number to extract.
///
/// # Returns
///
/// This function returns a `Result` containing a `Vec<u16>` of the assignment if successful,
/// or a `SampleError` if an error occurred.
///
/// # Example
///
/// ```no_run
/// use ben::decode::read::extract_assignment_xben;
/// use std::{fs::File, io::BufReader};
///
/// let file = File::open("data.jsonl.xben").unwrap();
/// let reader = BufReader::new(file);
/// let sample_number = 2;
///
/// let result = extract_assignment_xben(reader, sample_number);
/// match result {
///     Ok(assignment) => {
///         eprintln!("Extracted assignment: {:?}", assignment);
///     }
///     Err(e) => {
///         eprintln!("Error: {}", e);
///     }
/// }
/// ```
///
/// # Errors
///
/// This function can return a `SampleError` if an error occurs during the extraction process.
/// The error can be one of the following:
/// * `InvalidSampleNumber` - The sample number is invalid. All sample numbers must be greater than 0.
/// * `SampleNotFound` - The sample number was not found in the file. The last sample number is provided.
/// * `IoError` - An IO error occurred during the extraction process.
/// * `JsonError` - A JSON error occurred during the extraction process.
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

// #[cfg(test)]
// mod tests {
//     include!("tests/read_tests.rs");
// }
#[cfg(test)]
#[path = "tests/read_tests.rs"]
mod tests;
