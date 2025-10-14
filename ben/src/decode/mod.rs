//! This module contains the main functions for decoding XBEN and BEN files.
//!
//! XBEN files are generally transformed back into BEN files, and BEN files
//! are transformed into a JSONL file with the formatting
//!
//! ```json
//! {"assignment": [...], "sample": #}
//! ```
//!
//! The BEN file format is a bit-packed binary format that is used to store
//! run-length encoded assignment vectors, and is streamable. Therefore, the
//! BEN file format works well with the `read` submodule of this module
//! which is designed to extract a single assignment vector from a BEN file.
pub mod read;

use byteorder::{BigEndian, ReadBytesExt};
use serde_json::json;
use std::fs::File;
use std::io::{self, BufRead, Read, Write}; // trait imports
use std::io::{BufReader, Cursor, Error}; // type import
use std::iter::Peekable;
use std::path::Path;
use std::path::PathBuf;
use xz2::read::XzDecoder;

use crate::utils::rle_to_vec;

use super::encode::translate::*;
use super::{log, logln, BenVariant};

pub type MkvRecord = (Vec<u16>, u16);

#[derive(Debug)]
pub enum DecoderInitError {
    InvalidFileFormat(Vec<u8>),
    Io(io::Error),
}

/// Check if the given header matches the XZ magic number.
/// This is used to provide a more informative error message when
/// a user tries to decode a compressed .xben file with the
/// `BenDecoder` instead of the `decode_xben_to_ben` function.
fn is_xz_header(h: &[u8]) -> bool {
    h.len() >= 6 && &h[..6] == b"\xFD\x37\x7A\x58\x5A\x00"
}

/// Convert a byte slice to a hex string for display purposes.
/// Each byte is represented as two uppercase hex digits, separated by spaces.
fn to_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{:02X}", b))
        .collect::<Vec<_>>()
        .join(" ")
}

impl std::fmt::Display for DecoderInitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "IO error: {e}"),
            Self::InvalidFileFormat(header) => {
                if is_xz_header(header) {
                    write!(
                        f,
                        "Invalid file format: Compressed header detected (hex: {}). \
                     This reader expects an uncompressed .ben file. \
                     Decompress this file using the BEN cli `ben -m decode <file_name>.xben` tool \
                     or the `decode_xben_to_ben` function in this library.",
                        to_hex(header)
                    )
                } else {
                    let lossy = String::from_utf8_lossy(header);
                    write!(
                        f,
                        "Invalid file format. Found header (utf8-lossy: {lossy:?}, hex: {})",
                        to_hex(header)
                    )
                }
            }
        }
    }
}

impl std::error::Error for DecoderInitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DecoderInitError::Io(e) => Some(e),
            DecoderInitError::InvalidFileFormat(_) => None,
        }
    }
}

impl From<io::Error> for DecoderInitError {
    fn from(error: io::Error) -> Self {
        DecoderInitError::Io(error)
    }
}

impl From<DecoderInitError> for io::Error {
    fn from(error: DecoderInitError) -> Self {
        match error {
            DecoderInitError::Io(e) => e,
            DecoderInitError::InvalidFileFormat(msg) => {
                io::Error::new(io::ErrorKind::InvalidData, format!("{msg:?}"))
            }
        }
    }
}

pub struct BenDecoder<R: Read> {
    reader: R,
    sample_count: usize,
    variant: BenVariant,
}

/// A single frame from a BEN file.
#[derive(Clone)]
pub struct BenFrame {
    pub max_val_bits: u8,  // number of bits used for each value
    pub max_len_bits: u8,  // number of bits used for each run-length
    pub count: u16,        // repetition count (1 for Standard)
    pub n_bytes: u32,      // number of bytes used for the raw assignment data
    pub raw_data: Vec<u8>, // raw bit-compressed BEN data
}

impl<R: Read> BenDecoder<R> {
    /// Create a new BenDecoder from a reader.
    /// The reader must contain a valid BEN file.
    /// The first 17 bytes of the file are checked to determine
    /// the variant of the BEN file.
    pub fn new(mut reader: R) -> Result<Self, DecoderInitError> {
        let mut check_buffer = [0u8; 17];

        if let Err(e) = reader.read_exact(&mut check_buffer) {
            return Err(DecoderInitError::Io(e));
        }

        match &check_buffer {
            b"STANDARD BEN FILE" => Ok(BenDecoder {
                reader,
                sample_count: 0,
                variant: BenVariant::Standard,
            }),
            b"MKVCHAIN BEN FILE" => Ok(BenDecoder {
                reader,
                sample_count: 0,
                variant: BenVariant::MkvChain,
            }),
            _ => Err(DecoderInitError::InvalidFileFormat(check_buffer.to_vec())),
        }
    }

    /// Write all decoded assignments to a writer in JSONL format.
    ///
    /// Arguments:
    ///
    /// * `writer`: A mutable reference to a writer where the JSONL output will be written.
    fn write_all_jsonl(&mut self, mut writer: impl Write) -> io::Result<()> {
        while let Some(result_tuple) = self.next() {
            match result_tuple {
                Ok((assignment, count)) => {
                    for _ in 0..count {
                        self.sample_count += 1;
                        let line = json!({
                            "assignment": assignment,
                            "sample": self.sample_count,
                        })
                        .to_string()
                            + "\n";
                        writer.write_all(line.as_bytes()).unwrap();
                    }
                }
                Err(e) => {
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    /// Internal helper function that pops a single ben frame from the reader.
    /// This frame may then either be decoded into an assignment vector
    /// or returned as-is for further processing.
    fn pop_frame_from_reader(&mut self) -> Option<io::Result<BenFrame>> {
        let mut b1 = [0u8; 1];
        let max_val_bits = match self.reader.read_exact(&mut b1) {
            Ok(()) => b1[0],
            Err(e) => {
                if e.kind() == io::ErrorKind::UnexpectedEof {
                    // clean EOF before starting a new frame
                    logln!();
                    logln!("Done!");
                    return None;
                }
                return Some(Err(e));
            }
        };

        let mut b2 = [0u8; 1];
        if let Err(e) = self.reader.read_exact(&mut b2) {
            return Some(Err(e));
        }
        let max_len_bits = b2[0];

        let n_bytes = match self.reader.read_u32::<BigEndian>() {
            Ok(n) => n,
            Err(e) => return Some(Err(e)),
        };

        let mut raw_assignment = vec![0u8; n_bytes as usize];
        if let Err(e) = self.reader.read_exact(&mut raw_assignment) {
            return Some(Err(e));
        }

        let count = if self.variant == BenVariant::MkvChain {
            match self.reader.read_u16::<BigEndian>() {
                Ok(c) => c,
                Err(e) => return Some(Err(e)),
            }
        } else {
            1
        };

        Some(Ok(BenFrame {
            max_val_bits,
            max_len_bits,
            n_bytes,
            raw_data: raw_assignment,
            count,
        }))
    }
}

/// Helper function to decode a ben frame into an assignment vector.
fn decode_ben_frame_to_assignment(frame: &BenFrame) -> io::Result<Vec<u16>> {
    decode_ben_line(
        Cursor::new(&frame.raw_data),
        frame.max_val_bits,
        frame.max_len_bits,
        frame.n_bytes,
    )
    .map(rle_to_vec)
}

impl<R: Read> Iterator for BenDecoder<R> {
    type Item = io::Result<MkvRecord>;

    fn next(&mut self) -> Option<io::Result<MkvRecord>> {
        let ben_frame = match self.pop_frame_from_reader() {
            Some(Ok(frame)) => frame,
            Some(Err(e)) => return Some(Err(e)),
            None => return None,
        };
        let assignment = match decode_ben_frame_to_assignment(&ben_frame) {
            Ok(assgn) => assgn,
            Err(e) => return Some(Err(e)),
        };
        log!(
            "Decoding sample: {}\r",
            self.sample_count + ben_frame.count as usize
        );
        Some(Ok((assignment, ben_frame.count)))
    }
}

pub struct BenFrameDecoeder<R: Read> {
    inner: BenDecoder<R>,
}

impl<R: Read> BenFrameDecoeder<R> {
    pub fn new(reader: R) -> io::Result<Self> {
        Ok(Self {
            inner: BenDecoder::new(reader)?,
        })
    }
}

impl<R: Read> Iterator for BenFrameDecoeder<R> {
    type Item = io::Result<BenFrame>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.pop_frame_from_reader()
    }
}

impl<R: Read> BenDecoder<R> {
    /// Consume this decoder and iterate raw ben frames instead of decoded assignments.
    pub fn into_frames(self) -> BenFrameDecoeder<R> {
        BenFrameDecoeder { inner: self }
    }
}

impl<R: Read> BenDecoder<R> {
    /// Count how many samples remain in this BEN stream.
    /// Consumes the decoder (fast: walks frames only).
    pub fn count_samples(self) -> io::Result<usize> {
        let mut total = 0usize;
        for frame_res in self.into_frames() {
            let f = frame_res?; // BenFrame
            total += f.count as usize; // 1 for Standard; >1 for MKVCHAIN
        }
        Ok(total)
    }
}

/// This function takes a reader containing a single ben32 encoded assignment
/// vector and decodes it into a full assignment vector of u16s.
///
/// # Errors
///
/// This function will return an error if the input reader is not a multiple of 4
/// bytes long since each assignment vector is an run-length encoded as a 32 bit
/// integer (2 bytes for the value and 2 bytes for the count).
///
fn decode_ben32_line<R: BufRead>(mut reader: R, variant: BenVariant) -> io::Result<MkvRecord> {
    let mut buffer = [0u8; 4];
    let mut output_vec: Vec<u16> = Vec::new();

    loop {
        match reader.read_exact(&mut buffer) {
            Ok(()) => {
                let encoded = u32::from_be_bytes(buffer);
                if encoded == 0 {
                    // Check for separator (all 0s)
                    break; // Exit loop to process next sample
                }

                let value = (encoded >> 16) as u16; // High 16 bits
                let count = (encoded & 0xFFFF) as u16; // Low 16 bits

                // Reconstruct the original data
                for _ in 0..count {
                    output_vec.push(value);
                }
            }
            Err(e) => {
                return Err(e); // Propagate other errors
            }
        }
    }

    let count = if variant == BenVariant::MkvChain {
        reader
            .read_u16::<BigEndian>()
            .expect("Error when reading sample.")
    } else {
        1
    };

    Ok((output_vec, count))
}

/// This function takes a reader containing a file encoded with the
/// "ben32" format and decodes it into a JSONL file.
///
/// The output JSONL file will have the formatting
///
/// ```json
/// {"assignment": [...], "sample": #}
/// ```
///
/// # Errors
///
/// This function will return an error if the input reader contains invalid ben32
/// data or if the the decode method encounters while trying to extract a single
/// assignment vector, that error is propagated.
fn jsonl_decode_ben32<R: BufRead, W: Write>(
    mut reader: R,
    mut writer: W,
    starting_sample: usize,
    variant: BenVariant,
) -> io::Result<()> {
    let mut sample_number = 1;
    loop {
        let result = decode_ben32_line(&mut reader, variant);
        if let Err(e) = result {
            if e.kind() == io::ErrorKind::UnexpectedEof {
                return Ok(());
            }
            return Err(e);
        }

        let (output_vec, count) = result.unwrap();

        for _ in 0..count {
            // Write the reconstructed vector as JSON to the output file
            let line = json!({
                "assignment": output_vec,
                "sample": sample_number + starting_sample,
            })
            .to_string()
                + "\n";

            writer.write_all(line.as_bytes())?;
            sample_number += 1;
        }
    }
}

/// This function takes a reader containing a file encoded in the XBEN format
/// and decodes it into a BEN file.
///
/// # Errors
///
/// This function will return an error if the input reader contains invalid xben
/// data or if the the decode method encounters while trying to convert the
/// xben data to ben data.
pub fn decode_xben_to_ben<R: BufRead, W: Write>(reader: R, mut writer: W) -> io::Result<()> {
    let mut decoder = XzDecoder::new(reader);

    let mut first_buffer = [0u8; 17];

    if let Err(e) = decoder.read_exact(&mut first_buffer) {
        return Err(e);
    }

    let variant = match &first_buffer {
        b"STANDARD BEN FILE" => {
            writer.write_all(b"STANDARD BEN FILE")?;
            BenVariant::Standard
        }
        b"MKVCHAIN BEN FILE" => {
            writer.write_all(b"MKVCHAIN BEN FILE")?;
            BenVariant::MkvChain
        }
        _ => {
            return Err(Error::new(
                io::ErrorKind::InvalidData,
                "Invalid file format",
            ));
        }
    };

    let mut buffer = [0u8; 1048576]; // 1MB buffer
    let mut overflow: Vec<u8> = Vec::new();

    let mut line_count: usize = 0;
    while let Ok(count) = decoder.read(&mut buffer) {
        if count == 0 {
            break;
        }

        overflow.extend(&buffer[..count]);

        let mut last_valid_assignment = 0;

        // It is technically faster to read backwards from the last
        // multiple of 4 smaller than the length of the overflow buffer
        // but this provides only a minute speedup in almost all cases (maybe a
        // few seconds). Reading from the front is both safer from a
        // maintenance perspective and allows for a better progress indicator
        match variant {
            BenVariant::Standard => {
                for i in (3..overflow.len()).step_by(4) {
                    if overflow[i - 3..=i] == [0, 0, 0, 0] {
                        last_valid_assignment = i + 1;
                        line_count += 1;
                        log!("Decoding sample: {}\r", line_count);
                    }
                }
            }
            BenVariant::MkvChain => {
                for i in (3..overflow.len() - 2).step_by(2) {
                    if overflow[i - 3..=i] == [0, 0, 0, 0] {
                        last_valid_assignment = i + 3;
                        let lines = &overflow[i + 1..i + 3];
                        let n_lines = u16::from_be_bytes([lines[0], lines[1]]);
                        line_count += n_lines as usize;
                        log!("Decoding sample: {}\r", line_count);
                    }
                }
            }
        }

        if last_valid_assignment == 0 {
            continue;
        }

        ben32_to_ben_lines(&overflow[0..last_valid_assignment], &mut writer, variant)?;
        overflow = overflow[last_valid_assignment..].to_vec();
    }
    logln!();
    logln!("Done!");
    Ok(())
}

/// This is a convenience function that decodes a general level 9 LZMA2 compressed file.
///
/// ```
/// use ben::encode::xz_compress;
/// use ben::decode::xz_decompress;
/// use lipsum::lipsum;
/// use std::io::{BufReader, BufWriter};
///
/// let input = lipsum(100);
/// let reader = BufReader::new(input.as_bytes());
/// let mut output_buffer = Vec::new();
/// let writer = BufWriter::new(&mut output_buffer);
///
/// xz_compress(reader, writer, Some(1), Some(1)).unwrap();
///
/// let mut recovery_buff = Vec::new();
/// let recovery_reader = BufWriter::new(&mut recovery_buff);
/// xz_decompress(output_buffer.as_slice(), recovery_reader).unwrap();
/// println!("{:?}", output_buffer);
/// ```
pub fn xz_decompress<R: BufRead, W: Write>(reader: R, mut writer: W) -> io::Result<()> {
    let mut decoder = XzDecoder::new(reader);
    let mut buffer = [0u8; 4096];

    while let Ok(count) = decoder.read(&mut buffer) {
        if count == 0 {
            break;
        }
        writer.write_all(&buffer[..count])?;
    }

    Ok(())
}

/// This is a helper function that is designed to read in a single
/// ben encoded line and convert it to a regular run-length encoded
/// assignment vector.
pub fn decode_ben_line<R: Read>(
    mut reader: R,
    max_val_bits: u8,
    max_len_bits: u8,
    n_bytes: u32,
) -> io::Result<Vec<(u16, u16)>> {
    let mut assign_bits: Vec<u8> = vec![0; n_bytes as usize];
    reader.read_exact(&mut assign_bits)?;

    // This should be right, but it doesn't need to be exact
    let n_assignments: usize =
        (n_bytes as f64 / ((max_val_bits + max_len_bits) as f64 / 8.0)) as usize;
    let mut output_rle: Vec<(u16, u16)> = Vec::with_capacity(n_assignments);

    let mut buffer: u32 = 0;
    let mut n_bits_in_buff: u16 = 0;

    let mut val = 0;
    let mut val_set = false;
    let mut len = 0;
    let mut len_set = false;

    for (_, &byte) in assign_bits.iter().enumerate() {
        buffer = buffer | ((byte as u32).to_be() >> (n_bits_in_buff));
        n_bits_in_buff += 8;

        if n_bits_in_buff >= max_val_bits as u16 && !val_set {
            val = (buffer >> (32 - max_val_bits)) as u16;

            buffer = (buffer << max_val_bits) as u32;
            n_bits_in_buff -= max_val_bits as u16;
            val_set = true;
        }

        if n_bits_in_buff >= max_len_bits as u16 && val_set && !len_set {
            len = (buffer >> (32 - max_len_bits)) as u16;
            buffer = buffer << max_len_bits;
            n_bits_in_buff -= max_len_bits as u16;
            len_set = true;
        }

        if val_set && len_set {
            // If max_val_bits and max_len_bits are <= 4
            // then the rle can bet (0,0) pairs pushed to it
            if len > 0 {
                output_rle.push((val, len));
            }
            val_set = false;
            len_set = false;
        }

        while n_bits_in_buff >= max_val_bits as u16 + max_len_bits as u16 {
            if n_bits_in_buff >= max_val_bits as u16 && !val_set {
                val = (buffer >> (32 - max_val_bits)) as u16;
                buffer = (buffer << max_val_bits) as u32;
                n_bits_in_buff -= max_val_bits as u16;
                val_set = true;
            }

            if n_bits_in_buff >= max_len_bits as u16 && val_set && !len_set {
                len = (buffer >> (32 - max_len_bits)) as u16;
                buffer = buffer << max_len_bits;
                n_bits_in_buff -= max_len_bits as u16;
                len_set = true;
            }

            if val_set && len_set {
                // If the max_val_bits and max_len_bits are <= 4
                // then the rle can bet (0,0) pairs pushed to it
                if len > 0 {
                    output_rle.push((val, len));
                }
                val_set = false;
                len_set = false;
            }
        }
    }

    Ok(output_rle)
}

/// This function takes a reader containing a file encoded in the BEN format
/// and decodes it into a JSONL file.
///
/// The output JSONL file will have the formatting
///
/// ```json
/// {"assignment": [...], "sample": #}
/// ```
///
/// # Errors
///
/// This function will return an error if the input reader contains invalid ben
/// data or if the the decode method encounters while trying to extract a single
/// assignment vector, that error is then propagated.
pub fn decode_ben_to_jsonl<R: Read, W: Write>(reader: R, writer: W) -> io::Result<()> {
    let mut ben_decoder = BenDecoder::new(reader)?;
    ben_decoder.write_all_jsonl(writer)
}

/// This function takes a reader containing a file encoded in the XBEN format
/// and decodes it into a JSONL file.
///
/// The output JSONL file will have the formatting
///
/// ```json
/// {"assignment": [...], "sample": #}
/// ```
///
/// # Errors
///
/// This function will return an error if the input reader contains invalid xben
/// data or if the the decode method encounters while trying to extract a single
/// assignment vector, that error is then propagated.
pub fn decode_xben_to_jsonl<R: BufRead, W: Write>(reader: R, mut writer: W) -> io::Result<()> {
    let mut decoder = XzDecoder::new(reader);

    let mut first_buffer = [0u8; 17];

    if let Err(e) = decoder.read_exact(&mut first_buffer) {
        return Err(e);
    }

    let variant = match &first_buffer {
        b"STANDARD BEN FILE" => BenVariant::Standard,
        b"MKVCHAIN BEN FILE" => BenVariant::MkvChain,
        _ => {
            return Err(Error::new(
                io::ErrorKind::InvalidData,
                "Invalid file format",
            ));
        }
    };

    let mut buffer = [0u8; 1 << 20]; // 1MB buffer
    let mut overflow: Vec<u8> = Vec::new();

    let mut line_count: usize = 0;
    let mut starting_sample: usize = 0;
    while let Ok(count) = decoder.read(&mut buffer) {
        if count == 0 {
            break;
        }

        overflow.extend(&buffer[..count]);

        let mut last_valid_assignment = 0;

        // It is technically faster to read backwards from the last
        // multiple of 4 smaller than the length of the overflow buffer
        // but this provides only a minute speedup in almost all cases (maybe a
        // few seconds). Reading from the front is both safer from a
        // maintenance perspective and allows for a better progress indicator
        match variant {
            BenVariant::Standard => {
                for i in (3..overflow.len()).step_by(4) {
                    if overflow[i - 3..=i] == [0, 0, 0, 0] {
                        last_valid_assignment = i + 1;
                        line_count += 1;
                        log!("Decoding sample: {}\r", line_count);
                    }
                }
            }
            BenVariant::MkvChain => {
                // Need a different step size here because each assignment
                // vector is no longer guaranteed to be a multiple of 4 bytes
                // due to the 2-byte repetition count appended at the end
                for i in (last_valid_assignment + 3..overflow.len().saturating_sub(2)).step_by(2) {
                    if overflow[i - 3..=i] == [0, 0, 0, 0] {
                        last_valid_assignment = i + 3;
                        let lines = &overflow[i + 1..i + 3];
                        let n_lines = u16::from_be_bytes([lines[0], lines[1]]);
                        line_count += n_lines as usize;
                        log!("Decoding sample: {}\r", line_count);
                    }
                }
            }
        }

        if last_valid_assignment == 0 {
            continue;
        }

        jsonl_decode_ben32(
            &overflow[0..last_valid_assignment],
            &mut writer,
            starting_sample,
            variant,
        )?;
        overflow.drain(..last_valid_assignment);
        starting_sample = line_count;
    }
    logln!();
    logln!("Done!");
    Ok(())
}

/// Iterator over decoded assignments inside an XBEN stream.
/// Yields `(assignment, count)` where `count` is the repetition count
pub struct XBenDecoder<R: Read> {
    xz: BufReader<XzDecoder<R>>,
    pub variant: BenVariant,
    overflow: Vec<u8>,
    buf: Box<[u8]>, // reusable read buffer
}

impl<R: Read> XBenDecoder<R> {
    pub fn new(reader: R) -> io::Result<Self> {
        let xz = XzDecoder::new(reader);
        let mut xz = BufReader::with_capacity(1 << 20, xz);

        // Read the 17-byte banner to determine variant
        let mut first = [0u8; 17];
        xz.read_exact(&mut first)?;
        let variant = match &first {
            b"STANDARD BEN FILE" => BenVariant::Standard,
            b"MKVCHAIN BEN FILE" => BenVariant::MkvChain,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Invalid .xben header (expecting STANDARD/MKVCHAIN BEN FILE)",
                ));
            }
        };

        Ok(Self {
            xz,
            variant,
            overflow: Vec::with_capacity(1 << 20),
            buf: vec![0u8; 1 << 20].into_boxed_slice(),
        })
    }

    /// Try to pop one *complete* ben32 frame from `overflow`.
    ///
    /// # Arguments
    ///
    /// * `overflow` - A byte slice that may contain one or more complete ben32 frames.
    ///
    /// # Returns
    ///
    /// An Option containing a tuple of:
    ///
    /// * the complete frame as a byte slice,
    /// * the number of bytes consumed from the start of `overflow` to get this frame,
    fn pop_frame_from_overflow<'a>(&self, overflow: &'a [u8]) -> Option<(&'a [u8], usize, u16)> {
        match self.variant {
            BenVariant::Standard => {
                // Frame ends right after 4 zero bytes
                // ... [payload] ... 00 00 00 00
                if overflow.len() < 4 {
                    return None;
                }
                for i in (3..overflow.len()).step_by(4) {
                    if overflow[i - 3..=i] == [0, 0, 0, 0] {
                        let end = i + 1;
                        let frame = &overflow[..end];
                        // In STANDARD, count is always 1
                        return Some((frame, end, 1));
                    }
                }
                None
            }
            BenVariant::MkvChain => {
                // ... [payload] ... 00 00 00 00 <n_lines_hi_byte> <n_lines_lo_byte>
                if overflow.len() < 6 {
                    return None;
                }
                for i in (3..overflow.len().saturating_sub(2)).step_by(2) {
                    if overflow[i - 3..=i] == [0, 0, 0, 0] {
                        let count_hi = overflow[i + 1];
                        let count_lo = overflow[i + 2];
                        let count = u16::from_be_bytes([count_hi, count_lo]);
                        let end = i + 3; // inclusive of count bytes
                        let frame = &overflow[..end];
                        return Some((frame, end, count));
                    }
                }
                None
            }
        }
    }
}

/// Helper function to decode a ben32 frame (raw bytes) into an assignment vector.
fn decode_xben_frame_to_assignment(
    frame_bytes: &[u8],
    variant: BenVariant,
) -> io::Result<Vec<u16>> {
    let cursor = Cursor::new(frame_bytes);
    let (assignment, _) = decode_ben32_line(cursor, variant)?;
    Ok(assignment)
}

impl<R: Read> Iterator for XBenDecoder<R> {
    type Item = io::Result<MkvRecord>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            // If we already have a complete frame in overflow, decode and return it
            if let Some((frame_bytes, consumed, count)) =
                self.pop_frame_from_overflow(&self.overflow)
            {
                let res = match decode_xben_frame_to_assignment(frame_bytes, self.variant) {
                    Ok(assignment) => Ok((assignment, count)),
                    Err(e) => Err(e),
                };
                // drop the used bytes
                self.overflow.drain(..consumed);
                return Some(res);
            }

            // Otherwise, read more from the XZ stream
            let read = match self.xz.read(&mut self.buf) {
                Ok(0) => {
                    // EOF: no more data; if there's leftover but not a full frame, report error or stop
                    if self.overflow.is_empty() {
                        return None;
                    } else {
                        return Some(Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "truncated .xben stream (partial frame at EOF)",
                        )));
                    }
                }
                Ok(n) => n,
                Err(e) => return Some(Err(e)),
            };
            self.overflow.extend_from_slice(&self.buf[..read]);
        }
    }
}

/// A frame is the raw ben32 bytes plus its repetition count (1 for Standard).
pub type Ben32Frame = (Vec<u8>, u16);

/// Iterator over raw ben32 frames inside an XBEN stream.
///
/// Yields `(frame_bytes, count)` where `frame_bytes` includes the 4-byte
/// 0x00_00_00_00 terminator; for `MkvChain` frames it also includes the
/// 2-byte big-endian repetition count at the end. `count` is the decoded
/// repetition count (1 for Standard).
///
/// Mainly useful for finding an assignment quickly
pub struct XBenFrameDecoder<R: Read> {
    inner: XBenDecoder<R>,
}

impl<R: Read> XBenFrameDecoder<R> {
    pub fn new(reader: R) -> io::Result<Self> {
        Ok(Self {
            inner: XBenDecoder::new(reader)?,
        })
    }
}

impl<R: Read> Iterator for XBenFrameDecoder<R> {
    type Item = io::Result<Ben32Frame>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some((frame, consumed, count)) =
                self.inner.pop_frame_from_overflow(&self.inner.overflow)
            {
                // copy out the frame; caller owns the bytes
                let out = frame.to_vec();
                self.inner.overflow.drain(..consumed);
                return Some(Ok((out, count)));
            }

            // refill from xz
            let read = match self.inner.xz.read(&mut self.inner.buf) {
                Ok(0) => {
                    if self.inner.overflow.is_empty() {
                        return None;
                    } else {
                        return Some(Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "truncated .xben stream (partial frame at EOF)",
                        )));
                    }
                }
                Ok(n) => n,
                Err(e) => return Some(Err(e)),
            };
            self.inner
                .overflow
                .extend_from_slice(&self.inner.buf[..read]);
        }
    }
}

impl<R: Read> XBenDecoder<R> {
    /// Consumes the decoder and iterate raw ben32 frames instead of decoded assignments.
    pub fn into_frames(self) -> XBenFrameDecoder<R> {
        XBenFrameDecoder { inner: self }
    }
}

impl<R: Read> XBenDecoder<R> {
    /// Count how many samples remain in this XBEN stream.
    /// Consumes the decoder (fast: walks frames only).
    pub fn count_samples(self) -> io::Result<usize> {
        let mut total = 0usize;
        for frame_res in self.into_frames() {
            let (_bytes, cnt) = frame_res?; // raw ben32 bytes + repetition count
            total += cnt as usize;
        }
        Ok(total)
    }
}

/// A generalized frame object that can be either a BenFrame
/// or a XBEN frame (raw bytes + variant).
#[derive(Clone)]
pub enum Frame {
    Ben(BenFrame),             // from BenFrameDecoeder
    XBen(Vec<u8>, BenVariant), // raw ben32 bytes + variant (count is carried beside)
}

pub enum Selection {
    Indices(Peekable<std::vec::IntoIter<usize>>), // 1-based, sorted
    Every { step: usize, offset: usize },         // 1-based
    Range { start: usize, end: usize },           // inclusive, 1-based
}

/// Decode a Frame (Ben or XBen) into an assignment vector.
fn decode_frame_to_assignment(frame: &Frame) -> io::Result<Vec<u16>> {
    match frame {
        Frame::Ben(f) => decode_ben_frame_to_assignment(f),
        Frame::XBen(bytes, variant) => decode_xben_frame_to_assignment(bytes, *variant),
    }
}

pub struct SubsampleFrameDecoder<I>
where
    I: Iterator<Item = io::Result<(Frame, u16)>>,
{
    inner: I,
    selection: Selection,
    sample: usize, // processed so far (1-based)
}

impl<I> SubsampleFrameDecoder<I>
where
    I: Iterator<Item = io::Result<(Frame, u16)>>,
{
    pub fn new(inner: I, selection: Selection) -> Self {
        Self {
            inner,
            selection,
            sample: 0,
        }
    }

    /// 1-based indices, in any order (duplicates removed internally).
    pub fn by_indices<T>(inner: I, indices: T) -> Self
    where
        T: IntoIterator<Item = usize>,
    {
        let mut v: Vec<usize> = indices.into_iter().collect();
        v.sort_unstable();
        v.dedup();
        Self::new(inner, Selection::Indices(v.into_iter().peekable()))
    }

    /// Inclusive 1-based range [start, end].
    pub fn by_range(inner: I, start: usize, end: usize) -> Self {
        assert!(
            start >= 1 && end >= start,
            "range must be 1-based and end >= start"
        );
        Self::new(inner, Selection::Range { start, end })
    }

    /// Every `step` samples starting from 1-based `offset`.
    pub fn every(inner: I, step: usize, offset: usize) -> Self {
        assert!(step >= 1 && offset >= 1, "step and offset must be >= 1");
        Self::new(inner, Selection::Every { step, offset })
    }

    // Helper function to count how many selected samples are in the interval [lo, hi].
    // Both lo and hi are 1-based, inclusive.
    fn count_selected_in(&mut self, lo: usize, hi: usize) -> u16 {
        match &mut self.selection {
            Selection::Indices(iter) => {
                let mut taken = 0u16;
                while let Some(&next) = iter.peek() {
                    if next < lo {
                        iter.next();
                        continue;
                    }
                    if next > hi {
                        break;
                    }
                    iter.next();
                    taken = taken.saturating_add(1);
                }
                taken
            }
            Selection::Every { step, offset } => {
                let start = lo.max(*offset);
                if start > hi {
                    return 0;
                }
                let r = (start as isize - *offset as isize).rem_euclid(*step as isize) as usize;
                let first = start + ((*step - r) % *step);
                if first > hi {
                    0
                } else {
                    (1 + (hi - first) / *step) as u16
                }
            }
            Selection::Range { start, end } => {
                if hi < *start || lo > *end {
                    0
                } else {
                    let a = lo.max(*start);
                    let b = hi.min(*end);
                    (b - a + 1) as u16
                }
            }
        }
    }
}

impl<I> Iterator for SubsampleFrameDecoder<I>
where
    I: Iterator<Item = io::Result<(Frame, u16)>>,
{
    type Item = io::Result<MkvRecord>; // (Vec<u16>, u16)

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            // early-exit for Range
            if let Selection::Range { end, .. } = self.selection {
                if self.sample >= end {
                    return None;
                }
            }
            // early-exit for Indices
            if let Selection::Indices(ref mut it) = self.selection {
                if it.peek().is_none() {
                    return None;
                }
            }

            let (frame, count) = match self.inner.next()? {
                Ok(x) => x,
                Err(e) => return Some(Err(e)),
            };

            let lo = self.sample + 1;
            let hi = self.sample + count as usize;
            let selected = self.count_selected_in(lo, hi);

            // advance regardless
            self.sample = hi;

            if selected > 0 {
                match decode_frame_to_assignment(&frame) {
                    Ok(assignment) => return Some(Ok((assignment, selected))),
                    Err(e) => return Some(Err(e)),
                }
            }
        }
    }
}

pub type FrameIter = Box<dyn Iterator<Item = io::Result<(Frame, u16)>> + Send>;

/// Build a frame iterator from a file path and mode ("ben" or "xben")
///
/// Frame iteration is useful for subsampling since you do not need to decode every frame
/// into an assignment vector. Since the BEN standard includes information about the number
/// of bytes used to encode each frame, reading through the file and extracting particular
/// frames is incredibly fast.
///
/// # Arguments
///
/// * `file_path` - A PathBuf pointing to the BEN or XBEN file.
/// * `mode` - A string slice indicating the file type: "ben" or "xben".
///
/// # Returns
///
/// An io::Result containing a boxed iterator over frames and their repetition counts.
pub fn build_frame_iter(file_path: &PathBuf, mode: &str) -> io::Result<FrameIter> {
    let file = File::options().read(true).open(file_path)?;
    let reader = BufReader::new(file);

    match mode {
        "ben" => {
            // Ben -> BenFrameDecoeder
            let frames = BenFrameDecoeder::new(reader)?; // Iterator<Item=io::Result<BenFrame>>
            let mapped = frames.map(|res| {
                res.map(|f| {
                    let cnt = f.count;
                    (Frame::Ben(f), cnt)
                })
            });
            Ok(Box::new(mapped))
        }
        "xben" => {
            // XBen -> XBenFrameDecoder (need variant)
            let x = XBenDecoder::new(reader)?;
            let variant = x.variant;
            let frames = x.into_frames(); // Iterator<Item=io::Result<(Vec<u8>, u16)>>
            let mapped =
                frames.map(move |res| res.map(|(bytes, cnt)| (Frame::XBen(bytes, variant), cnt)));
            Ok(Box::new(mapped))
        }
        _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "Unknown mode")),
    }
}

impl<R: Read + Send> BenDecoder<R> {
    /// Create a subsample iterator from this decoder that iterates over specific indices.
    /// These indices are 1-based.
    ///
    /// # Arguments
    ///
    /// * `indices` - A collection of 1-based indices to select.
    ///
    /// # Returns
    ///
    /// An io::Result containing a SubsampleFrameDecoder that yields
    /// decoded assignments and their repetition counts.
    pub fn into_subsample_by_indices<T>(
        self,
        indices: T,
    ) -> SubsampleFrameDecoder<impl Iterator<Item = io::Result<(Frame, u16)>> + Send>
    where
        T: IntoIterator<Item = usize>,
    {
        let frames = self.into_frames().map(|res| {
            res.map(|f| {
                let count = f.count;
                (Frame::Ben(f), count)
            })
        });
        SubsampleFrameDecoder::by_indices(frames, indices)
    }

    /// Create a subsample iterator from this decoder that iterates over a range of samples.
    ///
    /// # Arguments
    ///
    /// * `start` - The 1-based start index (inclusive).
    /// * `end` - The 1-based end index (inclusive).
    ///
    /// # Returns
    ///
    /// An io::Result containing a SubsampleFrameDecoder that yields
    /// decoded assignments and their repetition counts.
    pub fn into_subsample_by_range(
        self,
        start: usize,
        end: usize,
    ) -> SubsampleFrameDecoder<impl Iterator<Item = io::Result<(Frame, u16)>> + Send> {
        let frames = self.into_frames().map(|res| {
            res.map(|f| {
                let cnt = f.count;
                (Frame::Ben(f), cnt)
            })
        });
        SubsampleFrameDecoder::by_range(frames, start, end)
    }

    /// Create a subsample iterator from this decoder that iterates every `step` samples
    /// starting from 1-based `offset`.
    ///
    /// # Arguments
    ///
    /// * `step` - The step size (must be >= 1).
    /// * `offset` - The 1-based offset to start from (must be >= 1).
    ///
    /// # Returns
    ///
    /// An io::Result containing a SubsampleFrameDecoder that yields
    /// decoded assignments and their repetition counts.
    pub fn into_subsample_every(
        self,
        step: usize,
        offset: usize,
    ) -> SubsampleFrameDecoder<impl Iterator<Item = io::Result<(Frame, u16)>> + Send> {
        let frames = self.into_frames().map(|res| {
            res.map(|f| {
                let cnt = f.count;
                (Frame::Ben(f), cnt)
            })
        });
        SubsampleFrameDecoder::every(frames, step, offset)
    }
}

impl<R: Read + Send> XBenDecoder<R> {
    /// Create a subsample iterator from this decoder that iterates over specific indices.
    /// These indices are 1-based.
    ///
    /// # Arguments
    ///
    /// * `indices` - A collection of 1-based indices to select.
    ///
    /// # Returns
    ///
    /// An io::Result containing a SubsampleFrameDecoder that yields
    /// decoded assignments and their repetition counts.
    pub fn into_subsample_by_indices<T>(
        self,
        indices: T,
    ) -> SubsampleFrameDecoder<impl Iterator<Item = io::Result<(Frame, u16)>> + Send>
    where
        T: IntoIterator<Item = usize>,
    {
        let variant = self.variant; // ensure BenVariant: Copy
        let frames = self
            .into_frames()
            .map(move |res| res.map(|(bytes, cnt)| (Frame::XBen(bytes, variant), cnt)));
        SubsampleFrameDecoder::by_indices(Box::new(frames), indices)
    }

    /// Create a subsample iterator from this decoder that iterates over a range of samples.
    ///
    /// # Arguments
    ///
    /// * `start` - The 1-based start index (inclusive).
    /// * `end` - The 1-based end index (inclusive).
    ///
    /// # Returns
    ///
    /// An io::Result containing a SubsampleFrameDecoder that yields
    /// decoded assignments and their repetition counts.
    pub fn into_subsample_by_range(
        self,
        start: usize,
        end: usize,
    ) -> SubsampleFrameDecoder<impl Iterator<Item = io::Result<(Frame, u16)>> + Send> {
        let variant = self.variant;
        let frames = self
            .into_frames()
            .map(move |res| res.map(|(bytes, cnt)| (Frame::XBen(bytes, variant), cnt)));
        SubsampleFrameDecoder::by_range(Box::new(frames), start, end)
    }

    /// Create a subsample iterator from this decoder that iterates every `step` samples
    /// starting from 1-based `offset`.
    ///
    /// # Arguments
    ///
    /// * `step` - The step size (must be >= 1).
    /// * `offset` - The 1-based offset to start from (must be >= 1).
    ///
    /// # Returns
    ///
    /// An io::Result containing a SubsampleFrameDecoder that yields
    /// decoded assignments and their repetition counts.
    pub fn into_subsample_every(
        self,
        step: usize,
        offset: usize,
    ) -> SubsampleFrameDecoder<impl Iterator<Item = io::Result<(Frame, u16)>> + Send> {
        let variant = self.variant;
        let frames = self
            .into_frames()
            .map(move |res| res.map(|(bytes, cnt)| (Frame::XBen(bytes, variant), cnt)));
        SubsampleFrameDecoder::every(Box::new(frames), step, offset)
    }
}

pub fn count_samples_from_file(path: &Path, mode: &str) -> io::Result<usize> {
    let iter = build_frame_iter(&path.to_path_buf(), mode)?;
    let mut total = 0usize;
    for item in iter {
        let (_frame, cnt) = item?;
        total += cnt as usize;
    }
    Ok(total)
}

#[cfg(test)]
#[path = "tests/decode_tests.rs"]
mod tests;
