//! This module contains the main functions that are used for translating
//! between the ben32 and BEN formats. The ben32 format is a simple run-length
//! encoding of an assignment vector done at the byte level and for which every
//! 32 bits of data encodes a one (assignment, count) pair. The BEN format is
//! a bit-packed version of the ben32 format along with some extra headers.
use byteorder::{BigEndian, ReadBytesExt};
use std::io::{self, Error, Read, Write};

use super::{log, logln, BenVariant};
use crate::decode::decode_ben_line;
use crate::encode::encode_ben_vec_from_rle;

/// This function takes a ben32 encoded assignment vector and
/// transforms into a ben encoded assignment vector.
///
/// # Arguments
///
/// * `ben32_vec` - A vector of bytes containing the ben32 encoded assignment vector
///
/// # Returns
///
/// A vector of bytes containing the ben encoded assignment vector
///
/// # Errors
///
/// This function will return an error if the input ben32 vector is not a multiple of 4
/// bytes long or if the end of line separator (4 bytes of 0) is missing. All
/// assignment vectors are expected to be a multiple of 4 bytes long since each
/// assignment vector is an run-length encoded as a 32 bit integer (2 bytes for
/// the value and 2 bytes for the count). The end of line separator is also the
/// only way that the ben32 format has to separate assignment vectors.
fn ben32_to_ben_line(ben32_vec: Vec<u8>) -> io::Result<Vec<u8>> {
    let mut buffer = [0u8; 4];
    let mut ben32_rle: Vec<(u16, u16)> = Vec::new();

    let mut reader = ben32_vec.as_slice();

    if ben32_vec.len() % 4 != 0 {
        return Err(Error::new(
            io::ErrorKind::InvalidData,
            "Invalid ben32 data length",
        ));
    }

    for _ in 0..((ben32_vec.len() / 4) - 1) {
        reader.read_exact(&mut buffer)?;
        let encoded = u32::from_be_bytes(buffer);

        let value = (encoded >> 16) as u16; // High 16 bits
        let count = (encoded & 0xFFFF) as u16; // Low 16 bits

        ben32_rle.push((value, count));
    }

    // read the last 4 bytes which should be 0 since they are a separator
    reader.read_exact(&mut buffer)?;
    if buffer != [0u8; 4] {
        return Err(Error::new(
            io::ErrorKind::InvalidData,
            "Invalid ben32 data format. Missing end of line separator.",
        ));
    }

    Ok(encode_ben_vec_from_rle(ben32_rle))
}

/// This function takes a reader that contains a several ben32 encoded assignment
/// vectors and encodes them into ben encoded assignment vectors and writes them
/// to the designated writer.
///
/// # Arguments
///
/// * `reader` - A reader that contains ben32 encoded assignment vectors
/// * `writer` - A writer that will contain the ben encoded assignment vectors
///
/// # Returns
///
/// An io::Result containing the result of the operation
///
/// # Errors
///
/// This function will return an error if the input reader contains invalid ben32
/// data or if the writer encounters an error while writing the ben data.
pub fn ben32_to_ben_lines<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    variant: BenVariant,
) -> io::Result<()> {
    'outer: loop {
        let mut ben32_vec: Vec<u8> = Vec::new();
        let mut ben32_read_buff: [u8; 4] = [0u8; 4];

        let mut n_reps = 0;

        // extract the ben32 data
        'inner: loop {
            match reader.read_exact(&mut ben32_read_buff) {
                Ok(()) => {
                    ben32_vec.extend(ben32_read_buff);
                    if ben32_read_buff == [0u8; 4] {
                        if variant == BenVariant::MkvChain {
                            n_reps = reader.read_u16::<BigEndian>()?;
                        }
                        break 'inner;
                    }
                }
                Err(e) => {
                    if e.kind() == io::ErrorKind::UnexpectedEof {
                        break 'outer;
                    }
                    return Err(e);
                }
            }
        }

        let ben_vec = ben32_to_ben_line(ben32_vec)?;
        writer.write_all(&ben_vec)?;
        if variant == BenVariant::MkvChain {
            writer.write_all(&n_reps.to_be_bytes())?;
        }
    }

    Ok(())
}

/// This function takes a ben encoded assignment vector and transforms it into
/// a ben32 encoded assignment vector.
///
/// # Arguments
///
/// * `reader` - A reader that contains ben encoded assignment vectors
/// * `max_val_bits` - The maximum number of bits that the value of an assignment can have
/// * `max_len_bits` - The maximum number of bits that the length of an assignment can have
///
/// # Returns
///
/// A vector of bytes containing the ben32 encoded assignment vector
fn ben_to_ben32_line<R: Read>(
    reader: R,
    max_val_bits: u8,
    max_len_bits: u8,
    n_bytes: u32,
) -> io::Result<Vec<u8>> {
    let ben_rle: Vec<(u16, u16)> = decode_ben_line(reader, max_val_bits, max_len_bits, n_bytes)?;

    let mut ben32_vec: Vec<u8> = Vec::new();

    for (value, count) in ben_rle.into_iter() {
        let encoded = ((value as u32) << 16) | (count as u32);
        ben32_vec.extend(&encoded.to_be_bytes());
    }

    ben32_vec.extend(&[0u8; 4]);

    Ok(ben32_vec)
}

/// This function takes a reader that contains a several ben encoded assignment
/// vectors and encodes them into ben32 encoded assignment vectors and writes them
/// to the designated writer.
///
/// # Arguments
///
/// * `reader` - A reader that contains ben encoded assignment vectors
/// * `writer` - A writer that will contain the ben32 encoded assignment vectors
///
/// # Returns
///
/// An io::Result containing the result of the operation
///
/// # Errors
///
/// This function will return an error if the input reader contains invalid ben
/// data or if the writer encounters an error while writing the ben32 data.
pub fn ben_to_ben32_lines<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    variant: BenVariant,
) -> io::Result<()> {
    let mut sample_number = 1;
    'outer: loop {
        let mut tmp_buffer = [0u8];
        let max_val_bits = match reader.read_exact(&mut tmp_buffer) {
            Ok(()) => tmp_buffer[0],
            Err(e) => {
                if e.kind() == io::ErrorKind::UnexpectedEof {
                    break 'outer;
                }
                return Err(e);
            }
        };

        let max_len_bits = reader.read_u8()?;
        let n_bytes = reader.read_u32::<BigEndian>()?;

        log!("Encoding line: {}\r", sample_number);

        match variant {
            BenVariant::Standard => {
                sample_number += 1;
                let ben32_vec =
                    ben_to_ben32_line(&mut reader, max_val_bits, max_len_bits, n_bytes)?;
                writer.write_all(&ben32_vec)?;
            }
            BenVariant::MkvChain => {
                let ben32_vec =
                    ben_to_ben32_line(&mut reader, max_val_bits, max_len_bits, n_bytes)?;

                // Read the number of repetitions AFTER the ben32 data
                let n_reps = reader.read_u16::<BigEndian>()?;
                sample_number += n_reps as usize;
                writer.write_all(&ben32_vec)?;
                writer.write_all(&n_reps.to_be_bytes())?;
            }
        }
    }

    logln!();
    logln!("Done!");
    Ok(())
}

#[cfg(test)]
#[path = "tests/translate_tests.rs"]
mod tests;
