//! Relabeling operations for BEN files.

use crate::codec::decode::decode_ben_line;
use crate::codec::encode::encode_ben_vec_from_rle;
use crate::util::rle::{assign_slice_to_rle, rle_to_vec_in_place};
use crate::{progress, BenVariant};
use byteorder::{BigEndian, ReadBytesExt};
use std::collections::HashMap;
use std::io::{self, Error, Read, Write};

/// Convert a sparse permutation map into a dense index vector.
///
/// # Arguments
///
/// * `new_to_old_node_map` - The sparse map from new index to old index.
///
/// # Returns
///
/// Returns a dense permutation vector where `perm[new_idx] == old_idx`.
fn dense_permutation(new_to_old_node_map: &HashMap<usize, usize>) -> io::Result<Vec<usize>> {
    let Some(max_key) = new_to_old_node_map.keys().copied().max() else {
        return Ok(Vec::new());
    };

    let mut permutation = vec![usize::MAX; max_key + 1];
    for (&new_idx, &old_idx) in new_to_old_node_map {
        permutation[new_idx] = old_idx;
    }

    if permutation.iter().any(|&old_idx| old_idx == usize::MAX) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Relabel map must contain a contiguous set of new indices",
        ));
    }

    Ok(permutation)
}

/// Canonicalize the labels used inside each BEN frame.
///
/// Labels are reassigned in first-seen order within each assignment vector,
/// which can improve downstream compression ratios.
///
/// # Arguments
///
/// * `reader` - The BEN input stream without its 17-byte file banner.
/// * `writer` - The destination for the relabeled BEN frames.
/// * `variant` - The BEN variant, used to determine whether repetition counts
///   follow each frame.
///
/// # Returns
///
/// Returns `Ok(())` after all frames have been relabeled and written.
pub fn relabel_ben_lines<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    variant: BenVariant,
) -> io::Result<()> {
    let mut sample_number = 0;
    let mut label_map = HashMap::new();
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
        label_map.clear();
        label_map.reserve(ben_line.len());
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

/// Relabel an entire BEN file, preserving its leading BEN banner.
///
/// # Arguments
///
/// * `reader` - The input BEN stream, including its banner.
/// * `writer` - The destination for the relabeled BEN file.
///
/// # Returns
///
/// Returns `Ok(())` after the full BEN file has been relabeled.
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

/// Relabel BEN frames using an externally supplied node map.
///
/// `new_to_old_node_map` maps the new node index to the position that should be
/// read from the original assignment vector.
///
/// # Arguments
///
/// * `reader` - The BEN input stream without its 17-byte file banner.
/// * `writer` - The destination for the relabeled BEN frames.
/// * `new_to_old_node_map` - The permutation describing how node positions
///   should be reordered.
/// * `variant` - The BEN variant, used to determine whether repetition counts
///   follow each frame.
///
/// # Returns
///
/// Returns `Ok(())` after all frames have been relabeled and written.
pub fn relabel_ben_lines_with_map<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    new_to_old_node_map: HashMap<usize, usize>,
    variant: BenVariant,
) -> io::Result<()> {
    let mut sample_number = 0;
    let permutation = dense_permutation(&new_to_old_node_map)?;
    let mut assignment_vec = Vec::new();
    let mut new_assignment_vec = vec![0u16; permutation.len()];
    let mut new_rle = Vec::new();
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
        rle_to_vec_in_place(&ben_line, &mut assignment_vec);

        if assignment_vec.len() != permutation.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "Relabel map length {} does not match assignment length {}",
                    permutation.len(),
                    assignment_vec.len()
                ),
            ));
        }

        for (new_idx, &old_idx) in permutation.iter().enumerate() {
            new_assignment_vec[new_idx] = assignment_vec[old_idx];
        }

        assign_slice_to_rle(&new_assignment_vec, &mut new_rle);

        let relabeled = encode_ben_vec_from_rle(new_rle.clone());
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

/// Relabel an entire BEN file using an externally supplied node map.
///
/// # Arguments
///
/// * `reader` - The input BEN stream, including its banner.
/// * `writer` - The destination for the relabeled BEN file.
/// * `new_to_old_node_map` - The permutation describing how node positions
///   should be reordered.
///
/// # Returns
///
/// Returns `Ok(())` after the full BEN file has been relabeled.
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
