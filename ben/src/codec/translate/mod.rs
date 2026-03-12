//! Translation helpers between BEN and ben32 representations.

use byteorder::{BigEndian, ReadBytesExt};
use std::io::{self, Error, Read, Write};

use crate::codec::decode::decode_ben_line;
use crate::codec::encode::encode_ben_vec_from_rle;
use crate::{log, logln, BenVariant};

fn ben32_to_ben_line(ben32_vec: Vec<u8>) -> io::Result<Vec<u8>> {
    let mut buffer = [0u8; 4];
    let mut ben32_rle: Vec<(u16, u16)> = Vec::new();

    let mut reader = ben32_vec.as_slice();

    if !ben32_vec.len().is_multiple_of(4) {
        return Err(Error::new(
            io::ErrorKind::InvalidData,
            "Invalid ben32 data length",
        ));
    }

    for _ in 0..((ben32_vec.len() / 4) - 1) {
        reader.read_exact(&mut buffer)?;
        let encoded = u32::from_be_bytes(buffer);

        let value = (encoded >> 16) as u16;
        let count = (encoded & 0xFFFF) as u16;

        ben32_rle.push((value, count));
    }

    reader.read_exact(&mut buffer)?;
    if buffer != [0u8; 4] {
        return Err(Error::new(
            io::ErrorKind::InvalidData,
            "Invalid ben32 data format. Missing end of line separator.",
        ));
    }

    Ok(encode_ben_vec_from_rle(ben32_rle))
}

pub fn ben32_to_ben_lines<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    variant: BenVariant,
) -> io::Result<()> {
    'outer: loop {
        let mut ben32_vec: Vec<u8> = Vec::new();
        let mut ben32_read_buff: [u8; 4] = [0u8; 4];

        let mut n_reps = 0;

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

fn ben_to_ben32_line<R: Read>(
    reader: R,
    max_val_bits: u8,
    max_len_bits: u8,
    n_bytes: u32,
) -> io::Result<Vec<u8>> {
    let ben_rle: Vec<(u16, u16)> = decode_ben_line(reader, max_val_bits, max_len_bits, n_bytes)?;

    let mut ben32_vec: Vec<u8> = Vec::new();

    for (value, count) in ben_rle {
        let encoded = ((value as u32) << 16) | (count as u32);
        ben32_vec.extend(&encoded.to_be_bytes());
    }

    ben32_vec.extend(&[0u8; 4]);

    Ok(ben32_vec)
}

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
mod tests;
