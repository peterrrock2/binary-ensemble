//! Sample extraction helpers for BEN and XBEN streams.

use crate::codec::decode::{decode_ben32_line, decode_ben_line};
use crate::io::reader::{BenDecoder, XBenDecoder};
use crate::util::rle::rle_to_vec;
use serde_json::Error as SerdeError;
use std::fmt;
use std::io::Cursor;
use std::io::{self, Read};

#[derive(Debug)]
pub enum SampleErrorKind {
    InvalidSampleNumber,
    SampleNotFound { sample_number: usize },
    IoError(io::Error),
    JsonError(SerdeError),
}

#[derive(Debug)]
pub struct SampleError {
    pub kind: SampleErrorKind,
}

impl SampleError {
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
