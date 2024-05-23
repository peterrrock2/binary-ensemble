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
use std::io::{self, BufRead, Error, Read, Write};

use crate::utils::rle_to_vec;

use super::encode::translate::*;
use super::{log, logln};

// Note: This will make Read easier to use since
// I can now implement the read chunk with a Cursor
// object.
pub struct BenDecoder<R: Read> {
    reader: R,
    sample_count: usize,
}

impl<R: Read> BenDecoder<R> {
    pub fn new(mut reader: R) -> Self {
        let mut check_buffer = [0u8; 17];

        match reader.read_exact(&mut check_buffer) {
            Ok(_) => {
                if &check_buffer != b"STANDARD BEN FILE" {
                    panic!("Invalid file format");
                }
            }
            Err(e) => {
                panic!("Error reading file: {}", e);
            }
        }

        BenDecoder {
            reader,
            sample_count: 0,
        }
    }

    pub fn write_all_jsonl(&mut self, mut writer: impl Write) -> io::Result<()> {
        while let Some(result) = self.next() {
            match result {
                Ok(assignment) => {
                    let line = json!({
                        "assignment": assignment,
                        "sample": self.sample_count,
                    })
                    .to_string()
                        + "\n";
                    writer.write_all(line.as_bytes()).unwrap();
                }
                Err(e) => {
                    return Err(e);
                }
            }
        }
        Ok(())
    }
}

impl<R: Read> Iterator for BenDecoder<R> {
    type Item = io::Result<Vec<u16>>;

    fn next(&mut self) -> Option<io::Result<Vec<u16>>> {
        let mut tmp_buffer = [0u8];
        let max_val_bits: u8 = match self.reader.read_exact(&mut tmp_buffer) {
            Ok(()) => tmp_buffer[0],
            Err(e) => {
                if e.kind() == io::ErrorKind::UnexpectedEof {
                    logln!();
                    logln!("Done!");
                    return None;
                }
                return Some(Err(e));
            }
        };

        self.sample_count += 1;
        log!("Decoding sample: {}\r", self.sample_count);
        let max_len_bits = self
            .reader
            .read_u8()
            .expect(format!("Error when reading sample {}.", self.sample_count).as_str());
        let n_bytes = self
            .reader
            .read_u32::<BigEndian>()
            .expect(format!("Error when reading sample {}.", self.sample_count).as_str());

        match decode_ben_line(&mut self.reader, max_val_bits, max_len_bits, n_bytes) {
            Ok(output_rle) => Some(Ok(rle_to_vec(output_rle))),
            Err(e) => Some(Err(e)),
        }
    }
}

/// This function takes a reader containing a single ben32 encoded assignment
/// vector and decodes it into a full assignment vector of u16s.
///
/// # Arguments
///
/// * `reader` - A reader containing the ben32 encoded assignment vector
///
/// # Returns
///
/// A vector of u16s containing the decoded assignment vector
///
/// # Errors
///
/// This function will return an error if the input reader is not a multiple of 4
/// bytes long since each assignment vector is an run-length encoded as a 32 bit
/// integer (2 bytes for the value and 2 bytes for the count).
///
fn decode_ben32_line<R: BufRead>(mut reader: R) -> io::Result<Vec<u16>> {
    let mut buffer = [0u8; 4];
    let mut output_vec: Vec<u16> = Vec::new();

    loop {
        // Read 4 bytes (u32) from the encoded file
        // https://stackoverflow.com/questions/30412521/how-to-read-a-specific-number-of-bytes-from-a-stream
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
    Ok(output_vec)
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
/// # Arguments
///
/// * `reader` - A reader containing the ben32 encoded assignment vectors
/// * `writer` - A writer that will contain the JSONL formatted assignment vectors
///
/// # Returns
///
/// An io::Result containing the result of the operation
///
/// # Errors
///
/// This function will return an error if the input reader contains invalid ben32
/// data or if the the decode method encounters while trying to extract a single
/// assignment vector, that error is propagated.
fn jsonl_decode_ben32<R: BufRead, W: Write>(mut reader: R, mut writer: W) -> io::Result<()> {
    let mut sample_number = 1;
    let mut check_buffer = [0u8; 17];
    reader.read_exact(&mut check_buffer)?;

    if &check_buffer != b"STANDARD BEN FILE" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid file format",
        ));
    }

    loop {
        let output_vec = decode_ben32_line(&mut reader);
        if let Err(e) = output_vec {
            if e.kind() == io::ErrorKind::UnexpectedEof {
                return Ok(());
            }
            return Err(e);
        }

        // Write the reconstructed vector as JSON to the output file
        let line = json!({
            "assignment": output_vec.unwrap(),
            "sample": sample_number,
        })
        .to_string()
            + "\n";

        writer.write_all(line.as_bytes())?;
        sample_number += 1;
    }
}

/// This function takes a reader containing a file encoded in the XBEN format
/// and decodes it into a BEN file.
///
/// # Arguments
///
/// * `reader` - A reader containing the xben encoded assignment vectors
/// * `writer` - A writer that will contain the BEN formatted assignment vectors
///
/// # Returns
///
/// An io::Result containing the result of the operation
///
/// # Errors
///
/// This function will return an error if the input reader contains invalid xben
/// data or if the the decode method encounters while trying to convert the
/// xben data to ben data.
pub fn decode_xben_to_ben<R: BufRead, W: Write>(reader: R, mut writer: W) -> io::Result<()> {
    let mut decoder = xz2::read::XzDecoder::new(reader);

    let mut first_buffer = [0u8; 17];

    match decoder.read(&mut first_buffer) {
        Ok(_) => {
            if &first_buffer[..17] != b"STANDARD BEN FILE" {
                return Err(Error::new(
                    io::ErrorKind::InvalidData,
                    "Invalid file format",
                ));
            }
            writer.write_all(b"STANDARD BEN FILE")?;
        }
        Err(e) => {
            return Err(e);
        }
    }

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
        // few seconds). Reading form the front is both safer from a
        // maintenance perspective and allows for a better progress indicator
        for i in (3..overflow.len()).step_by(4) {
            if overflow[i - 3..=i] == [0, 0, 0, 0] {
                last_valid_assignment = i + 1;
                line_count += 1;
                log!("Decoding sample: {}\r", line_count);
            }
        }

        if last_valid_assignment == 0 {
            continue;
        }

        ben32_to_ben_lines(&overflow[0..last_valid_assignment], &mut writer)?;
        overflow = overflow[last_valid_assignment..].to_vec();
    }
    logln!();
    logln!("Done!");
    Ok(())
}

/// This is a convenience function that decodes a general level 9 LZMA2 compressed file.
///
/// # Arguments
///
/// * `reader` - A reader containing the LZMA2 compressed data
/// * `writer` - A writer that will contain the decompressed data
///
/// # Returns
///
/// An io::Result containing the result of the operation
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
/// xz_compress(reader, writer).unwrap();
///
/// let mut recovery_buff = Vec::new();
/// let recovery_reader = BufWriter::new(&mut recovery_buff);
/// xz_decompress(output_buffer.as_slice(), recovery_reader).unwrap();
/// println!("{:?}", output_buffer);
/// ```
pub fn xz_decompress<R: BufRead, W: Write>(reader: R, mut writer: W) -> io::Result<()> {
    let mut decoder = xz2::read::XzDecoder::new(reader);
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
///
/// # Arguments
///
/// * `reader` - A reader containing the ben encoded assignment vectors
/// * `max_val_bits` - The maximum number of bits used to encode the value
/// * `max_len_bits` - The maximum number of bits used to encode the length
/// * `n_bytes` - The number of bytes used to encode the assignment vector
///
/// # Returns
///
/// A vector of tuples containing the run-length encoded assignment vector
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
/// # Arguments
///
/// * `reader` - A reader containing the ben encoded assignment vectors
/// * `writer` - A writer that will contain the JSONL formatted assignment vectors
///
/// # Returns
///
/// An io::Result containing the result of the operation
///
/// # Errors
///
/// This function will return an error if the input reader contains invalid ben
/// data or if the the decode method encounters while trying to extract a single
/// assignment vector, that error is then propagated.
pub fn jsonl_decode_ben<R: Read, W: Write>(reader: R, writer: W) -> io::Result<()> {
    let mut ben_decoder = BenDecoder::new(reader);
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
/// # Arguments
///
/// * `reader` - A reader containing the xben encoded assignment vectors
/// * `writer` - A writer that will contain the JSONL formatted assignment vectors
///
/// # Returns
///
/// An io::Result containing the result of the operation
///
/// # Errors
///
/// This function will return an error if the input reader contains invalid xben
/// data or if the the decode method encounters while trying to extract a single
/// assignment vector, that error is then propagated.
pub fn jsonl_decode_xben<R: BufRead, W: Write>(reader: R, mut writer: W) -> io::Result<()> {
    let mut decoder = xz2::read::XzDecoder::new(reader);

    let mut first_buffer = [0u8; 17];

    match decoder.read(&mut first_buffer) {
        Ok(_) => {
            if &first_buffer[..17] != b"STANDARD BEN FILE" {
                return Err(Error::new(
                    io::ErrorKind::InvalidData,
                    "Invalid file format",
                ));
            }
        }
        Err(e) => {
            return Err(e);
        }
    }

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
        // few seconds). Reading form the front is both safer from a
        // maintenance perspective and allows for a better progress indicator
        for i in (3..overflow.len()).step_by(4) {
            if overflow[i - 3..=i] == [0, 0, 0, 0] {
                last_valid_assignment = i + 1;
                line_count += 1;
                log!("Decoding sample: {}\r", line_count);
            }
        }

        if last_valid_assignment == 0 {
            continue;
        }

        let mut new_vec: Vec<u8> = b"STANDARD BEN FILE".to_vec();
        new_vec.extend(&overflow[0..last_valid_assignment]);
        jsonl_decode_ben32(&new_vec[..], &mut writer)?;
        overflow = overflow[last_valid_assignment..].to_vec();
    }
    logln!();
    logln!("Done!");
    Ok(())
}

#[cfg(test)]
mod tests {
    include!("tests/decode_tests.rs");
}
