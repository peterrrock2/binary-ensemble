//! Relabeling operations for BEN files.

use crate::codec::decode::decode_ben_line;
use crate::codec::encode::encode_ben_vec_from_rle;
use crate::util::rle::{assign_to_rle, rle_to_vec};
use crate::{progress, BenVariant};
use byteorder::{BigEndian, ReadBytesExt};
use std::collections::HashMap;
use std::io::{self, Error, Read, Write};

pub fn relabel_ben_lines<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    variant: BenVariant,
) -> io::Result<()> {
    let mut sample_number = 0;
    loop {
        let mut tmp_buffer = [0u8];
        let max_val_bits = match reader.read_exact(&mut tmp_buffer) {
            Ok(_) => tmp_buffer[0],
            Err(e) => {
                if e.kind() == io::ErrorKind::UnexpectedEof {
                    break;
                }
                return Err(e);
            }
        };

        let max_len_bits = reader.read_u8()?;
        let n_bytes = reader.read_u32::<BigEndian>()?;

        let mut ben_line = decode_ben_line(&mut reader, max_val_bits, max_len_bits, n_bytes)?;

        let mut label = 0;
        let mut label_map = HashMap::new();
        for (val, _len) in &mut ben_line {
            let new_val = match label_map.get(val) {
                Some(v) => *v,
                None => {
                    label += 1;
                    label_map.insert(*val, label);
                    label
                }
            };
            *val = new_val;
        }

        let relabeled = encode_ben_vec_from_rle(ben_line);
        writer.write_all(&relabeled)?;

        let count_occurrences = if variant == BenVariant::MkvChain {
            let count = reader.read_u16::<BigEndian>()?;
            writer.write_all(&count.to_be_bytes())?;
            count
        } else {
            1
        };

        sample_number += count_occurrences as usize;

        progress!("Relabeling line: {}\r", sample_number);
    }
    tracing::trace!("");
    tracing::trace!("Done!");

    Ok(())
}

pub fn relabel_ben_file<R: Read, W: Write>(mut reader: R, mut writer: W) -> io::Result<()> {
    let mut check_buffer = [0u8; 17];
    reader.read_exact(&mut check_buffer)?;

    let variant = match &check_buffer {
        b"STANDARD BEN FILE" => BenVariant::Standard,
        b"MKVCHAIN BEN FILE" => BenVariant::MkvChain,
        _ => {
            return Err(Error::new(
                io::ErrorKind::InvalidData,
                "Invalid file format",
            ));
        }
    };

    writer.write_all(&check_buffer)?;

    relabel_ben_lines(&mut reader, &mut writer, variant)?;

    Ok(())
}

pub fn relabel_ben_lines_with_map<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    new_to_old_node_map: HashMap<usize, usize>,
    variant: BenVariant,
) -> io::Result<()> {
    let mut sample_number = 0;
    loop {
        let mut tmp_buffer = [0u8];
        let max_val_bits = match reader.read_exact(&mut tmp_buffer) {
            Ok(_) => tmp_buffer[0],
            Err(e) => {
                if e.kind() == io::ErrorKind::UnexpectedEof {
                    break;
                }
                return Err(e);
            }
        };

        let max_len_bits = reader.read_u8()?;
        let n_bytes = reader.read_u32::<BigEndian>()?;

        let ben_line = decode_ben_line(&mut reader, max_val_bits, max_len_bits, n_bytes)?;

        let assignment_vec = rle_to_vec(ben_line);
        let new_assignment_vec = assignment_vec
            .iter()
            .enumerate()
            .map(|(i, _)| {
                let new_val_pos = new_to_old_node_map.get(&i).unwrap();
                assignment_vec[*new_val_pos]
            })
            .collect::<Vec<u16>>();

        let new_rle = assign_to_rle(new_assignment_vec);

        let relabeled = encode_ben_vec_from_rle(new_rle);
        writer.write_all(&relabeled)?;

        let count_occurrences = if variant == BenVariant::MkvChain {
            let count = reader.read_u16::<BigEndian>()?;
            writer.write_all(&count.to_be_bytes())?;
            count
        } else {
            1
        };

        sample_number += count_occurrences as usize;
        progress!("Relabeling line: {}\r", sample_number);
    }
    tracing::trace!("");
    tracing::trace!("Done!");

    Ok(())
}

pub fn relabel_ben_file_with_map<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    new_to_old_node_map: HashMap<usize, usize>,
) -> io::Result<()> {
    let mut check_buffer = [0u8; 17];
    reader.read_exact(&mut check_buffer)?;

    let variant = match &check_buffer {
        b"STANDARD BEN FILE" => BenVariant::Standard,
        b"MKVCHAIN BEN FILE" => BenVariant::MkvChain,
        _ => {
            return Err(Error::new(
                io::ErrorKind::InvalidData,
                "Invalid file format",
            ));
        }
    };

    writer.write_all(&check_buffer)?;

    relabel_ben_lines_with_map(&mut reader, &mut writer, new_to_old_node_map, variant)?;

    Ok(())
}

#[cfg(test)]
mod tests;
