//! Sample extraction helpers for BEN and XBEN streams.

use crate::codec::decode::decode_ben32_line;
use crate::codec::frames::{check_payload_len, check_twodelta_run_width};
use crate::format::banners::{variant_from_banner, BANNER_LEN, TWODELTA_BEN_BANNER};
use crate::io::reader::twodelta::{BEN_TWODELTA_DELTA_TAG, BEN_TWODELTA_SNAPSHOT_TAG};
use crate::io::reader::BenStreamReader;
use serde_json::Error as SerdeError;
use std::fs::File;
use std::io::Cursor;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
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
    let mut inner_decoder = BenStreamReader::from_ben(&mut reader).map_err(io::Error::from)?;

    if inner_decoder.variant() == BenVariant::TwoDelta {
        let mut found = None;
        inner_decoder
            .for_each_assignment(|assignment, count| {
                if current_sample == sample_number
                    || current_sample + count as usize > sample_number
                {
                    found = Some(assignment.to_vec());
                    return Ok(false);
                }
                current_sample += count as usize;
                Ok(true)
            })
            .map_err(SampleError::new_io_error)?;
        if let Some(assignment) = found {
            return Ok(assignment);
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

/// Extract a single 1-based sample from a seekable uncompressed BEN stream.
///
/// Plain TwoDelta streams use a byte-level pre-scan to seek to the latest snapshot before the
/// requested sample, then replay from there. Other variants delegate to [`extract_assignment_ben`].
pub fn extract_assignment_ben_seek<R: Read + Seek>(
    mut reader: R,
    sample_number: usize,
) -> Result<Vec<u16>, SampleError> {
    if sample_number == 0 {
        return Err(SampleError::InvalidSampleNumber);
    }

    let mut banner = [0u8; BANNER_LEN];
    reader.read_exact(&mut banner)?;
    let variant = variant_from_banner(&banner)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "unrecognized BEN banner"))?;
    reader.seek(SeekFrom::Start(0))?;

    if variant != BenVariant::TwoDelta {
        return extract_assignment_ben(reader, sample_number);
    }

    let (snapshot_offset, snapshot_sample) =
        latest_twodelta_snapshot_before(&mut reader, sample_number)?;
    reader.seek(SeekFrom::Start(snapshot_offset))?;

    let mut prefixed = Cursor::new(TWODELTA_BEN_BANNER.as_slice()).chain(&mut reader);
    let mut decoder = BenStreamReader::from_ben(&mut prefixed).map_err(io::Error::from)?;
    let mut current_sample = snapshot_sample;
    let mut found = None;
    decoder
        .for_each_assignment(|assignment, count| {
            if current_sample == sample_number || current_sample + count as usize > sample_number {
                found = Some(assignment.to_vec());
                return Ok(false);
            }
            current_sample += count as usize;
            Ok(true)
        })
        .map_err(SampleError::new_io_error)?;

    found.ok_or(SampleError::SampleNotFound {
        sample_number: current_sample,
    })
}

fn latest_twodelta_snapshot_before<R: Read + Seek>(
    reader: &mut R,
    sample_number: usize,
) -> Result<(u64, usize), SampleError> {
    reader.seek(SeekFrom::Start(BANNER_LEN as u64))?;

    let mut current_sample = 1usize;
    let mut latest_snapshot = None;

    loop {
        let frame_offset = reader.stream_position()?;
        let tag = match read_u8_or_clean_eof(reader)? {
            Some(tag) => tag,
            None => break,
        };
        let count = match tag {
            BEN_TWODELTA_SNAPSHOT_TAG => {
                latest_snapshot = Some((frame_offset, current_sample));
                read_twodelta_snapshot_count(reader)?
            }
            BEN_TWODELTA_DELTA_TAG => read_twodelta_delta_count(reader)?,
            other => {
                return Err(SampleError::new_io_error(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("unknown TwoDelta frame tag byte {other:#04x}"),
                )))
            }
        };
        if count == 0 {
            return Err(SampleError::new_io_error(io::Error::new(
                io::ErrorKind::InvalidData,
                "BEN frame count must be greater than zero",
            )));
        }

        if current_sample == sample_number || current_sample + count as usize > sample_number {
            return latest_snapshot.ok_or_else(|| {
                SampleError::new_io_error(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "TwoDelta stream has no snapshot before requested sample",
                ))
            });
        }
        current_sample += count as usize;
    }

    Err(SampleError::SampleNotFound {
        sample_number: current_sample,
    })
}

fn read_u8_or_clean_eof(reader: &mut impl Read) -> io::Result<Option<u8>> {
    let mut byte = [0u8; 1];
    match reader.read_exact(&mut byte) {
        Ok(()) => Ok(Some(byte[0])),
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Ok(None),
        Err(e) => Err(e),
    }
}

fn read_twodelta_snapshot_count<R: Read + Seek>(reader: &mut R) -> io::Result<u16> {
    let mut header = [0u8; 6];
    reader.read_exact(&mut header)?;
    let n_bytes = u32::from_be_bytes([header[2], header[3], header[4], header[5]]);
    check_payload_len(n_bytes)?;
    reader.seek(SeekFrom::Current(i64::from(n_bytes)))?;
    read_u16_be(reader)
}

fn read_twodelta_delta_count<R: Read + Seek>(reader: &mut R) -> io::Result<u16> {
    let mut header = [0u8; 9];
    reader.read_exact(&mut header)?;
    let max_len_bits = header[4];
    check_twodelta_run_width(max_len_bits)?;
    let n_bytes = u32::from_be_bytes([header[5], header[6], header[7], header[8]]);
    check_payload_len(n_bytes)?;
    reader.seek(SeekFrom::Current(i64::from(n_bytes)))?;
    read_u16_be(reader)
}

fn read_u16_be(reader: &mut impl Read) -> io::Result<u16> {
    let mut bytes = [0u8; 2];
    reader.read_exact(&mut bytes)?;
    Ok(u16::from_be_bytes(bytes))
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
    extract_assignment_ben_seek(reader, sample_number)
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
