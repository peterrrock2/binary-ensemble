use super::errors::BenEncodeError;
use crate::codec::frames::TwoDeltaFrame;
use std::collections::HashMap;
use std::io::{Error, ErrorKind, Result};

impl TwoDeltaFrame {
    /// Build a TwoDelta frame by packing a run-length vector into the binary format.
    ///
    /// Run lengths are packed at `max_len_bit_count` bits per value (the minimum
    /// bit width needed to represent the largest run length), MSB-first with no
    /// padding between values. If the total bit count is not a multiple of 8, the
    /// final byte is zero-padded on the right.
    ///
    /// The serialized layout is:
    /// ```text
    /// [pair.0: u16 BE][pair.1: u16 BE][max_len_bit_count: u8][n_bytes: u32 BE][payload...]
    /// ```
    /// where the payload is the bit-packed run lengths.
    ///
    /// # Arguments
    ///
    /// * `pair` - The ordered pair of assignment ids. `pair.0` corresponds to the first run.
    /// * `run_length_vector` - The lengths of alternating runs of `pair.0` and `pair.1`
    ///   over the positions occupied by the pair, in position order.
    ///
    /// # Returns
    ///
    /// A fully serialized `TwoDeltaFrame` with both the packed `raw_bytes` and the
    /// original `run_length_vector` stored on the struct.
    pub fn from_run_lengths(pair: (u16, u16), run_length_vector: Vec<u16>) -> Self {
        let max_len = run_length_vector.iter().copied().max().unwrap_or(0);
        let max_len_bit_count = (16 - max_len.leading_zeros() as u8).max(1);

        let payload_bits = max_len_bit_count as u32 * run_length_vector.len() as u32;
        let n_bytes = payload_bits.div_ceil(8);

        // pair_bytes (4) + max_len_bit_count (1) + n_bytes (4) + payload (n_bytes)
        let mut raw_bytes = Vec::with_capacity((n_bytes + 9) as usize);
        raw_bytes.extend_from_slice(&pair.0.to_be_bytes());
        raw_bytes.extend_from_slice(&pair.1.to_be_bytes());
        raw_bytes.push(max_len_bit_count);
        raw_bytes.extend_from_slice(&n_bytes.to_be_bytes());

        let mut remainder: u32 = 0;
        let mut remainder_bits: u8 = 0;

        for &item in &run_length_vector {
            let mut packed = (remainder << max_len_bit_count) | item as u32;
            let mut bits_left = remainder_bits + max_len_bit_count;

            while bits_left >= 8 {
                bits_left -= 8;
                raw_bytes.push((packed >> bits_left) as u8);
                packed &= !((u32::MAX) << bits_left);
            }

            remainder = packed;
            remainder_bits = bits_left;
        }

        if remainder_bits > 0 {
            raw_bytes.push((remainder << (8 - remainder_bits)) as u8);
        }

        Self {
            pair,
            max_len_bit_count,
            n_bytes,
            run_length_vector,
            raw_bytes,
        }
    }

    /// Reconstruct a TwoDelta frame from already-parsed header fields and a raw payload.
    ///
    /// This is the inverse of `from_run_lengths`: it re-assembles the serialized bytes
    /// and decodes the bit-packed payload back into the run-length vector so that both
    /// representations are available on the resulting frame.
    ///
    /// The decoding reads `max_len_bit_count` bits at a time from the payload, MSB-first,
    /// and discards any trailing zero-valued items produced by right-padding in the final byte.
    ///
    /// # Arguments
    ///
    /// * `pair` - The ordered pair of assignment ids as read from the frame header.
    /// * `max_len_bit_count` - The bit width of each packed run length, as read from the
    ///   frame header.
    /// * `payload` - The raw packed payload bytes, not including the 9-byte header.
    ///
    /// # Returns
    ///
    /// A `TwoDeltaFrame` with both `raw_bytes` (header + payload) and the decoded
    /// `run_length_vector` populated.
    pub fn from_parts(pair: (u16, u16), max_len_bit_count: u8, payload: Vec<u8>) -> Self {
        let n_bytes = payload.len() as u32;
        let mut raw_bytes = Vec::with_capacity(9 + payload.len());
        raw_bytes.extend_from_slice(&pair.0.to_be_bytes());
        raw_bytes.extend_from_slice(&pair.1.to_be_bytes());
        raw_bytes.push(max_len_bit_count);
        raw_bytes.extend_from_slice(&n_bytes.to_be_bytes());
        raw_bytes.extend_from_slice(&payload);

        let mut run_length_vector = Vec::new();
        let mut buffer: u32 = 0;
        let mut n_bits_in_buff: u16 = 0;
        let mut current: Option<u16> = None;

        for byte in payload {
            buffer |= (byte as u32).to_be() >> n_bits_in_buff;
            n_bits_in_buff += 8;

            if n_bits_in_buff >= max_len_bit_count as u16 && current.is_none() {
                current = Some((buffer >> (32 - max_len_bit_count)) as u16);
                buffer <<= max_len_bit_count;
                n_bits_in_buff -= max_len_bit_count as u16;
            }

            if let Some(item) = current.take() {
                if item > 0 {
                    run_length_vector.push(item);
                }
            }

            while n_bits_in_buff >= max_len_bit_count as u16 {
                let item = (buffer >> (32 - max_len_bit_count)) as u16;
                buffer <<= max_len_bit_count;
                n_bits_in_buff -= max_len_bit_count as u16;
                if item > 0 {
                    run_length_vector.push(item);
                }
            }
        }

        Self {
            pair,
            max_len_bit_count,
            n_bytes,
            run_length_vector,
            raw_bytes,
        }
    }
}

/// Encode a transition between two assignment vectors as a TwoDelta frame, optionally
/// using caller-supplied hints to accelerate encoding.
///
/// # Arguments
///
/// * `previous_assignment` - The full assignment vector from the preceding sample.
/// * `new_assignment` - The full assignment vector for the sample being encoded.
/// * `delta_pair` - An optional hint asserting which pair of ids is involved in the
///   transition. Must be provided together with `masks`, and the two ids must be distinct.
/// * `masks` - An optional mutable map from assignment id to the sorted list of positions
///   it occupies in `previous_assignment`. When provided, the map is updated in-place to
///   reflect `new_assignment` before returning.
///
/// # Returns
///
/// A `TwoDeltaFrame` describing the transition from `previous_assignment` to
/// `new_assignment`.
///
/// # TwoDelta encoding
///
/// A TwoDelta frame is valid only when every position that changes between
/// `previous_assignment` and `new_assignment` involves exactly two assignment ids
/// (call them A and B), and no position outside that pair changes. The frame stores
/// the pair and the lengths of alternating runs of A and B over the positions
/// occupied by the pair, ordered by position. The first run always corresponds to
/// whichever id occupies the lowest-indexed position.
///
/// # Hints
///
/// Two optional hints can be provided to avoid scanning the full assignment vector:
///
/// - `delta_pair`: The caller asserts that exactly this pair of ids is involved in
///   the transition. Must be provided together with `masks`. The pair must have two
///   distinct ids — passing `(x, x)` is an error.
///
/// - `masks`: A mutable map from assignment id to the sorted list of positions it
///   occupies in `previous_assignment`. When provided, the function reads positions
///   directly from the map instead of scanning the assignment vector, and updates
///   the map in-place to reflect `new_assignment` before returning. The masks must
///   cover every id that appears in the pair; a missing or empty entry is an error.
///
/// The hints are not independent: `delta_pair` requires `masks`. Providing `masks`
/// without `delta_pair` is allowed — the function will infer the pair from the first
/// differing position and then use the masks from there.
///
/// When no hints are provided the function falls back to a full scan of both
/// assignment vectors.
///
/// # Errors
///
/// Returns an error if:
/// - The assignment vectors have different lengths.
/// - `delta_pair` is provided without `masks`.
/// - `delta_pair` contains two identical ids.
/// - A mask entry required by the pair is absent or empty.
/// - A position referenced by a mask holds a value outside the pair.
/// - The transition involves more than two distinct ids.
/// - The two assignments are identical (returns `BenEncodeError::RepeatedSample`).
pub(crate) fn encode_twodelta_frame_with_hint(
    previous_assignment: impl AsRef<[u16]>,
    new_assignment: impl AsRef<[u16]>,
    delta_pair: Option<(u16, u16)>,
    masks: Option<&mut HashMap<u16, Vec<usize>>>,
) -> Result<TwoDeltaFrame> {
    let previous_assignment = previous_assignment.as_ref();
    let new_assignment = new_assignment.as_ref();

    if previous_assignment.len() != new_assignment.len() {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!(
                "TwoDelta requires previous and new assignment vectors to be of \
                equal length, but got lengths {} and {}",
                previous_assignment.len(),
                new_assignment.len()
            ),
        ));
    }

    if delta_pair.is_some() {
        if masks.is_none() {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "TwoDelta pair hint provided without corresponding masks",
            ));
        }
        let pair = delta_pair.unwrap();
        if pair.0 == pair.1 {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "TwoDelta pair hint cannot have identical values for the two ids",
            ));
        }
    }

    match (delta_pair, masks) {
        (Some(pair), Some(masks)) => construct_twodelta_frame_from_pair_and_mask_hints(
            previous_assignment,
            new_assignment,
            pair,
            masks,
        ),
        (None, Some(masks)) => {
            construct_twodelta_frame_from_mask_hint(previous_assignment, new_assignment, masks)
        }
        _ => construct_twodelta_frame_from_scratch(previous_assignment, new_assignment),
    }

    // Ok(TwoDeltaFrame::from_run_lengths(ordered_pair, run_lengths))
}

/// Validate that `masks` contains non-empty entries for both ids in `pair` and return
/// the pair ordered so that `pair.0` occupies a lower index than `pair.1`.
///
/// Ordering by first position ensures that the run-length sequence produced during
/// encoding always begins with the id whose positions come first in the assignment
/// vector, which is required for deterministic round-trip decoding.
///
/// # Arguments
///
/// * `pair` - The two assignment ids to validate and order.
/// * `masks` - The position mask map to look up entries in.
///
/// # Returns
///
/// The pair reordered so that `pair.0` has a smaller first position than `pair.1`,
/// or an error if either id is absent from `masks` or has an empty position list.
fn validate_masks_and_order_pairs_for_twodelta(
    pair: (u16, u16),
    masks: &HashMap<u16, Vec<usize>>,
) -> Result<(u16, u16)> {
    let mask_a = match masks.get(&pair.0) {
        Some(m) => m,
        None => {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "TwoDelta pair mask is missing for the previous assignment",
            ))
        }
    };

    let mask_b = match masks.get(&pair.1) {
        Some(m) => m,
        None => {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "TwoDelta pair mask is missing for the current assignment",
            ))
        }
    };

    if mask_a.len() == 0 {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("TwoDelta pair mask for the id {} is empty", pair.0),
        ));
    };

    if mask_b.len() == 0 {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("TwoDelta pair mask for the id {} is empty", pair.1),
        ));
    };

    if mask_a[0] < mask_b[0] {
        Ok((pair.0, pair.1))
    } else {
        Ok((pair.1, pair.0))
    }
}

/// Build a TwoDelta frame using both a known pair and pre-computed position masks.
///
/// This is the fast path used during recombination-aware encoding, where the caller
/// already knows which two ids are swapping and has maintained a mask for each id.
///
/// The function merges the two sorted position lists from `masks` to produce the
/// interleaved sequence of positions, validates that every referenced position in
/// `previous` and `current` belongs to the pair, computes the run lengths over
/// `current`, and then updates `masks` in-place to reflect the new positions of
/// each id in `current`.
///
/// # Arguments
///
/// * `previous` - The full assignment vector from the preceding sample.
/// * `current` - The full assignment vector for the sample being encoded.
/// * `delta_pair` - The pair of ids asserted to be involved in the transition.
/// * `masks` - Mutable position mask map for both ids in the pair. Updated in-place
///   to reflect `current` before returning.
///
/// # Returns
///
/// A `TwoDeltaFrame` for the transition, or `BenEncodeError::RepeatedSample` if no
/// position actually changed value (signalling the frame can be deduplicated), or
/// another error if a mask entry is inconsistent with the assignment data.
fn construct_twodelta_frame_from_pair_and_mask_hints(
    previous: &[u16],
    current: &[u16],
    delta_pair: (u16, u16),
    masks: &mut HashMap<u16, Vec<usize>>,
) -> Result<TwoDeltaFrame> {
    let pair = match validate_masks_and_order_pairs_for_twodelta(delta_pair, masks) {
        Ok(pair) => pair,
        Err(e) => {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!(
                    "Encountered when validating masks and ordering pairs in \
                    `determine_twodelta_run_from_pair_and_mask_hints`:\n{}",
                    e
                ),
            ));
        }
    };

    let mask_a = masks
        .get(&pair.0)
        .expect("Failed to get mask for pair.0 after validation");
    let mask_b = masks
        .get(&pair.1)
        .expect("Failed to get mask for pair.1 after validation");

    let new_capacity = mask_a.len() + mask_b.len();
    let mut run_lengths = Vec::with_capacity(new_capacity);
    // Accumulate updated masks reflecting positions in `current`.
    let mut new_mask_a = Vec::with_capacity(new_capacity);
    let mut new_mask_b = Vec::with_capacity(new_capacity);

    // Two-pointer merge over the sorted position lists. `current_value` tracks
    // which id owns the active run; `current_mask_count` is the length of that run.
    let (mut i, mut j) = (0usize, 0usize);
    let mut current_mask_count = 0u16;
    let mut current_value = pair.0;

    let mut found_assignment_change = false;

    while i < mask_a.len() || j < mask_b.len() {
        // Pick the next position from whichever mask is lower, mirroring the
        // merge step used when building pair_positions from two masks.
        let idx = if j == mask_b.len() || (i < mask_a.len() && mask_a[i] < mask_b[j]) {
            if current_value != pair.0 {
                run_lengths.push(current_mask_count);
                current_mask_count = 1;
                current_value = pair.0;
            } else {
                current_mask_count += 1;
            }
            i += 1;
            mask_a[i - 1]
        } else {
            if current_value != pair.1 {
                run_lengths.push(current_mask_count);
                current_mask_count = 1;
                current_value = pair.1;
            } else {
                current_mask_count += 1;
            }
            j += 1;
            mask_b[j - 1]
        };

        let previous_value = previous[idx];
        let current_value = current[idx];

        if previous_value != pair.0 && previous_value != pair.1 {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "TwoDelta pair mask referenced an index outside the selected id pair",
            ));
        }
        if current_value != pair.0 && current_value != pair.1 {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "TwoDelta payload encountered an assignment outside the selected id pair",
            ));
        }
        if current_value != previous_value {
            found_assignment_change = true;
        }

        if current_value == pair.0 {
            new_mask_a.push(idx);
        } else {
            new_mask_b.push(idx);
        }
    }
    run_lengths.push(current_mask_count);

    // Special error that signals that we can reuse the last TwoDelta frame
    if !found_assignment_change {
        return Err(BenEncodeError::RepeatedSample.into());
    }

    masks.insert(pair.0, new_mask_a);
    masks.insert(pair.1, new_mask_b);
    Ok(TwoDeltaFrame::from_run_lengths(pair, run_lengths))
}

/// Build a TwoDelta frame using only pre-computed position masks, inferring the pair
/// from the first differing position between `previous` and `current`.
///
/// Scans until it finds a position where the two assignments differ, then delegates
/// to `construct_twodelta_frame_from_pair_and_mask_hints` with that pair. If no
/// difference is found the assignments are identical and
/// `BenEncodeError::RepeatedSample` is returned.
///
/// # Arguments
///
/// * `previous` - The full assignment vector from the preceding sample.
/// * `current` - The full assignment vector for the sample being encoded.
/// * `masks` - Mutable position mask map covering all ids that may appear in the pair.
///   Updated in-place to reflect `current` before returning.
///
/// # Returns
///
/// A `TwoDeltaFrame` for the transition, or `BenEncodeError::RepeatedSample` if the
/// two assignments are identical.
fn construct_twodelta_frame_from_mask_hint(
    previous: &[u16],
    current: &[u16],
    masks: &mut HashMap<u16, Vec<usize>>,
) -> Result<TwoDeltaFrame> {
    for (&assign0, &assign1) in previous.iter().zip(current.iter()) {
        if assign0 != assign1 {
            return construct_twodelta_frame_from_pair_and_mask_hints(
                previous,
                current,
                (assign0, assign1),
                masks,
            );
        }
    }

    return Err(BenEncodeError::RepeatedSample.into());
}

/// Build a TwoDelta frame by scanning both assignment vectors from scratch, with no
/// hints from the caller.
///
/// Simultaneously discovers the pair and computes run lengths in a single pass over
/// the zipped assignments. Only positions where the two assignments differ are
/// considered; unchanged positions are skipped entirely. The pair is ordered so that
/// the first id encountered in `current` at a changed position becomes `pair.0`,
/// which ensures the run-length sequence begins with the id that appears first.
///
/// # Arguments
///
/// * `previous` - The full assignment vector from the preceding sample.
/// * `current` - The full assignment vector for the sample being encoded.
///
/// # Returns
///
/// A `TwoDeltaFrame` for the transition, or an error if more than two distinct ids
/// appear across all changed positions.
fn construct_twodelta_frame_from_scratch(
    previous: &[u16],
    current: &[u16],
) -> Result<TwoDeltaFrame> {
    let mut delta_pair = [0u16; 2];
    let mut pair_len = 0usize;

    let mut run_lengths = Vec::new();
    let mut current_value = 0u16;
    let mut current_run_length = 0u16;

    for (&assign0, &assign1) in previous.iter().zip(current.iter()) {
        if assign0 != assign1 {
            // We are encoding the current, so the first value we encounter in the current should
            // be added to the front of the pair
            for value in [assign1, assign0] {
                if !delta_pair[..pair_len].contains(&value) {
                    // We have found both values for the pair and yet encountered a third value
                    // so this is not a valid TwoDelta transition.
                    if pair_len == 2 {
                        return Err(Error::new(
                            ErrorKind::InvalidData,
                            "TwoDelta transitions may involve at most two assignment ids",
                        ));
                    }
                    delta_pair[pair_len] = value;
                    pair_len += 1;
                }
            }
            if current_run_length > 0 && current_value != assign1 {
                run_lengths.push(current_run_length);
                current_run_length = 1;
                current_value = assign1;
            } else {
                current_run_length += 1;
            }
        }
    }
    run_lengths.push(current_run_length);

    Ok(TwoDeltaFrame::from_run_lengths(
        (delta_pair[0], delta_pair[1]),
        run_lengths,
    ))
}

/// Encode a transition between two assignment vectors as a TwoDelta frame.
///
/// This is the unhinted entry point. It falls back to a full scan of both
/// assignment vectors to discover the pair and compute run lengths. Prefer
/// `encode_twodelta_frame_with_hint` when masks are available, as it avoids
/// the scan entirely.
///
/// The transition is valid only when all changed positions involve exactly two
/// assignment ids and positions outside that pair remain unchanged.
///
/// # Arguments
///
/// * `previous_assignment` - The previous full assignment vector.
/// * `new_assignment` - The next full assignment vector.
///
/// # Returns
///
/// Returns a TwoDelta frame describing the transition, or an error if the
/// transition involves more than two ids or the assignments are identical.
pub fn encode_twodelta_frame(
    previous_assignment: impl AsRef<[u16]>,
    new_assignment: impl AsRef<[u16]>,
) -> Result<TwoDeltaFrame> {
    encode_twodelta_frame_with_hint(previous_assignment, new_assignment, None, None)
}
