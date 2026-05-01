//! Sample extraction helpers for BEN and XBEN streams.

use crate::codec::decode::decode_ben32_line;
use crate::io::reader::{AssignmentReader, XZAssignmentReader};
use serde_json::Error as SerdeError;
use std::fs::File;
use std::io::Cursor;
use std::io::{self, BufReader, Read};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
/// Error returned by sample extraction helpers.
pub enum SampleError {
    /// The provided sample number was zero, which is invalid.
    #[error("Invalid sample number. Sample number must be greater than 0")]
    InvalidSampleNumber,

    /// The requested sample index was not found in the file.
    #[error(
        "Sample number not found in file. Failed to find sample '{sample_number}'. \
         Last sample seems to be '{}'",
        sample_number - 1
    )]
    SampleNotFound { sample_number: usize },

    /// An I/O error occurred during extraction.
    #[error("IO Error: {0}")]
    IoError(#[from] io::Error),

    /// A JSON parsing error occurred during extraction.
    #[error("JSON Error: {0}")]
    JsonError(#[from] SerdeError),
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
    /// Returns a new [`SampleError`] with [`SampleError::IoError`].
    pub fn new_io_error(error: io::Error) -> Self {
        SampleError::IoError(error)
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
        return Err(SampleError::InvalidSampleNumber);
    }

    let mut current_sample = 1;
    let inner_decoder = AssignmentReader::new(&mut reader).map_err(io::Error::from)?;
    for record in inner_decoder {
        let (assignment, count) = record.map_err(SampleError::new_io_error)?;
        if current_sample == sample_number || current_sample + count as usize > sample_number {
            return Ok(assignment);
        }
        current_sample += count as usize;
    }

    Err(SampleError::SampleNotFound {
        sample_number: current_sample,
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
        return Err(SampleError::InvalidSampleNumber);
    }

    let inner_decoder = XZAssignmentReader::new(&mut reader)
        .map_err(|e| SampleError::new_io_error(io::Error::from(e)))?;
    let variant = inner_decoder.variant();
    let frame_iterator = inner_decoder.into_frames();

    let mut current_sample = 1;
    for frame in frame_iterator {
        let frame = frame.map_err(SampleError::new_io_error)?;
        if current_sample == sample_number || current_sample + frame.1 as usize > sample_number {
            // XZAssignmentFrameReader guarantees complete zero-sentinel
            // frames, so decode_ben32_line always succeeds here.
            let (assignment, _) = decode_ben32_line(Cursor::new(&frame.0), variant)
                .expect("complete frame from XZAssignmentFrameReader");
            return Ok(assignment);
        }
        current_sample += frame.1 as usize;
    }

    Err(SampleError::SampleNotFound {
        sample_number: current_sample,
    })
}

/// Extract a single 1-based sample from a BEN file at `input`.
pub fn extract_assignment_ben_path(
    input: &Path,
    sample_number: usize,
) -> Result<Vec<u16>, SampleError> {
    let reader = BufReader::new(File::open(input).map_err(SampleError::new_io_error)?);
    extract_assignment_ben(reader, sample_number)
}

/// Extract a single 1-based sample from an XBEN file at `input`.
pub fn extract_assignment_xben_path(
    input: &Path,
    sample_number: usize,
) -> Result<Vec<u16>, SampleError> {
    let reader = BufReader::new(File::open(input).map_err(SampleError::new_io_error)?);
    extract_assignment_xben(reader, sample_number)
}

#[cfg(test)]
mod tests;
