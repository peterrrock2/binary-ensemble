//! Module documentation.
//!
//! This module provides functionality for extracting single assignment
//! vectors from a BEN file.
use serde_json::{Error as SerdeError, Value};
use std::fmt::{self};

use super::*;

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
                    "Sample number not found in file. Last sample is {}",
                    sample_number
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

    let mut check_buffer = [0u8; 17];
    reader.read_exact(&mut check_buffer)?;

    if &check_buffer != b"STANDARD BEN FILE" {
        return Err(SampleError {
            kind: SampleErrorKind::IoError(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid file format",
            )),
        });
    }

    let mut r_sample = 1;
    let mut writer = Vec::new();
    loop {
        let mut tmp_buffer = [0u8];
        let max_val_bits: u8 = match reader.read_exact(&mut tmp_buffer) {
            Ok(()) => tmp_buffer[0],
            Err(e) => {
                if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    return Err(SampleError {
                        kind: SampleErrorKind::SampleNotFound {
                            sample_number: r_sample,
                        },
                    });
                }
                return Err(e.into());
            }
        };
        let max_len_bits = reader.read_u8()?;
        let n_bytes = reader.read_u32::<BigEndian>()?;

        let mut assign_bits: Vec<u8> = vec![0; n_bytes as usize];
        reader.read_exact(&mut assign_bits)?;

        // Reader buffer gets thrown away after each iteration
        // and only decoded if we are in the right sample.
        // This speeds up the process significantly by not decoding all samples.
        if r_sample == sample_number {
            // Write the ben header that is expected by jsonl_decode_ben
            let mut tmp_reader = b"STANDARD BEN FILE".to_vec();
            // Write the actual ben data
            tmp_reader.extend(vec![max_val_bits, max_len_bits]);
            tmp_reader.extend(n_bytes.to_be_bytes().to_vec());
            tmp_reader.extend(assign_bits);

            jsonl_decode_ben(&mut tmp_reader.as_slice(), &mut writer)?;
            break;
        }
        r_sample += 1;
    }

    let decoded = serde_json::from_str::<Value>(&String::from_utf8(writer).unwrap())?;
    let assignment = decoded["assignment"]
        .as_array()
        .unwrap()
        .into_iter()
        .map(|x| x.as_u64().unwrap() as u16)
        .collect::<Vec<u16>>();

    Ok(assignment)
}

#[cfg(test)]
mod tests {
    include!("tests/read_tests.rs");
}
