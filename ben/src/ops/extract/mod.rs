//! Sample extraction helpers for BEN and XBEN streams.

use crate::codec::decode::decode_ben32_line;
use crate::io::reader::BenStreamReader;
use serde_json::Error as SerdeError;
use std::fs::File;
use std::io::Cursor;
use std::io::{self, BufReader, Read};
use std::path::Path;
use thiserror::Error;

use crate::BenVariant;

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
    let inner_decoder = BenStreamReader::from_ben(&mut reader).map_err(io::Error::from)?;

    if inner_decoder.variant() == BenVariant::TwoDelta {
        for record in inner_decoder {
            let (assignment, count) = record.map_err(SampleError::new_io_error)?;
            if current_sample == sample_number || current_sample + count as usize > sample_number {
                return Ok(assignment);
            }
            current_sample += count as usize;
        }
    } else {
        for frame in inner_decoder.into_frames() {
            let (decode_frame, count) = frame.map_err(SampleError::new_io_error)?;
            if current_sample == sample_number || current_sample + count as usize > sample_number {
                return decode_frame
                    .expand_self_contained()
                    .map_err(SampleError::new_io_error);
            }
            current_sample += count as usize;
        }
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

    let inner_decoder = BenStreamReader::from_xben(&mut reader)
        .map_err(|e| SampleError::new_io_error(io::Error::from(e)))?;
    let variant = inner_decoder.variant();
    let frame_iterator = inner_decoder.into_frames();

    let mut current_sample = 1;
    for frame in frame_iterator {
        let (decode_frame, count) = frame.map_err(SampleError::new_io_error)?;
        if current_sample == sample_number || current_sample + count as usize > sample_number {
            // The frame iterator guarantees structurally complete zero-sentinel ben32 frames in
            // the XBEN arm, but the runs inside can still be semantically corrupt (zero-length
            // run, oversized expansion), so the decode is fallible.
            let bytes = match &decode_frame {
                crate::io::reader::DecodeFrame::XBen(b, _) => b,
                crate::io::reader::DecodeFrame::Ben(_) => {
                    unreachable!("XBEN reader yields XBen frames")
                }
            };
            let (assignment, _) = decode_ben32_line(Cursor::new(bytes), variant)
                .map_err(SampleError::new_io_error)?;
            return Ok(assignment);
        }
        current_sample += count as usize;
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
