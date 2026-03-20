//! Relabeling operations for BEN files.

mod errors;
use errors::RelabelError;

use crate::codec::decode::decode_ben_line;
use crate::codec::{BenConstruct, BenEncodeFrame};
use crate::format::banners::{variant_from_banner, BANNER_LEN};
use crate::format::FormatError;
use crate::io::reader::BenDecoder;
use crate::io::writer::BenEncoder;
use crate::util::rle::{assign_slice_to_rle, rle_to_vec_in_place};
use crate::{progress, BenVariant};
use byteorder::{BigEndian, ReadBytesExt};
use std::collections::HashMap;
use std::io::{self, Cursor, Read, Write};

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

    let missing = permutation.iter().filter(|&&x| x == usize::MAX).count();
    if missing > 0 {
        return Err(io::Error::from(RelabelError::NonContiguousMap {
            max_key,
            missing,
        }));
    }

    Ok(permutation)
}

/// Canonicalize an assignment vector by remapping labels in first-seen order.
///
/// # Arguments
///
/// * `assignment` - The original assignment slice whose labels should be remapped.
///
/// # Returns
///
/// Returns a new vector with labels replaced by sequential integers starting at 1,
/// assigned in the order they first appear.
fn canonicalize_assignment(assignment: &[u16]) -> Vec<u16> {
    let mut label_map = HashMap::new();
    let mut next_label = 0u16;
    let mut out = Vec::with_capacity(assignment.len());

    for &value in assignment {
        let mapped = match label_map.get(&value) {
            Some(mapped) => *mapped,
            None => {
                next_label += 1;
                label_map.insert(value, next_label);
                next_label
            }
        };
        out.push(mapped);
    }

    out
}

/// Reorder an assignment vector according to a dense permutation.
///
/// # Arguments
///
/// * `assignment` - The original assignment slice to permute.
/// * `permutation` - A dense permutation vector where `permutation[new_idx] == old_idx`.
///
/// # Returns
///
/// Returns a new vector with elements rearranged so that `out[new_idx] == assignment[old_idx]`,
/// or an error if the lengths do not match.
fn permute_assignment(assignment: &[u16], permutation: &[usize]) -> io::Result<Vec<u16>> {
    if assignment.len() != permutation.len() {
        return Err(io::Error::from(RelabelError::LengthMismatch {
            map_len: permutation.len(),
            assignment_len: assignment.len(),
        }));
    }

    let mut out = vec![0u16; permutation.len()];
    for (new_idx, &old_idx) in permutation.iter().enumerate() {
        out[new_idx] = assignment[old_idx];
    }
    Ok(out)
}

/// Decode a BEN stream, apply a per-assignment transform, and re-encode into the target variant.
///
/// # Arguments
///
/// * `reader` - The full BEN input stream, including its banner.
/// * `writer` - The destination for the re-encoded BEN output.
/// * `variant` - The target BEN variant to encode into.
/// * `max_samples` - Optional upper bound on the number of expanded samples to write.
/// * `transform` - A closure that takes ownership of each decoded assignment
///   vector and returns the transformed version.
///
/// # Returns
///
/// Returns `Ok(())` after all (or up to `max_samples`) samples have been processed.
fn relabel_ben_file_via_decoder<R: Read, W: Write, F>(
    reader: R,
    writer: W,
    variant: BenVariant,
    max_samples: Option<usize>,
    mut transform: F,
) -> io::Result<()>
where
    F: FnMut(&[u16]) -> io::Result<Vec<u16>>,
{
    let mut decoder = BenDecoder::new(reader)?.silent(true);
    let mut encoder = BenEncoder::new(writer, variant)?;
    let mut sample_number = 0usize;

    decoder.for_each_assignment(|assignment, count| {
        if max_samples.is_some_and(|limit| sample_number >= limit) {
            return Ok(false);
        }

        let relabeled = transform(assignment)?;
        let out_count = max_samples
            .map(|limit| (limit - sample_number).min(count as usize))
            .unwrap_or(count as usize);

        for _ in 1..out_count {
            encoder.write_assignment(relabeled.clone())?;
        }
        encoder.write_assignment(relabeled)?;

        sample_number += out_count;
        progress!("Relabelling line: {}\r", sample_number);
        Ok(true)
    })?;

    tracing::trace!("");
    tracing::trace!("Done!");
    encoder.finish()?;
    Ok(())
}

/// Determine the BEN variant from a 17-byte file banner.
///
/// # Arguments
///
/// * `header` - The 17-byte banner read from the start of a BEN file.
///
/// # Returns
///
/// Returns the detected `BenVariant`, or an error if the banner is not recognized.
fn detect_ben_variant(header: &[u8; 17]) -> io::Result<BenVariant> {
    match header {
        b"STANDARD BEN FILE" => Ok(BenVariant::Standard),
        b"MKVCHAIN BEN FILE" => Ok(BenVariant::MkvChain),
        b"TWODELTA BEN FILE" => Ok(BenVariant::TwoDelta),
        _ => Err(io::Error::from(FormatError::UnknownBanner {
            actual: header.to_vec(),
        })),
    }
}

/// Shared implementation for converting a BEN file into a different variant without relabeling.
///
/// # Arguments
///
/// * `reader` - The input BEN stream, including its banner.
/// * `writer` - The destination for the converted BEN output.
/// * `target_variant` - The BEN variant to encode into.
/// * `max_samples` - Optional upper bound on the number of expanded samples to write.
///
/// # Returns
///
/// Returns `Ok(())` after all (or up to `max_samples`) samples have been converted.
fn convert_ben_file_impl<R: Read, W: Write>(
    mut reader: R,
    writer: W,
    target_variant: BenVariant,
    max_samples: Option<usize>,
) -> io::Result<()> {
    let mut check_buffer = [0u8; BANNER_LEN];
    reader.read_exact(&mut check_buffer)?;
    let _input_variant = detect_ben_variant(&check_buffer)?;

    let chained = Cursor::new(check_buffer).chain(reader);
    relabel_ben_file_via_decoder(chained, writer, target_variant, max_samples, |a| {
        Ok(a.to_vec())
    })
}

/// Rewrite a BEN file into the requested BEN variant.
///
/// # Arguments
///
/// * `reader` - The input BEN stream, including its banner.
/// * `writer` - The destination for the converted BEN output.
/// * `target_variant` - The BEN variant to encode into.
///
/// # Returns
///
/// Returns `Ok(())` after the full BEN file has been converted.
pub fn convert_ben_file<R: Read, W: Write>(
    reader: R,
    writer: W,
    target_variant: BenVariant,
) -> io::Result<()> {
    convert_ben_file_impl(reader, writer, target_variant, None)
}

/// Rewrite at most `max_samples` expanded samples into the requested BEN variant.
///
/// # Arguments
///
/// * `reader` - The input BEN stream, including its banner.
/// * `writer` - The destination for the converted BEN output.
/// * `target_variant` - The BEN variant to encode into.
/// * `max_samples` - The maximum number of expanded samples to write.
///
/// # Returns
///
/// Returns `Ok(())` after up to `max_samples` samples have been converted.
pub fn convert_ben_file_limit<R: Read, W: Write>(
    reader: R,
    writer: W,
    target_variant: BenVariant,
    max_samples: usize,
) -> io::Result<()> {
    convert_ben_file_impl(reader, writer, target_variant, Some(max_samples))
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
    relabel_ben_lines_impl(&mut reader, &mut writer, variant, None)
}

/// Canonicalize up to a bounded number of samples from a BEN frame stream.
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
/// * `max_samples` - The maximum number of expanded samples to write.
///
/// # Returns
///
/// Returns `Ok(())` after up to `max_samples` samples have been relabeled and
/// written.
pub fn relabel_ben_lines_limit<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    variant: BenVariant,
    max_samples: usize,
) -> io::Result<()> {
    relabel_ben_lines_impl(&mut reader, &mut writer, variant, Some(max_samples))
}

/// Shared implementation for canonical BEN relabeling.
///
/// # Arguments
///
/// * `reader` - The BEN input stream without its 17-byte file banner.
/// * `writer` - The destination for the relabeled BEN frames.
/// * `variant` - The BEN variant, used to determine whether repetition counts
///   follow each frame.
/// * `max_samples` - Optional upper bound on the number of expanded samples to write.
///
/// # Returns
///
/// Returns `Ok(())` after all (or up to `max_samples`) samples have been relabeled.
fn relabel_ben_lines_impl<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    variant: BenVariant,
    max_samples: Option<usize>,
) -> io::Result<()> {
    let mut sample_number = 0;
    let mut label_map = HashMap::new();
    loop {
        if max_samples.is_some_and(|limit| sample_number >= limit) {
            break;
        }
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

        let count_occurrences = if variant == BenVariant::MkvChain {
            let count = reader.read_u16::<BigEndian>()?;
            let out_count = max_samples
                .map(|limit| ((limit - sample_number).min(count as usize)) as u16)
                .unwrap_or(count);
            out_count
        } else {
            1
        };

        let relabeled = BenEncodeFrame::from_rle(ben_line, None);
        writer.write_all(relabeled.as_slice())?;
        if variant == BenVariant::MkvChain {
            writer.write_all(&count_occurrences.to_be_bytes())?;
        }

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
    relabel_ben_file_impl(&mut reader, &mut writer, None)
}

/// Relabel at most `max_samples` expanded samples from a BEN file, preserving
/// its leading BEN banner.
///
/// # Arguments
///
/// * `reader` - The input BEN stream, including its banner.
/// * `writer` - The destination for the relabeled BEN file.
/// * `max_samples` - The maximum number of expanded samples to write.
///
/// # Returns
///
/// Returns `Ok(())` after up to `max_samples` samples have been relabeled.
pub fn relabel_ben_file_limit<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    max_samples: usize,
) -> io::Result<()> {
    relabel_ben_file_impl(&mut reader, &mut writer, Some(max_samples))
}

/// Shared implementation for BEN-file canonical relabeling.
///
/// # Arguments
///
/// * `reader` - The input BEN stream, including its banner.
/// * `writer` - The destination for the relabeled BEN file.
/// * `max_samples` - Optional upper bound on the number of expanded samples to write.
///
/// # Returns
///
/// Returns `Ok(())` after all (or up to `max_samples`) samples have been relabeled.
fn relabel_ben_file_impl<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    max_samples: Option<usize>,
) -> io::Result<()> {
    let mut check_buffer = [0u8; BANNER_LEN];
    reader.read_exact(&mut check_buffer)?;

    let variant = variant_from_banner(&check_buffer).ok_or_else(|| {
        io::Error::from(FormatError::UnknownBanner {
            actual: check_buffer.to_vec(),
        })
    })?;

    match variant {
        BenVariant::Standard | BenVariant::MkvChain => {
            writer.write_all(&check_buffer)?;
            relabel_ben_lines_impl(&mut reader, &mut writer, variant, max_samples)?
        }
        BenVariant::TwoDelta => {
            let chained = Cursor::new(check_buffer).chain(reader);
            relabel_ben_file_via_decoder(
                chained,
                &mut writer,
                variant,
                max_samples,
                |assignment| Ok(canonicalize_assignment(assignment)),
            )?
        }
    }

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
    relabel_ben_lines_with_map_impl(&mut reader, &mut writer, new_to_old_node_map, variant, None)
}

/// Relabel BEN frames using an externally supplied node map, up to a bounded
/// number of expanded samples.
///
/// # Arguments
///
/// * `reader` - The BEN input stream without its 17-byte file banner.
/// * `writer` - The destination for the relabeled BEN frames.
/// * `new_to_old_node_map` - The permutation describing how node positions
///   should be reordered.
/// * `variant` - The BEN variant, used to determine whether repetition counts
///   follow each frame.
/// * `max_samples` - The maximum number of expanded samples to write.
///
/// # Returns
///
/// Returns `Ok(())` after up to `max_samples` samples have been relabeled and
/// written.
pub fn relabel_ben_lines_with_map_limit<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    new_to_old_node_map: HashMap<usize, usize>,
    variant: BenVariant,
    max_samples: usize,
) -> io::Result<()> {
    relabel_ben_lines_with_map_impl(
        &mut reader,
        &mut writer,
        new_to_old_node_map,
        variant,
        Some(max_samples),
    )
}

/// Shared implementation for mapped BEN relabeling.
///
/// # Arguments
///
/// * `reader` - The BEN input stream without its 17-byte file banner.
/// * `writer` - The destination for the relabeled BEN frames.
/// * `new_to_old_node_map` - The permutation describing how node positions
///   should be reordered.
/// * `variant` - The BEN variant, used to determine whether repetition counts
///   follow each frame.
/// * `max_samples` - Optional upper bound on the number of expanded samples to write.
///
/// # Returns
///
/// Returns `Ok(())` after all (or up to `max_samples`) samples have been relabeled.
fn relabel_ben_lines_with_map_impl<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    new_to_old_node_map: HashMap<usize, usize>,
    variant: BenVariant,
    max_samples: Option<usize>,
) -> io::Result<()> {
    let mut sample_number = 0;
    let permutation = dense_permutation(&new_to_old_node_map)?;
    let mut assignment_vec = Vec::new();
    let mut new_assignment_vec = vec![0u16; permutation.len()];
    let mut new_rle = Vec::new();
    loop {
        if max_samples.is_some_and(|limit| sample_number >= limit) {
            break;
        }
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
            return Err(io::Error::from(RelabelError::LengthMismatch {
                map_len: permutation.len(),
                assignment_len: assignment_vec.len(),
            }));
        }

        for (new_idx, &old_idx) in permutation.iter().enumerate() {
            new_assignment_vec[new_idx] = assignment_vec[old_idx];
        }

        assign_slice_to_rle(&new_assignment_vec, &mut new_rle);

        let count_occurrences = if variant == BenVariant::MkvChain {
            let count = reader.read_u16::<BigEndian>()?;
            let out_count = max_samples
                .map(|limit| ((limit - sample_number).min(count as usize)) as u16)
                .unwrap_or(count);
            out_count
        } else {
            1
        };

        let relabeled = BenEncodeFrame::from_rle(new_rle.clone(), None);
        writer.write_all(relabeled.as_slice())?;
        if variant == BenVariant::MkvChain {
            writer.write_all(&count_occurrences.to_be_bytes())?;
        }

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
    relabel_ben_file_with_map_impl(&mut reader, &mut writer, new_to_old_node_map, None)
}

/// Relabel at most `max_samples` expanded samples from a BEN file using an
/// externally supplied node map.
///
/// # Arguments
///
/// * `reader` - The input BEN stream, including its banner.
/// * `writer` - The destination for the relabeled BEN file.
/// * `new_to_old_node_map` - The permutation describing how node positions
///   should be reordered.
/// * `max_samples` - The maximum number of expanded samples to write.
///
/// # Returns
///
/// Returns `Ok(())` after up to `max_samples` samples have been relabeled.
pub fn relabel_ben_file_with_map_limit<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    new_to_old_node_map: HashMap<usize, usize>,
    max_samples: usize,
) -> io::Result<()> {
    relabel_ben_file_with_map_impl(
        &mut reader,
        &mut writer,
        new_to_old_node_map,
        Some(max_samples),
    )
}

/// Shared implementation for BEN-file mapped relabeling.
///
/// # Arguments
///
/// * `reader` - The input BEN stream, including its banner.
/// * `writer` - The destination for the relabeled BEN file.
/// * `new_to_old_node_map` - The permutation describing how node positions
///   should be reordered.
/// * `max_samples` - Optional upper bound on the number of expanded samples to write.
///
/// # Returns
///
/// Returns `Ok(())` after all (or up to `max_samples`) samples have been relabeled.
fn relabel_ben_file_with_map_impl<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    new_to_old_node_map: HashMap<usize, usize>,
    max_samples: Option<usize>,
) -> io::Result<()> {
    let mut check_buffer = [0u8; BANNER_LEN];
    reader.read_exact(&mut check_buffer)?;

    let variant = variant_from_banner(&check_buffer).ok_or_else(|| {
        io::Error::from(FormatError::UnknownBanner {
            actual: check_buffer.to_vec(),
        })
    })?;

    match variant {
        BenVariant::Standard | BenVariant::MkvChain => {
            writer.write_all(&check_buffer)?;
            relabel_ben_lines_with_map_impl(
                &mut reader,
                &mut writer,
                new_to_old_node_map,
                variant,
                max_samples,
            )?
        }
        BenVariant::TwoDelta => {
            let permutation = dense_permutation(&new_to_old_node_map)?;
            let chained = Cursor::new(check_buffer).chain(reader);
            relabel_ben_file_via_decoder(
                chained,
                &mut writer,
                variant,
                max_samples,
                |assignment| permute_assignment(assignment, &permutation),
            )?
        }
    }

    Ok(())
}

/// Canonicalize BEN assignments and write them using the requested BEN variant.
///
/// # Arguments
///
/// * `reader` - The input BEN stream, including its banner.
/// * `writer` - The destination for the relabeled BEN output.
/// * `target_variant` - The BEN variant to encode into.
///
/// # Returns
///
/// Returns `Ok(())` after the full BEN file has been relabeled and converted.
pub fn relabel_ben_file_as_variant<R: Read, W: Write>(
    mut reader: R,
    writer: W,
    target_variant: BenVariant,
) -> io::Result<()> {
    let mut check_buffer = [0u8; BANNER_LEN];
    reader.read_exact(&mut check_buffer)?;
    let _input_variant = detect_ben_variant(&check_buffer)?;

    let chained = Cursor::new(check_buffer).chain(reader);
    relabel_ben_file_via_decoder(chained, writer, target_variant, None, |assignment| {
        Ok(canonicalize_assignment(&assignment))
    })
}

/// Canonicalize up to `max_samples` expanded samples and write the requested BEN variant.
///
/// # Arguments
///
/// * `reader` - The input BEN stream, including its banner.
/// * `writer` - The destination for the relabeled BEN output.
/// * `target_variant` - The BEN variant to encode into.
/// * `max_samples` - The maximum number of expanded samples to write.
///
/// # Returns
///
/// Returns `Ok(())` after up to `max_samples` samples have been relabeled and converted.
pub fn relabel_ben_file_as_variant_limit<R: Read, W: Write>(
    mut reader: R,
    writer: W,
    target_variant: BenVariant,
    max_samples: usize,
) -> io::Result<()> {
    let mut check_buffer = [0u8; BANNER_LEN];
    reader.read_exact(&mut check_buffer)?;
    let _input_variant = detect_ben_variant(&check_buffer)?;

    let chained = Cursor::new(check_buffer).chain(reader);
    relabel_ben_file_via_decoder(
        chained,
        writer,
        target_variant,
        Some(max_samples),
        |assignment| Ok(canonicalize_assignment(assignment)),
    )
}

/// Relabel a BEN file with a supplied node map and write the requested BEN variant.
///
/// # Arguments
///
/// * `reader` - The input BEN stream, including its banner.
/// * `writer` - The destination for the relabeled BEN output.
/// * `new_to_old_node_map` - The permutation describing how node positions
///   should be reordered.
/// * `target_variant` - The BEN variant to encode into.
///
/// # Returns
///
/// Returns `Ok(())` after the full BEN file has been relabeled and converted.
pub fn relabel_ben_file_with_map_as_variant<R: Read, W: Write>(
    mut reader: R,
    writer: W,
    new_to_old_node_map: HashMap<usize, usize>,
    target_variant: BenVariant,
) -> io::Result<()> {
    let mut check_buffer = [0u8; BANNER_LEN];
    reader.read_exact(&mut check_buffer)?;
    let _input_variant = detect_ben_variant(&check_buffer)?;

    let permutation = dense_permutation(&new_to_old_node_map)?;
    let chained = Cursor::new(check_buffer).chain(reader);
    relabel_ben_file_via_decoder(chained, writer, target_variant, None, |assignment| {
        permute_assignment(assignment, &permutation)
    })
}

/// Relabel up to `max_samples` expanded samples with a supplied node map and write the requested BEN variant.
///
/// # Arguments
///
/// * `reader` - The input BEN stream, including its banner.
/// * `writer` - The destination for the relabeled BEN output.
/// * `new_to_old_node_map` - The permutation describing how node positions
///   should be reordered.
/// * `target_variant` - The BEN variant to encode into.
/// * `max_samples` - The maximum number of expanded samples to write.
///
/// # Returns
///
/// Returns `Ok(())` after up to `max_samples` samples have been relabeled and converted.
pub fn relabel_ben_file_with_map_as_variant_limit<R: Read, W: Write>(
    mut reader: R,
    writer: W,
    new_to_old_node_map: HashMap<usize, usize>,
    target_variant: BenVariant,
    max_samples: usize,
) -> io::Result<()> {
    let mut check_buffer = [0u8; BANNER_LEN];
    reader.read_exact(&mut check_buffer)?;
    let _input_variant = detect_ben_variant(&check_buffer)?;

    let permutation = dense_permutation(&new_to_old_node_map)?;
    let chained = Cursor::new(check_buffer).chain(reader);
    relabel_ben_file_via_decoder(
        chained,
        writer,
        target_variant,
        Some(max_samples),
        |assignment| permute_assignment(assignment, &permutation),
    )
}

#[cfg(test)]
mod tests;
