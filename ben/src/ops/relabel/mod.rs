//! Relabeling operations for BEN files.
//!
//! All seven logical relabel/convert operations route through the single [`relabel_ben_file`]
//! driver, parameterised by [`RelabelOptions`].

mod errors;
mod permutation;

#[cfg(test)]
mod tests;

use crate::codec::decode::decode_ben_line;
use crate::codec::BenEncodeFrame;
use crate::format::banners::{variant_from_banner, BANNER_LEN};
use crate::format::FormatError;
use crate::io::reader::BenStreamReader;
use crate::io::writer::BenStreamWriter;
use crate::progress::Spinner;
use crate::BenVariant;
use byteorder::{BigEndian, ReadBytesExt};
use permutation::{
    dense_permutation, first_seen_relabel_assignment, first_seen_relabel_rle, permute_assignment,
};
use std::collections::HashMap;
use std::io::{self, Cursor, Read, Write};

/// What value-level transform to apply to each decoded assignment.
#[non_exhaustive]
pub enum RelabelTransform {
    /// Pass each assignment through unchanged.
    Identity,
    /// Rewrite labels in first-appearance order, starting at 1.
    FirstSeen,
    /// Reorder elements according to a `new_idx -> old_idx` map.
    NodePermutation(HashMap<usize, usize>),
}

/// Whether the driver may merge adjacent equal output assignments.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RunPolicy {
    /// Each input frame produces a separate output frame; counts are preserved where the target
    /// variant can encode them, and expanded to one-sample frames otherwise.
    PreserveFrameBoundaries,
    /// Adjacent identical output assignments are merged into a single counted frame where the
    /// target variant can encode counts.
    CollapseAdjacentEqualAssignments,
}

/// Options for [`relabel_ben_file`].
///
/// Constructed via [`RelabelOptions::first_seen`], [`RelabelOptions::node_permutation`], or
/// [`RelabelOptions::convert_to`], then refined with the `with_*` builder methods.
#[non_exhaustive]
pub struct RelabelOptions {
    transform: RelabelTransform,
    target_variant: Option<BenVariant>,
    max_samples: Option<usize>,
    run_policy: RunPolicy,
}

impl RelabelOptions {
    /// First-seen district relabeling, preserving the input variant and frame boundaries.
    pub fn first_seen() -> Self {
        Self {
            transform: RelabelTransform::FirstSeen,
            target_variant: None,
            max_samples: None,
            run_policy: RunPolicy::PreserveFrameBoundaries,
        }
    }

    /// Node permutation through `new_idx -> old_idx`, preserving the input variant and frame
    /// boundaries.
    pub fn node_permutation(map: HashMap<usize, usize>) -> Self {
        Self {
            transform: RelabelTransform::NodePermutation(map),
            target_variant: None,
            max_samples: None,
            run_policy: RunPolicy::PreserveFrameBoundaries,
        }
    }

    /// Convert to `target` without relabeling, collapsing adjacent equal assignments to preserve
    /// today's conversion compression behavior.
    pub fn convert_to(target: BenVariant) -> Self {
        Self {
            transform: RelabelTransform::Identity,
            target_variant: Some(target),
            max_samples: None,
            run_policy: RunPolicy::CollapseAdjacentEqualAssignments,
        }
    }

    /// Set a concrete sample limit. Convenience form for call sites that hold a plain `usize`; for
    /// an already-optional value (e.g. a parsed CLI argument) use [`Self::with_max_samples_opt`]
    /// instead of unwrapping.
    pub fn with_max_samples(mut self, n: usize) -> Self {
        self.max_samples = Some(n);
        self
    }

    /// Set the sample limit from an optional value: `Some(n)` sets the limit, `None` clears it. Lets
    /// CLI argument plumbing pass an `Option<usize>` straight through.
    pub fn with_max_samples_opt(mut self, n: Option<usize>) -> Self {
        self.max_samples = n;
        self
    }

    pub fn with_target_variant(mut self, target: BenVariant) -> Self {
        self.target_variant = Some(target);
        self
    }

    pub fn with_run_policy(mut self, policy: RunPolicy) -> Self {
        self.run_policy = policy;
        self
    }

    pub fn transform(&self) -> &RelabelTransform {
        &self.transform
    }

    pub fn target_variant(&self) -> Option<BenVariant> {
        self.target_variant
    }

    pub fn max_samples(&self) -> Option<usize> {
        self.max_samples
    }

    pub fn run_policy(&self) -> RunPolicy {
        self.run_policy
    }
}

/// Process a BEN file according to the supplied options.
///
/// All seven logical relabel/convert operations route through this driver. Internally chooses
/// between an RLE-fast-path byte walker (first-seen relabeling, no variant change,
/// frame-preserving, Standard/MkvChain input) and the high-level decoder driver (everything else).
pub fn relabel_ben_file<R: Read, W: Write>(
    reader: R,
    writer: W,
    options: RelabelOptions,
) -> io::Result<()> {
    let mut reader = reader;
    let mut banner = [0u8; BANNER_LEN];
    reader.read_exact(&mut banner)?;
    let input_variant = variant_from_banner(&banner).ok_or_else(|| {
        io::Error::from(FormatError::UnknownBanner {
            actual: banner.to_vec(),
        })
    })?;

    if can_use_first_seen_fast_path(
        &options.transform,
        options.target_variant,
        input_variant,
        options.run_policy,
    ) {
        let mut writer = writer;
        writer.write_all(&banner)?;
        return relabel_first_seen_via_byte_walk(
            reader,
            writer,
            input_variant,
            options.max_samples,
        );
    }

    let target_variant = options.target_variant.unwrap_or(input_variant);
    let chained = Cursor::new(banner).chain(reader);
    let permutation = match &options.transform {
        RelabelTransform::NodePermutation(map) => Some(dense_permutation(map)?),
        _ => None,
    };
    relabel_via_decoder(
        chained,
        writer,
        target_variant,
        options.max_samples,
        options.run_policy,
        |a| match &options.transform {
            RelabelTransform::Identity => Ok(a.to_vec()),
            RelabelTransform::FirstSeen => Ok(first_seen_relabel_assignment(a)),
            RelabelTransform::NodePermutation(_) => {
                permute_assignment(a, permutation.as_ref().expect("set above"))
            }
        },
    )
}

/// Convert a BEN file to the requested variant without relabeling.
pub fn convert_ben_file<R: Read, W: Write>(
    reader: R,
    writer: W,
    target: BenVariant,
) -> io::Result<()> {
    relabel_ben_file(reader, writer, RelabelOptions::convert_to(target))
}

/// True when the driver may take the byte-walking RLE fast path.
///
/// Kept as a single pure predicate (rather than inlined into [`relabel_ben_file`]) so the exact
/// conditions under which the fast path is safe are stated in one place and can be exhaustively
/// covered by a dedicated unit-test matrix.
fn can_use_first_seen_fast_path(
    transform: &RelabelTransform,
    target_variant: Option<BenVariant>,
    input: BenVariant,
    run_policy: RunPolicy,
) -> bool {
    matches!(transform, RelabelTransform::FirstSeen)
        && target_variant.is_none()
        && run_policy == RunPolicy::PreserveFrameBoundaries
        && matches!(input, BenVariant::Standard | BenVariant::MkvChain)
}

/// Decode a BEN stream, apply a per-assignment transform, and re-encode into the target variant.
///
/// With [`RunPolicy::PreserveFrameBoundaries`], the implementation never merges across input frame
/// boundaries: MkvChain/TwoDelta targets receive counted output frames, Standard targets receive
/// `count` one-sample frames because Standard cannot encode repetition counts. With
/// [`RunPolicy::CollapseAdjacentEqualAssignments`], the existing [`BenStreamWriter`] merging path
/// is used.
fn relabel_via_decoder<R: Read, W: Write, F>(
    reader: R,
    writer: W,
    target_variant: BenVariant,
    max_samples: Option<usize>,
    run_policy: RunPolicy,
    mut transform: F,
) -> io::Result<()>
where
    F: FnMut(&[u16]) -> io::Result<Vec<u16>>,
{
    let mut decoder = BenStreamReader::from_ben(reader)?.silent(true);
    let mut writer = BenStreamWriter::for_ben(writer, target_variant)?;
    let mut sample_number = 0usize;
    let spinner = Spinner::new("Relabeling line");

    // Both run policies share the same per-frame bookkeeping (sample limit, transform, output count,
    // progress); they differ only in how the relabeled assignment is emitted. `out_count` is bounded
    // by the input frame's `count` (a `u16`), so the `as u16` cast on the preserve path cannot
    // truncate.
    decoder.for_each_assignment(|assignment, count| {
        if max_samples.is_some_and(|limit| sample_number >= limit) {
            return Ok(false);
        }

        let relabeled = transform(assignment)?;
        let out_count = max_samples
            .map(|limit| (limit - sample_number).min(count as usize))
            .unwrap_or(count as usize);

        if out_count > 0 {
            match run_policy {
                // Emit `out_count` separate assignments; the writer merges adjacent equal ones into
                // a single counted frame where the target variant can encode counts.
                RunPolicy::CollapseAdjacentEqualAssignments => {
                    for _ in 1..out_count {
                        writer.write_assignment(relabeled.clone())?;
                    }
                    writer.write_assignment(relabeled)?;
                }
                // Emit one counted frame, never merging across input frame boundaries.
                RunPolicy::PreserveFrameBoundaries => {
                    writer.write_frame(relabeled, out_count as u16)?;
                }
            }
        }

        sample_number += out_count;
        spinner.set_count(sample_number as u64);
        Ok(true)
    })?;
    writer.finish()?;

    Ok(())
}

/// Byte-walking RLE fast path for first-seen relabeling on Standard/MkvChain.
///
/// Walks 6-byte frame headers, decodes the RLE in place, applies first-seen relabeling on the
/// `(val, len)` pairs, and re-encodes. Skips assignment vector materialization entirely. The output
/// banner has been emitted by the caller before this is invoked.
fn relabel_first_seen_via_byte_walk<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    input_variant: BenVariant,
    max_samples: Option<usize>,
) -> io::Result<()> {
    let mut sample_number = 0usize;
    let spinner = Spinner::new("Relabeling line");
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
        first_seen_relabel_rle(&mut ben_line);

        let count_occurrences = if input_variant == BenVariant::MkvChain {
            let count = reader.read_u16::<BigEndian>()?;
            max_samples
                .map(|limit| ((limit - sample_number).min(count as usize)) as u16)
                .unwrap_or(count)
        } else {
            1
        };

        let frame = BenEncodeFrame::from_rle(ben_line, input_variant, Some(count_occurrences));
        writer.write_all(frame.as_slice())?;

        sample_number += count_occurrences as usize;
        spinner.set_count(sample_number as u64);
    }

    Ok(())
}
