//! This module contains the main encoding functions for turning an
//! input JSONL or BEN file into a BEN or XBEN file.
//!
//! Any input JSONL file is expected to be in the standard
//!
//! ```json
//! {"assignment": [...], "sample": #}
//! ```
//!
//! format.
//!
//! The BEN format is
//! a simple bit-packed run-length encoded assignment vector with
//! some special headers that allow the decoder to know how many
//! bytes to read for each sample.
//!
//!
//! The XBEN format uses LZMA2 dictionary compression on
//! a byte-level decompressed version of the BEN format (known as ben32)
//! to achieve better compression ratios than we could achieve with applying
//! LZMA2 compression directly to the BEN format.

pub mod relabel;
pub mod translate;

use crate::utils::*;
use serde_json::Value;
use std::io::{self, BufRead, Cursor, Read, Result, Write};
use xz2::write::XzEncoder;

use self::translate::ben_to_ben32_lines;
use super::{log, logln, BenVariant};

/// A struct to make the writing of BEN files easier
/// and more ergonomic.
///
/// # Example
///
/// ```
/// use ben::{encode::BenEncoder, BenVariant};
///
/// let mut buffer = Vec::new();
/// let mut ben_encoder = BenEncoder::new(&mut buffer, BenVariant::Standard);
///
/// ben_encoder.write_assignment(vec![1, 1, 1, 2, 2, 2]);
/// ```
// pub struct BenEncoder<W: Write> {
//     writer: W,
// }

// impl<W: Write> BenEncoder<W> {
//     /// Create a new BenEncoder instance and handles
//     /// the BEN file header.
//     pub fn new(mut writer: W) -> Self {
//         writer.write_all(b"STANDARD BEN FILE").unwrap();
//         BenEncoder { writer }
//     }

//     /// Write a run-length encoded assignment vector to the
//     /// BEN file.
//     pub fn write_rle(&mut self, rle_vec: Vec<(u16, u16)>) -> Result<()> {
//         let encoded = encode_ben_vec_from_rle(rle_vec);
//         self.writer.write_all(&encoded)?;
//         Ok(())
//     }

//     /// Write an assignment vector to the BEN file.
//     pub fn write_assignment(&mut self, assign_vec: Vec<u16>) -> Result<()> {
//         let rle_vec = assign_to_rle(assign_vec);
//         self.write_rle(rle_vec)?;
//         Ok(())
//     }

//     /// Write a JSON value containing an assignment vector to the BEN file.
//     pub fn write_json_value(&mut self, data: Value) -> Result<()> {
//         let assign_vec = data["assignment"].as_array().unwrap();
//         let rle_vec = assign_to_rle(
//             assign_vec
//                 .into_iter()
//                 .map(|x| x.as_u64().unwrap() as u16)
//                 .collect(),
//         );
//         self.write_rle(rle_vec)?;
//         Ok(())
//     }
// }

pub struct BenEncoder<W: Write> {
    writer: W,
    previous_sample: Vec<u8>,
    count: u16,
    variant: BenVariant,
}

impl<W: Write> BenEncoder<W> {
    /// Create a new BenEncoder instance and handles
    /// the BEN file header.
    pub fn new(mut writer: W, variant: BenVariant) -> Self {
        match variant {
            BenVariant::Standard => {
                writer.write_all(b"STANDARD BEN FILE").unwrap();
            }
            BenVariant::MkvChain => {
                writer.write_all(b"MKVCHAIN BEN FILE").unwrap();
            }
        }
        BenEncoder {
            writer,
            previous_sample: Vec::new(),
            count: 0,
            variant,
        }
    }

    /// Write a run-length encoded assignment vector to the
    /// BEN file.
    pub fn write_rle(&mut self, rle_vec: Vec<(u16, u16)>) -> Result<()> {
        match self.variant {
            BenVariant::Standard => {
                let encoded = encode_ben_vec_from_rle(rle_vec);
                self.writer.write_all(&encoded)?;
                Ok(())
            }
            BenVariant::MkvChain => {
                let encoded = encode_ben_vec_from_rle(rle_vec);
                if encoded == self.previous_sample {
                    self.count += 1;
                } else {
                    if self.count > 0 {
                        self.writer.write_all(&self.previous_sample)?;
                        self.writer.write_all(&self.count.to_be_bytes())?;
                    }
                    self.previous_sample = encoded;
                    self.count = 1;
                }
                Ok(())
            }
        }
    }

    /// Write an assignment vector to the BEN file.
    pub fn write_assignment(&mut self, assign_vec: Vec<u16>) -> Result<()> {
        let rle_vec = assign_to_rle(assign_vec);
        self.write_rle(rle_vec)?;
        Ok(())
    }

    /// Write a JSON value containing an assignment vector to the BEN file.
    pub fn write_json_value(&mut self, data: Value) -> Result<()> {
        let assign_vec = data["assignment"].as_array().unwrap();
        let rle_vec = assign_to_rle(
            assign_vec
                .into_iter()
                .map(|x| x.as_u64().unwrap() as u16)
                .collect(),
        );
        self.write_rle(rle_vec)?;
        Ok(())
    }
}

impl<W: Write> Drop for BenEncoder<W> {
    fn drop(&mut self) {
        if self.variant == BenVariant::MkvChain && self.count > 0 {
            self.writer
                .write_all(&self.previous_sample)
                .expect("Error writing last line to file");
            self.writer
                .write_all(&self.count.to_be_bytes())
                .expect("Error writing last line count to file");
        }
    }
}

/// A struct to make the writing of XBEN files easier
/// and more ergonomic.
pub struct XBenEncoder<W: Write> {
    encoder: XzEncoder<W>,
    previous_sample: Vec<u8>,
    count: u16,
    variant: BenVariant,
}

impl<W: Write> XBenEncoder<W> {
    pub fn new(mut encoder: XzEncoder<W>, variant: BenVariant) -> Self {
        match variant {
            BenVariant::Standard => {
                encoder.write_all(b"STANDARD BEN FILE").unwrap();
                XBenEncoder {
                    encoder,
                    previous_sample: Vec::new(),
                    count: 0,
                    variant: BenVariant::Standard,
                }
            }
            BenVariant::MkvChain => {
                encoder.write_all(b"MKVCHAIN BEN FILE").unwrap();
                XBenEncoder {
                    encoder,
                    previous_sample: Vec::new(),
                    count: 0,
                    variant: BenVariant::MkvChain,
                }
            }
        }
    }

    /// Write a an assigment vector encoded as a JSON value
    /// to the XBEN file.
    pub fn write_json_value(&mut self, data: Value) -> Result<()> {
        let encoded = encode_ben32_line(data);
        match self.variant {
            BenVariant::Standard => {
                self.encoder.write_all(&encoded)?;
            }
            BenVariant::MkvChain => {
                if encoded == self.previous_sample {
                    self.count += 1;
                } else {
                    if self.count > 0 {
                        self.encoder.write_all(&self.previous_sample)?;
                        self.encoder.write_all(&self.count.to_be_bytes())?;
                    }
                    self.previous_sample = encoded;
                    self.count = 1;
                }
            }
        }
        Ok(())
    }

    /// Converts a raw BEN assignment file into to an XBEN file.
    /// This function will check to see if the header is there and then
    /// handle it accordingly.
    pub fn write_ben_file(&mut self, mut reader: impl BufRead) -> Result<()> {
        let mut buff = [0u8; 17];
        reader.read_exact(&mut buff)?;

        // Create a new reader that prepends buff back onto the original reader
        let mut reader =
            if buff != b"STANDARD BEN FILE".as_slice() || buff != b"MKVCHAIN BEN FILE".as_slice() {
                let cursor = Cursor::new(buff.to_vec());
                let reader = cursor.chain(reader);
                Box::new(reader) as Box<dyn BufRead>
            } else {
                Box::new(reader)
            };

        ben_to_ben32_lines(&mut *reader, &mut self.encoder, self.variant)
    }
}

impl<W: Write> Drop for XBenEncoder<W> {
    fn drop(&mut self) {
        if self.variant == BenVariant::MkvChain && self.count > 0 {
            self.encoder
                .write_all(&self.previous_sample)
                .expect("Error writing last line to file");
            self.encoder
                .write_all(&self.count.to_be_bytes())
                .expect("Error writing last line count to file");
        }
    }
}

/// This function takes a json encoded line containing an assignment
/// vector and a sample number and encodes the assignment vector
/// into a binary format known as "ben32". The ben32 format serves
/// as an intermediate format that allows for efficient compression
/// of BEN files using LZMA2 compression methods.
///
/// # Arguments
///
/// * `data` - A JSON object containing an assignment vector and a sample number
///
/// # Returns
///
/// A vector of bytes containing the ben32 encoded assignment vector
fn encode_ben32_line(data: Value) -> Vec<u8> {
    let assign_vec = data["assignment"].as_array().unwrap();
    let mut prev_assign: u16 = 0;
    let mut count: u16 = 0;
    let mut first = true;

    let mut ret = Vec::new();

    for assignment in assign_vec {
        let assign = assignment.as_u64().unwrap() as u16;
        if first {
            prev_assign = assign;
            count = 1;
            first = false;
            continue;
        }
        if assign == prev_assign {
            count += 1;
        } else {
            let encoded = (prev_assign as u32) << 16 | count as u32;
            ret.extend(&encoded.to_be_bytes());
            // Reset for next run
            prev_assign = assign;
            count = 1;
        }
    }

    // Handle the last run
    if count > 0 {
        let encoded = (prev_assign as u32) << 16 | count as u32;
        ret.extend(&encoded.to_be_bytes());
    }

    ret.extend([0, 0, 0, 0]);
    ret
}

/// This function takes a JSONL file and compresses it to the
/// XBEN format.
///
/// The JSONL file is assumed to be formatted in the standard
///
/// ```json
/// {"assignment": [...], "sample": #}
/// ```
///
/// format. While the BEN format is
/// a simple bit-packed (streamable!) run-length encoded assignment
/// vector, the XBEN format uses LZMA2 dictionary compression on
/// the byte level to achieve better compression ratios. In order
/// to use XBEN files, the `decode_xben_to_ben` function must be
/// used to decode the file back into a BEN format.
pub fn jsonl_encode_xben<R: BufRead, W: Write>(
    reader: R,
    writer: W,
    variant: BenVariant,
) -> Result<()> {
    let encoder = XzEncoder::new(writer, 9);
    let mut ben_encoder = XBenEncoder::new(encoder, variant);

    let mut line_num = 1;

    for line_result in reader.lines() {
        log!("Encoding line: {}\r", line_num);
        line_num += 1;
        let line = line_result?;
        let data: Value = serde_json::from_str(&line).expect("Error parsing JSON from line");

        ben_encoder.write_json_value(data)?;
    }

    logln!();
    logln!("Done!");

    Ok(())
}

/// This is a convenience function that applies level 9 LZMA2 compression
/// to a general file.
///
/// # Arguments
///
/// * `reader` - A buffered reader for the input file
/// * `writer` - A writer for the output file
///
/// # Returns
///
/// A Result type that contains the result of the operation
///
/// ```
/// use ben::encode::xz_compress;
/// use lipsum::lipsum;
/// use std::io::{BufReader, BufWriter};
///
/// let input = lipsum(100);
/// let reader = BufReader::new(input.as_bytes());
///
/// let mut output_buffer = Vec::new();
/// let writer = BufWriter::new(&mut output_buffer);
///
/// xz_compress(reader, writer).unwrap();
///
/// println!("{:?}", output_buffer);
/// ```
pub fn xz_compress<R: BufRead, W: Write>(mut reader: R, writer: W) -> Result<()> {
    let mut buff = [0; 4096];
    let mut encoder = XzEncoder::new(writer, 9);

    while let Ok(count) = reader.read(&mut buff) {
        if count == 0 {
            break;
        }
        encoder.write_all(&buff[..count])?;
    }
    drop(encoder); // Make sure to flush and finish compression
    Ok(())
}

/// This function takes in a standard assignment vector and encodes
/// it into a bit-packed ben version.
///
/// # Arguments
///
/// * `assign_vec` - A vector of u16 values representing the assignment vector
///
/// # Returns
///
/// A vector of bytes containing the bit-packed ben encoded assignment vector
pub fn encode_ben_vec_from_assign(assign_vec: Vec<u16>) -> Vec<u8> {
    let rle_vec: Vec<(u16, u16)> = assign_to_rle(assign_vec);
    encode_ben_vec_from_rle(rle_vec)
}

/// This function takes a run-length encoded assignment vector and
/// encodes into a bit-packed ben version
///
/// # Arguments
///
/// * `rle_vec` - A vector of tuples containing the value and length of each run
///
/// # Returns
///
/// A vector of bytes containing the bit-packed ben encoded assignment vector
fn encode_ben_vec_from_rle(rle_vec: Vec<(u16, u16)>) -> Vec<u8> {
    let mut output_vec: Vec<u8> = Vec::new();

    let max_val: u16 = rle_vec.iter().max_by_key(|x| x.0).unwrap().0;
    let max_len: u16 = rle_vec.iter().max_by_key(|x| x.1).unwrap().1;
    let max_val_bits: u8 = (16 - max_val.leading_zeros() as u8).max(1);
    let max_len_bits: u8 = 16 - max_len.leading_zeros() as u8;
    let assign_bits: u32 = (max_val_bits + max_len_bits) as u32;
    let n_bytes: u32 = if (assign_bits * rle_vec.len() as u32) % 8 == 0 {
        (assign_bits * rle_vec.len() as u32) / 8
    } else {
        (assign_bits * rle_vec.len() as u32) / 8 + 1
    };

    output_vec.push(max_val_bits);
    output_vec.push(max_len_bits);
    output_vec.extend(n_bytes.to_be_bytes().as_slice());

    let mut remainder: u32 = 0;
    let mut remainder_bits: u8 = 0;

    for (val, len) in rle_vec {
        let mut new_val: u32 = (remainder << max_val_bits) | (val as u32);

        let mut buff: u8;

        let mut n_bits_left: u8 = remainder_bits + max_val_bits;

        while n_bits_left >= 8 {
            n_bits_left -= 8;
            buff = (new_val >> n_bits_left) as u8;
            output_vec.push(buff);
            new_val = new_val & (!((0xFFFFFFFF as u32) << n_bits_left));
        }

        new_val = (new_val << max_len_bits) | (len as u32);
        n_bits_left += max_len_bits;

        while n_bits_left >= 8 {
            n_bits_left -= 8;
            buff = (new_val >> n_bits_left) as u8;
            output_vec.push(buff);
            new_val = new_val & (!((0xFFFFFFFF as u32) << n_bits_left));
        }

        remainder_bits = n_bits_left;
        remainder = new_val;
    }

    if remainder_bits > 0 {
        let buff = (remainder << (8 - remainder_bits)) as u8;
        output_vec.push(buff);
    }

    output_vec
}

/// This function takes a JSONL file and compresses it into
/// the BEN format.
///
/// The JSONL file is assumed to be formatted in the standard
///
/// ```json
/// {"assignment": [...], "sample": #}
/// ```
///
/// format.
///
/// # Arguments
///
/// * `reader` - A buffered reader for the input file
/// * `writer` - A writer for the output file
///
/// # Returns
///
/// A Result type that contains the result of the operation
///
/// # Example
///
/// ```
/// use std::io::{BufReader, BufWriter};
/// use serde_json::json;
/// use ben::{encode::jsonl_encode_ben, BenVariant};
///
/// let input = r#"{"assignment": [1,1,1,2,2,2], "sample": 1}"#.to_string()
///     + "\n"
///     + r#"{"assignment": [1,1,2,2,1,2], "sample": 2}"#;
///
/// let reader = BufReader::new(input.as_bytes());
/// let mut write_buffer = Vec::new();
/// let mut writer = BufWriter::new(&mut write_buffer);
///
/// jsonl_encode_ben(reader, writer, BenVariant::Standard).unwrap();
///
/// println!("{:?}", write_buffer);
/// // This will output
/// // [83, 84, 65, 78, 68, 65, 82, 68, 32,
/// //  66, 69, 78, 32, 70, 73, 76, 69, 2,
/// //  2, 0, 0, 0, 1, 123, 2, 2, 0, 0, 0,
/// //  2, 106, 89]
/// ```
///
pub fn jsonl_encode_ben<R: BufRead, W: Write>(
    reader: R,
    writer: W,
    variant: BenVariant,
) -> Result<()> {
    let mut line_num = 1;
    let mut ben_encoder = BenEncoder::new(writer, variant);
    for line_result in reader.lines() {
        log!("Encoding line: {}\r", line_num);
        line_num += 1;
        let line = line_result?; // Handle potential I/O errors for each line
        let data: Value = serde_json::from_str(&line).expect("Error parsing JSON from line");

        ben_encoder.write_json_value(data)?;
    }
    logln!();
    logln!("Done!"); // Print newline after progress bar
    Ok(())
}

/// This function takes a BEN file and encodes it into an XBEN
/// file using bit-to-byte decompression followed by LZMA2 compression.
///
/// # Arguments
///
/// * `reader` - A buffered reader for the input file
/// * `writer` - A writer for the output file
///
/// # Returns
///
/// A Result type that contains the result of the operation
pub fn ben_encode_xben<R: BufRead, W: Write>(mut reader: R, writer: W) -> Result<()> {
    let mut check_buffer = [0u8; 17];
    reader.read_exact(&mut check_buffer)?;

    let encoder = XzEncoder::new(writer, 9);

    let mut ben_encoder = match &check_buffer {
        b"STANDARD BEN FILE" => XBenEncoder::new(encoder, BenVariant::Standard),
        b"MKVCHAIN BEN FILE" => XBenEncoder::new(encoder, BenVariant::MkvChain),
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid file format",
            ));
        }
    };

    ben_encoder.write_ben_file(reader)?;

    Ok(())
}

#[cfg(test)]
#[path = "tests/encode_tests.rs"]
mod tests;
