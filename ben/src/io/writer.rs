use crate::codec::encode::{
    build_twodelta_runs_with_hint, encode_ben32_assignments, encode_ben_vec_from_assign,
    encode_twodelta_vec_with_hint, BenFrame, TwoDeltaFrame,
};
use crate::codec::translate::ben_to_ben32_lines;
use crate::format::banners::{banner_for_variant, has_known_banner_prefix, BANNER_LEN};
use crate::io::reader::BenDecoder;
use crate::util::rle::assign_to_rle;
use crate::BenVariant;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{self, BufRead, Read, Result, Write};
use xz2::write::XzEncoder;

const XBEN_TWODELTA_FULL_TAG: u8 = 0;
const XBEN_TWODELTA_DELTA_TAG: u8 = 1;

enum BufferedBenFrame {
    Ben(BenFrame),
    TwoDelta(TwoDeltaFrame),
}

impl BufferedBenFrame {
    fn as_slice(&self) -> &[u8] {
        match self {
            Self::Ben(frame) => frame.as_slice(),
            Self::TwoDelta(frame) => frame.as_slice(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct AssignmentHints {
    is_repeated: bool,
    delta_pair: Option<(u16, u16)>,
}

/// Check whether two assignment vectors are identical element-by-element.
///
/// # Arguments
///
/// * `previous_sample` - The previous assignment vector.
/// * `assign_vec` - The current assignment vector.
///
/// # Returns
///
/// Returns `true` if both vectors have the same length and every element matches.
fn is_repeated_assignment(previous_sample: &[u16], assign_vec: &[u16]) -> bool {
    if previous_sample.is_empty() || previous_sample.len() != assign_vec.len() {
        return false;
    }

    for (&previous, &current) in previous_sample.iter().zip(assign_vec.iter()) {
        if previous != current {
            return false;
        }
    }

    true
}

/// Analyze the transition between two assignment vectors for two-delta encoding.
///
/// Determines whether the assignments are identical (repeated) or differ by
/// exactly one swapped pair of values, which qualifies for delta encoding.
///
/// When `masks` are available the pair is detected in O(K) where K is the
/// number of unique label values, by checking each label's mask positions for
/// changes rather than scanning the full assignment array.
///
/// # Arguments
///
/// * `previous_sample` - The previous assignment vector.
/// * `assign_vec` - The current assignment vector.
/// * `masks` - An optional index map from each label value to its sorted
///   positions in the previous assignment.
///
/// # Returns
///
/// Returns an `AssignmentHints` with `is_repeated` set if the vectors match,
/// or `delta_pair` set if all differences involve exactly two values.
fn analyze_twodelta_transition(
    previous_sample: &[u16],
    assign_vec: &[u16],
    masks: Option<&HashMap<u16, Vec<usize>>>,
) -> AssignmentHints {
    if previous_sample.is_empty() || previous_sample.len() != assign_vec.len() {
        return AssignmentHints::default();
    }

    // Fast path: use masks to find the pair in O(K) instead of O(N).
    if let Some(masks) = masks {
        if previous_sample == assign_vec {
            return AssignmentHints {
                is_repeated: true,
                delta_pair: None,
            };
        }

        // Check each label's mask positions. Only labels involved in the swap
        // will have any changed positions; all others short-circuit immediately.
        let mut pair: Option<(u16, u16)> = None;
        for (&label, positions) in masks {
            for &pos in positions {
                if assign_vec[pos] != label {
                    let other = assign_vec[pos];
                    match pair {
                        None => {
                            pair = Some((label, other));
                            break;
                        }
                        Some((a, b)) => {
                            if (label == a || label == b) && (other == a || other == b) {
                                break;
                            }
                            // More than two values involved.
                            return AssignmentHints {
                                is_repeated: false,
                                delta_pair: None,
                            };
                        }
                    }
                }
            }
        }

        return AssignmentHints {
            is_repeated: false,
            delta_pair: pair,
        };
    }

    // Slow path: full O(N) scan when masks are not available.
    let Some(first_mismatch) = previous_sample
        .iter()
        .zip(assign_vec.iter())
        .position(|(&previous, &current)| previous != current)
    else {
        return AssignmentHints {
            is_repeated: true,
            delta_pair: None,
        };
    };

    let pair = (previous_sample[first_mismatch], assign_vec[first_mismatch]);

    for (&previous, &current) in previous_sample
        .iter()
        .zip(assign_vec.iter())
        .skip(first_mismatch + 1)
    {
        if previous == current {
            continue;
        }

        if previous != pair.0 && previous != pair.1 {
            return AssignmentHints {
                is_repeated: false,
                delta_pair: None,
            };
        }

        if current != pair.0 && current != pair.1 {
            return AssignmentHints {
                is_repeated: false,
                delta_pair: None,
            };
        }
    }

    AssignmentHints {
        is_repeated: false,
        delta_pair: Some(pair),
    }
}

/// Extract and validate the `assignment` array from a JSON object.
///
/// # Arguments
///
/// * `data` - A JSON value expected to contain an `assignment` array of integers.
///
/// # Returns
///
/// Returns a `Vec<u16>` of assignment values, or an error if the field is
/// missing, not an array, or contains values that do not fit in a `u16`.
fn parse_json_assignment(data: Value) -> Result<Vec<u16>> {
    let assign_vec = data["assignment"].as_array().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "'assignment' field either missing or is not an array of integers",
        )
    })?;

    assign_vec
        .iter()
        .map(|x| {
            let u = x.as_u64().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "The value '{}' could not be unwrapped as an unsigned 64 bit integer.",
                        x
                    ),
                )
            })?;

            u16::try_from(u).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("The value '{}' is too large to fit in a u16.", u),
                )
            })
        })
        .collect()
}

/// Encode an assignment vector as a full XBEN two-delta frame.
///
/// The frame begins with a full-frame tag byte followed by RLE-encoded
/// assignment runs in big-endian format.
///
/// # Arguments
///
/// * `assignments` - The full assignment vector to encode.
///
/// # Returns
///
/// Returns the encoded frame as a byte vector.
fn encode_xben_twodelta_full_frame(assignments: &[u16]) -> Vec<u8> {
    let runs = assign_to_rle(assignments);
    let mut bytes = Vec::with_capacity(1 + 4 + runs.len() * 4);
    bytes.push(XBEN_TWODELTA_FULL_TAG);
    bytes.extend_from_slice(&(runs.len() as u32).to_be_bytes());
    for (value, len) in runs {
        bytes.extend_from_slice(&value.to_be_bytes());
        bytes.extend_from_slice(&len.to_be_bytes());
    }
    bytes
}

/// Encode the difference between two assignments as an XBEN two-delta delta frame.
///
/// The frame begins with a delta tag byte, the swapped value pair, and then
/// run-length encoded flip positions in big-endian format.
///
/// # Arguments
///
/// * `previous_assignment` - The previous assignment vector.
/// * `new_assignment` - The current assignment vector.
/// * `delta_pair` - An optional pre-computed pair of swapped values.
/// * `masks` - An optional index map from value to positions in the previous assignment.
///
/// # Returns
///
/// Returns the encoded delta frame as a byte vector, or an error if encoding fails.
fn encode_xben_twodelta_delta_frame(
    previous_assignment: &[u16],
    new_assignment: &[u16],
    delta_pair: Option<(u16, u16)>,
    masks: Option<&HashMap<u16, Vec<usize>>>,
) -> io::Result<Vec<u8>> {
    let (ordered_pair, run_lengths) =
        build_twodelta_runs_with_hint(previous_assignment, new_assignment, delta_pair, masks)?;
    let mut bytes = Vec::with_capacity(1 + 2 + 2 + 4 + run_lengths.len() * 2);
    bytes.push(XBEN_TWODELTA_DELTA_TAG);
    bytes.extend_from_slice(&ordered_pair.0.to_be_bytes());
    bytes.extend_from_slice(&ordered_pair.1.to_be_bytes());
    bytes.extend_from_slice(&(run_lengths.len() as u32).to_be_bytes());
    for run_length in run_lengths {
        bytes.extend_from_slice(&run_length.to_be_bytes());
    }
    Ok(bytes)
}

/// A struct to make the writing of BEN files easier and more ergonomic.
pub struct BenEncoder<W: Write> {
    writer: W,
    previous_sample: Vec<u16>,
    previous_masks: HashMap<u16, Vec<usize>>,
    previous_encoded_sample: Option<BufferedBenFrame>,
    sample_count: u16,
    variant: BenVariant,
    complete: bool,
}

impl<W: Write> BenEncoder<W> {
    /// Create a new BEN writer and immediately emit the BEN banner.
    ///
    /// # Arguments
    ///
    /// * `writer` - The destination that will receive the BEN stream.
    /// * `variant` - The BEN variant to encode.
    ///
    /// # Returns
    ///
    /// Returns a new encoder ready to accept assignments or RLE frames.
    pub fn new(mut writer: W, variant: BenVariant) -> Self {
        writer.write_all(banner_for_variant(variant)).unwrap();

        BenEncoder {
            writer,
            previous_sample: Vec::new(),
            previous_masks: HashMap::new(),
            previous_encoded_sample: None,
            sample_count: 0,
            complete: false,
            variant,
        }
    }

    /// Rebuild the value-to-position index map from the current previous sample.
    fn rebuild_previous_masks(&mut self) {
        self.previous_masks.clear();
        for (idx, &assignment) in self.previous_sample.iter().enumerate() {
            self.previous_masks.entry(assignment).or_default().push(idx);
        }
    }

    /// Store a new previous sample along with its encoded frame and repetition count.
    ///
    /// # Arguments
    ///
    /// * `sample` - The assignment vector to cache.
    /// * `encoded` - The already-encoded frame for this assignment.
    /// * `sample_count` - The initial repetition count for this sample.
    fn set_previous_sample(
        &mut self,
        sample: Vec<u16>,
        encoded: BufferedBenFrame,
        sample_count: u16,
    ) {
        self.previous_sample = sample;
        self.rebuild_previous_masks();
        self.previous_encoded_sample = Some(encoded);
        self.sample_count = sample_count;
    }

    /// Encode and write an assignment vector using pre-computed transition hints.
    ///
    /// The encoding strategy depends on the configured `BenVariant`. Repeated
    /// assignments may be deduplicated or counted, and two-delta hints enable
    /// compact delta frames when applicable.
    ///
    /// # Arguments
    ///
    /// * `assign_vec` - The assignment vector to encode.
    /// * `hints` - Pre-computed hints about repetition and delta-pair eligibility.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` after the assignment has been queued or written.
    fn write_assignment_with_hints(
        &mut self,
        assign_vec: Vec<u16>,
        hints: AssignmentHints,
    ) -> Result<()> {
        match self.variant {
            BenVariant::Standard => {
                let repeated = is_repeated_assignment(&self.previous_sample, &assign_vec);
                if hints.is_repeated {
                    if let Some(encoded) = self.previous_encoded_sample.as_ref() {
                        self.writer.write_all(encoded.as_slice())?;
                        self.previous_sample = assign_vec;
                        return Ok(());
                    }
                }

                if repeated {
                    if let Some(encoded) = self.previous_encoded_sample.as_ref() {
                        self.writer.write_all(encoded.as_slice())?;
                        self.previous_sample = assign_vec;
                        return Ok(());
                    }
                }

                let encoded = encode_ben_vec_from_assign(&assign_vec);
                self.writer.write_all(encoded.as_slice())?;
                self.set_previous_sample(assign_vec, BufferedBenFrame::Ben(encoded), 0);
                Ok(())
            }
            BenVariant::MkvChain => {
                if is_repeated_assignment(&self.previous_sample, &assign_vec) {
                    self.sample_count += 1;
                    return Ok(());
                }

                if self.sample_count > 0 {
                    self.flush_pending_frame()?;
                }

                let encoded = encode_ben_vec_from_assign(&assign_vec);
                self.set_previous_sample(assign_vec, BufferedBenFrame::Ben(encoded), 1);
                Ok(())
            }
            BenVariant::TwoDelta => {
                if self.previous_sample.is_empty() {
                    let encoded = encode_ben_vec_from_assign(&assign_vec);
                    self.set_previous_sample(assign_vec, BufferedBenFrame::Ben(encoded), 1);
                    return Ok(());
                }

                if hints.is_repeated {
                    self.sample_count += 1;
                    return Ok(());
                }

                let encoded = encode_twodelta_vec_with_hint(
                    &self.previous_sample,
                    &assign_vec,
                    hints.delta_pair,
                    Some(&self.previous_masks),
                )?;
                self.flush_pending_frame()?;

                if let Some(pair) = hints.delta_pair {
                    self.update_masks_for_delta(&assign_vec, pair);
                    self.previous_sample = assign_vec;
                } else {
                    self.previous_sample = assign_vec;
                    self.rebuild_previous_masks();
                }
                self.previous_encoded_sample = Some(BufferedBenFrame::TwoDelta(encoded));
                self.sample_count = 1;
                Ok(())
            }
        }
    }

    /// Flush the buffered frame and its repetition count to the underlying writer.
    ///
    /// For MkvChain and TwoDelta variants, the repetition count is appended
    /// after the encoded frame. This is a no-op when no samples are pending.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` once the pending frame has been written.
    fn flush_pending_frame(&mut self) -> Result<()> {
        if self.sample_count == 0 {
            return Ok(());
        }

        let encoded = self
            .previous_encoded_sample
            .as_ref()
            .expect("missing previous BEN frame");
        self.writer.write_all(encoded.as_slice())?;

        if matches!(self.variant, BenVariant::MkvChain | BenVariant::TwoDelta) {
            self.writer.write_all(&self.sample_count.to_be_bytes())?;
        }

        Ok(())
    }

    /// Record additional repetitions of the most recently written assignment.
    ///
    /// For MkvChain and TwoDelta variants the repetition count is incremented
    /// directly. For Standard, the cached encoded frame is re-emitted once per
    /// additional repeat.
    ///
    /// # Arguments
    ///
    /// * `additional` - The number of extra copies beyond the one already written.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` after all additional repeats have been recorded.
    pub fn repeat_previous(&mut self, additional: u16) -> Result<()> {
        match self.variant {
            BenVariant::Standard => {
                if let Some(encoded) = self.previous_encoded_sample.as_ref() {
                    for _ in 0..additional {
                        self.writer.write_all(encoded.as_slice())?;
                    }
                }
            }
            BenVariant::MkvChain | BenVariant::TwoDelta => {
                self.sample_count += additional;
            }
        }
        Ok(())
    }

    /// Update the value-to-position masks incrementally for a two-delta transition.
    ///
    /// Instead of rebuilding the entire mask HashMap, only the positions belonging
    /// to the two swapped values are repartitioned. This is O(pair_positions)
    /// rather than O(assignment_length).
    ///
    /// # Arguments
    ///
    /// * `new_sample` - The new assignment vector after the transition.
    /// * `pair` - The two values involved in the delta swap.
    fn update_masks_for_delta(&mut self, new_sample: &[u16], pair: (u16, u16)) {
        if pair.0 == pair.1 {
            return;
        }

        let pos_a = self.previous_masks.remove(&pair.0).unwrap_or_default();
        let pos_b = self.previous_masks.remove(&pair.1).unwrap_or_default();

        let mut new_a = Vec::with_capacity(pos_a.len() + pos_b.len());
        let mut new_b = Vec::with_capacity(pos_a.len() + pos_b.len());

        let (mut i, mut j) = (0, 0);
        while i < pos_a.len() || j < pos_b.len() {
            let pos = if j >= pos_b.len() || (i < pos_a.len() && pos_a[i] < pos_b[j]) {
                let p = pos_a[i];
                i += 1;
                p
            } else {
                let p = pos_b[j];
                j += 1;
                p
            };

            if new_sample[pos] == pair.0 {
                new_a.push(pos);
            } else {
                new_b.push(pos);
            }
        }

        if !new_a.is_empty() {
            self.previous_masks.insert(pair.0, new_a);
        }
        if !new_b.is_empty() {
            self.previous_masks.insert(pair.1, new_b);
        }
    }

    /// Encode and write a full assignment vector.
    ///
    /// # Arguments
    ///
    /// * `assign_vec` - The full assignment vector to encode.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` after the assignment has been queued or written.
    pub fn write_assignment(&mut self, assign_vec: Vec<u16>) -> Result<()> {
        let hints = if self.variant == BenVariant::TwoDelta {
            let masks = if self.previous_masks.is_empty() {
                None
            } else {
                Some(&self.previous_masks)
            };
            analyze_twodelta_transition(&self.previous_sample, &assign_vec, masks)
        } else {
            AssignmentHints::default()
        };
        self.write_assignment_with_hints(assign_vec, hints)
    }

    /// Encode and write a JSON assignment record.
    ///
    /// The input must contain an `assignment` array of integers. Other fields
    /// are ignored.
    ///
    /// # Arguments
    ///
    /// * `data` - A JSON object containing an `assignment` array.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` after the record has been validated and encoded.
    pub fn write_json_value(&mut self, data: Value) -> Result<()> {
        self.write_assignment(parse_json_assignment(data)?)
    }

    /// Flush any buffered repetition state to the underlying writer.
    ///
    /// This matters for [`BenVariant::MkvChain`], where repeated consecutive
    /// samples are emitted only once together with their repetition count.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` once any buffered repetition state has been flushed.
    pub fn finish(&mut self) -> Result<()> {
        if self.complete {
            return Ok(());
        }
        self.flush_pending_frame()
            .expect("Error while flushing trailing BEN frame");
        self.complete = true;
        Ok(())
    }
}

impl<W: Write> Drop for BenEncoder<W> {
    /// Flush any buffered BEN state during drop.
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

/// A struct to make the writing of XBEN files easier and more ergonomic.
pub struct XBenEncoder<W: Write> {
    encoder: XzEncoder<W>,
    previous_assignment: Vec<u16>,
    previous_masks: HashMap<u16, Vec<usize>>,
    previous_frame: Vec<u8>,
    count: u16,
    variant: BenVariant,
}

impl<W: Write> XBenEncoder<W> {
    /// Rebuild the value-to-position index map from the current previous assignment.
    fn rebuild_previous_masks(&mut self) {
        self.previous_masks.clear();
        for (idx, &assignment) in self.previous_assignment.iter().enumerate() {
            self.previous_masks.entry(assignment).or_default().push(idx);
        }
    }

    /// Store a new previous assignment along with its encoded frame and repetition count.
    ///
    /// # Arguments
    ///
    /// * `assignment` - The assignment vector to cache.
    /// * `frame` - The already-encoded frame bytes for this assignment.
    /// * `count` - The initial repetition count for this assignment.
    fn set_previous_assignment(&mut self, assignment: Vec<u16>, frame: Vec<u8>, count: u16) {
        self.previous_assignment = assignment;
        self.rebuild_previous_masks();
        self.previous_frame = frame;
        self.count = count;
    }

    /// Update the value-to-position masks incrementally for a two-delta transition.
    ///
    /// Instead of rebuilding the entire mask HashMap, only the positions belonging
    /// to the two swapped values are repartitioned. This is O(pair_positions)
    /// rather than O(assignment_length).
    ///
    /// # Arguments
    ///
    /// * `new_sample` - The new assignment vector after the transition.
    /// * `pair` - The two values involved in the delta swap.
    fn update_masks_for_delta(&mut self, new_sample: &[u16], pair: (u16, u16)) {
        if pair.0 == pair.1 {
            return;
        }

        let pos_a = self.previous_masks.remove(&pair.0).unwrap_or_default();
        let pos_b = self.previous_masks.remove(&pair.1).unwrap_or_default();

        let mut new_a = Vec::with_capacity(pos_a.len() + pos_b.len());
        let mut new_b = Vec::with_capacity(pos_a.len() + pos_b.len());

        let (mut i, mut j) = (0, 0);
        while i < pos_a.len() || j < pos_b.len() {
            let pos = if j >= pos_b.len() || (i < pos_a.len() && pos_a[i] < pos_b[j]) {
                let p = pos_a[i];
                i += 1;
                p
            } else {
                let p = pos_b[j];
                j += 1;
                p
            };
            if new_sample[pos] == pair.0 {
                new_a.push(pos);
            } else {
                new_b.push(pos);
            }
        }

        if !new_a.is_empty() {
            self.previous_masks.insert(pair.0, new_a);
        }
        if !new_b.is_empty() {
            self.previous_masks.insert(pair.1, new_b);
        }
    }

    /// Flush the buffered frame and its repetition count to the XZ encoder.
    ///
    /// For MkvChain and TwoDelta variants, the repetition count is appended
    /// after the encoded frame. This is a no-op when no samples are pending.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` once the pending frame has been written.
    fn flush_pending_frame(&mut self) -> Result<()> {
        if self.count == 0 {
            return Ok(());
        }

        self.encoder.write_all(&self.previous_frame)?;
        if matches!(self.variant, BenVariant::MkvChain | BenVariant::TwoDelta) {
            self.encoder.write_all(&self.count.to_be_bytes())?;
        }
        Ok(())
    }

    /// Create a new XBEN writer around an already-configured XZ encoder.
    ///
    /// # Arguments
    ///
    /// * `encoder` - The configured XZ encoder that will receive the ben32
    ///   payload.
    /// * `variant` - The BEN variant to encode inside the compressed stream.
    ///
    /// # Returns
    ///
    /// Returns a new XBEN encoder ready to accept assignments or BEN frames.
    pub fn new(mut encoder: XzEncoder<W>, variant: BenVariant) -> Self {
        encoder.write_all(banner_for_variant(variant)).unwrap();
        match variant {
            BenVariant::Standard => XBenEncoder {
                encoder,
                previous_assignment: Vec::new(),
                previous_masks: HashMap::new(),
                previous_frame: Vec::new(),
                count: 0,
                variant: BenVariant::Standard,
            },
            BenVariant::MkvChain => XBenEncoder {
                encoder,
                previous_assignment: Vec::new(),
                previous_masks: HashMap::new(),
                previous_frame: Vec::new(),
                count: 0,
                variant: BenVariant::MkvChain,
            },
            BenVariant::TwoDelta => XBenEncoder {
                encoder,
                previous_assignment: Vec::new(),
                previous_masks: HashMap::new(),
                previous_frame: Vec::new(),
                count: 0,
                variant: BenVariant::TwoDelta,
            },
        }
    }

    /// Encode and write a full assignment vector into the compressed XBEN stream.
    ///
    /// # Arguments
    ///
    /// * `assign_vec` - The full assignment vector to encode.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` after the assignment has been queued or written.
    pub fn write_assignment(&mut self, assign_vec: Vec<u16>) -> Result<()> {
        match self.variant {
            BenVariant::Standard => {
                let encoded = encode_ben32_assignments(&assign_vec)?.into_u8_vec()?;
                self.encoder.write_all(&encoded)?;
                self.previous_assignment = assign_vec;
                self.previous_frame = encoded;
                Ok(())
            }
            BenVariant::MkvChain => {
                if is_repeated_assignment(&self.previous_assignment, &assign_vec) {
                    self.count += 1;
                    return Ok(());
                }

                self.flush_pending_frame()?;
                let encoded = encode_ben32_assignments(&assign_vec)?.into_u8_vec()?;
                self.set_previous_assignment(assign_vec, encoded, 1);
                Ok(())
            }
            BenVariant::TwoDelta => {
                if self.previous_assignment.is_empty() {
                    let encoded = encode_xben_twodelta_full_frame(&assign_vec);
                    self.set_previous_assignment(assign_vec, encoded, 1);
                    return Ok(());
                }

                let masks = if self.previous_masks.is_empty() {
                    None
                } else {
                    Some(&self.previous_masks)
                };
                let hints =
                    analyze_twodelta_transition(&self.previous_assignment, &assign_vec, masks);
                if hints.is_repeated {
                    self.count += 1;
                    return Ok(());
                }

                let encoded = encode_xben_twodelta_delta_frame(
                    &self.previous_assignment,
                    &assign_vec,
                    hints.delta_pair,
                    Some(&self.previous_masks),
                )?;
                self.flush_pending_frame()?;

                if let Some(pair) = hints.delta_pair {
                    self.update_masks_for_delta(&assign_vec, pair);
                    self.previous_assignment = assign_vec;
                } else {
                    self.previous_assignment = assign_vec;
                    self.rebuild_previous_masks();
                }
                self.previous_frame = encoded;
                self.count = 1;
                Ok(())
            }
        }
    }

    /// Encode and write a JSON assignment record into the compressed XBEN stream.
    ///
    /// # Arguments
    ///
    /// * `data` - A JSON object containing an `assignment` array.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` after the record has been validated and encoded.
    pub fn write_json_value(&mut self, data: Value) -> Result<()> {
        self.write_assignment(parse_json_assignment(data)?)
    }

    /// Read BEN frames from `reader` and write them into this XBEN stream.
    ///
    /// If the source still contains the 17-byte BEN banner, it is consumed and
    /// replaced by the banner already written by this encoder.
    ///
    /// # Arguments
    ///
    /// * `reader` - The BEN input stream, with or without its banner.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` after the BEN stream has been translated into XBEN.
    pub fn write_ben_file(&mut self, mut reader: impl BufRead) -> Result<()> {
        let peek = reader.fill_buf()?;
        let has_banner = peek.len() >= BANNER_LEN && has_known_banner_prefix(peek);

        if has_banner {
            if self.variant == BenVariant::TwoDelta {
                let mut banner = [0u8; BANNER_LEN];
                banner.copy_from_slice(&peek[..BANNER_LEN]);
                reader.consume(BANNER_LEN);

                let decoder =
                    BenDecoder::new(io::Cursor::new(banner).chain(reader))?.silent(true);
                for record in decoder {
                    let (assignment, count) = record?;
                    self.write_assignment(assignment)?;
                    if count > 1 {
                        self.count += count - 1;
                    }
                }
                return Ok(());
            }
            reader.consume(BANNER_LEN);
        }

        if self.variant == BenVariant::TwoDelta {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "TwoDelta XBEN translation requires a BEN stream with its banner",
            ));
        }

        ben_to_ben32_lines(&mut reader, &mut self.encoder, self.variant)
    }
}

impl<W: Write> Drop for XBenEncoder<W> {
    /// Flush any buffered XBEN repetition state during drop.
    fn drop(&mut self) {
        if matches!(self.variant, BenVariant::MkvChain | BenVariant::TwoDelta) && self.count > 0 {
            self.flush_pending_frame()
                .expect("Error writing last XBEN frame to file");
        }
    }
}
