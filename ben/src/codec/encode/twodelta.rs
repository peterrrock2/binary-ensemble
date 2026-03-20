use super::errors::EncodeError;
use crate::codec::frames::TwoDeltaEncodeFrame;
use std::collections::HashMap;
use std::io::{Error, ErrorKind, Result};

/// Encode a transition between two assignment vectors as a TwoDelta frame, optionally
/// using caller-supplied hints to accelerate encoding.
///
/// # Arguments
///
/// * `previous_assignment` - The full assignment vector from the preceding sample.
/// * `new_assignment` - The full assignment vector for the sample being encoded.
/// * `delta_pair` - An optional hint asserting which pair of ids is involved in the
///   transition. Must be provided together with `previous_masks`, and the two ids must be distinct.
/// * `previous_masks` - An optional mutable map from assignment id to the sorted list of positions
///   it occupies in `previous_assignment`. When provided, the map is updated in-place to
///   reflect `new_assignment` before returning.
///
/// # Returns
///
/// A `TwoDeltaEncodeFrame` describing the transition from `previous_assignment` to
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
///   the transition. Must be provided together with `previous_masks`. The pair must have two
///   distinct ids — passing `(x, x)` is an error.
///
/// - `previous_masks`: A mutable map from assignment id to the sorted list of positions it
///   occupies in `previous_assignment`. When provided, the function reads positions
///   directly from the map instead of scanning the assignment vector, and updates
///   the map in-place to reflect `new_assignment` before returning. The previous_masks must
///   cover every id that appears in the pair; a missing or empty entry is an error.
///
/// The hints are not independent: `delta_pair` requires `previous_masks`. Providing `previous_masks`
/// without `delta_pair` is allowed — the function will infer the pair from the first
/// differing position and then use the previous_masks from there.
///
/// When no hints are provided the function falls back to a full scan of both
/// assignment vectors.
///
/// # Errors
///
/// Returns an error if:
/// - The assignment vectors have different lengths.
/// - `delta_pair` is provided without `previous_masks`.
/// - `delta_pair` contains two identical ids.
/// - A mask entry required by the pair is absent or empty.
/// - A position referenced by a mask holds a value outside the pair.
/// - The transition involves more than two distinct ids.
/// - The two assignments are identical (returns `BenEncodeError::RepeatedSample`).
pub(crate) fn encode_twodelta_frame_with_hint(
    previous_assignment: impl AsRef<[u16]>,
    new_assignment: impl AsRef<[u16]>,
    delta_pair: Option<(u16, u16)>,
    previous_masks: Option<&mut HashMap<u16, Vec<usize>>>,
) -> Result<TwoDeltaEncodeFrame> {
    let previous_assignment = previous_assignment.as_ref();
    let new_assignment = new_assignment.as_ref();

    if previous_assignment.len() != new_assignment.len() {
        return Err(Error::from(EncodeError::TwoDeltaLengthMismatch {
            prev_len: previous_assignment.len(),
            new_len: new_assignment.len(),
        }));
    }

    if delta_pair.is_some() {
        if previous_masks.is_none() {
            return Err(Error::from(EncodeError::TwoDeltaHintWithoutMasks));
        }
        let pair = delta_pair.unwrap();
        if pair.0 == pair.1 {
            return Err(Error::from(EncodeError::TwoDeltaIdenticalPairHint {
                value: pair.0,
            }));
        }
    }

    match (delta_pair, previous_masks) {
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

    // Ok(TwoDeltaEncodeFrame::from_run_lengths(ordered_pair, run_lengths))
}

/// Validate that `previous_masks` contains non-empty entries for both ids in `pair` and return
/// the pair ordered so that `pair.0` occupies a lower index than `pair.1`.
///
/// Ordering by first position ensures that the run-length sequence produced during
/// encoding always begins with the id whose positions come first in the assignment
/// vector, which is required for deterministic round-trip decoding.
///
/// # Arguments
///
/// * `pair` - The two assignment ids to validate and order.
/// * `previous_masks` - The position mask map to look up entries in.
///
/// # Returns
///
/// The pair reordered so that `pair.0` has a smaller first position in the current vector than
/// `pair.1`, or an error if either id is absent from `previous_masks` or has an empty position list.
fn validate_masks_and_order_pairs_for_twodelta(
    pair: (u16, u16),
    masks: &HashMap<u16, Vec<usize>>,
    current: &[u16],
) -> Result<(u16, u16)> {
    let mask_a = match masks.get(&pair.0) {
        Some(m) => m,
        None => return Err(Error::from(EncodeError::TwoDeltaMissingMask { id: pair.0 })),
    };

    let mask_b = match masks.get(&pair.1) {
        Some(m) => m,
        None => return Err(Error::from(EncodeError::TwoDeltaMissingMask { id: pair.1 })),
    };

    if mask_a.len() == 0 {
        return Err(Error::from(EncodeError::TwoDeltaEmptyMask { id: pair.0 }));
    };

    if mask_b.len() == 0 {
        return Err(Error::from(EncodeError::TwoDeltaEmptyMask { id: pair.1 }));
    };

    // Order so that pair.0 is the value the new assignment places at the first
    // pair position (the lowest index held by either mask).  This guarantees
    // run_lengths[0] >= 1 with no leading-zero sentinel.
    let first_pos = mask_a[0].min(mask_b[0]);
    if current[first_pos] == pair.0 {
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
/// The function merges the two sorted position lists from `previous_masks` to produce the
/// interleaved sequence of positions, validates that every referenced position in
/// `previous` and `current` belongs to the pair, computes the run lengths over
/// `current`, and then updates `previous_masks` in-place to reflect the new positions of
/// each id in `current`.
///
/// # Arguments
///
/// * `previous` - The full assignment vector from the preceding sample.
/// * `current` - The full assignment vector for the sample being encoded.
/// * `delta_pair` - The pair of ids asserted to be involved in the transition.
/// * `previous_masks` - Mutable position mask map for both ids in the pair. Updated in-place
///   to reflect `current` before returning.
///
/// # Returns
///
/// A `TwoDeltaEncodeFrame` for the transition, or `BenEncodeError::RepeatedSample` if no
/// position actually changed value (signalling the frame can be deduplicated), or
/// another error if a mask entry is inconsistent with the assignment data.
fn construct_twodelta_frame_from_pair_and_mask_hints(
    previous: &[u16],
    current: &[u16],
    delta_pair: (u16, u16),
    previous_masks: &mut HashMap<u16, Vec<usize>>,
) -> Result<TwoDeltaEncodeFrame> {
    let pair =
        match validate_masks_and_order_pairs_for_twodelta(delta_pair, previous_masks, current) {
            Ok(pair) => pair,
            Err(e) => {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    format!(
                        "Encountered when validating previous_masks and ordering pairs in \
                    `determine_twodelta_run_from_pair_and_mask_hints`:\n{}",
                        e
                    ),
                ));
            }
        };

    let mask_a = previous_masks
        .get(&pair.0)
        .expect("Failed to get mask for pair.0 after validation");
    let mask_b = previous_masks
        .get(&pair.1)
        .expect("Failed to get mask for pair.1 after validation");

    let new_capacity = mask_a.len() + mask_b.len();
    let mut run_lengths = Vec::with_capacity(new_capacity);
    // Accumulate updated masks reflecting positions in `current`.
    let mut new_mask_a = Vec::with_capacity(new_capacity);
    let mut new_mask_b = Vec::with_capacity(new_capacity);

    let (mut i, mut j) = (0usize, 0usize);
    // pair.0 is guaranteed to equal current[first_pos] by validate_masks_and_order_pairs_for_twodelta,
    // so the first iteration always hits the `new_val == run_value` branch and increments
    // the count — no special-case initialization needed.
    let mut run_value = pair.0;
    let mut current_mask_count = 0u16;
    let mut found_assignment_change = false;

    while i < mask_a.len() || j < mask_b.len() {
        // Pick the next position from whichever mask is lower.
        let idx = if j == mask_b.len() || (i < mask_a.len() && mask_a[i] < mask_b[j]) {
            i += 1;
            mask_a[i - 1]
        } else {
            j += 1;
            mask_b[j - 1]
        };

        let previous_value = previous[idx];
        let new_val = current[idx];

        if previous_value != pair.0 && previous_value != pair.1 {
            return Err(Error::from(EncodeError::TwoDeltaMaskOutOfPair {
                pos: idx,
                actual: previous_value,
                a: pair.0,
                b: pair.1,
            }));
        }
        if new_val != pair.0 && new_val != pair.1 {
            return Err(Error::from(EncodeError::TwoDeltaMaskOutOfPair {
                pos: idx,
                actual: new_val,
                a: pair.0,
                b: pair.1,
            }));
        }
        if new_val != previous_value {
            found_assignment_change = true;
        }

        if new_val == run_value {
            current_mask_count += 1;
        } else {
            run_lengths.push(current_mask_count);
            run_value = new_val;
            current_mask_count = 1;
        }

        if new_val == pair.0 {
            new_mask_a.push(idx);
        } else {
            new_mask_b.push(idx);
        }
    }
    run_lengths.push(current_mask_count);

    // Special error that signals that we can reuse the last TwoDelta frame
    if !found_assignment_change {
        return Err(Error::from(EncodeError::TwoDeltaIdentical));
    }

    previous_masks.insert(pair.0, new_mask_a);
    previous_masks.insert(pair.1, new_mask_b);
    Ok(TwoDeltaEncodeFrame::from_run_lengths(pair, run_lengths))
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
/// * `previous_masks` - Mutable position mask map covering all ids that may appear in the pair.
///   Updated in-place to reflect `current` before returning.
///
/// # Returns
///
/// A `TwoDeltaEncodeFrame` for the transition, or `BenEncodeError::RepeatedSample` if the
/// two assignments are identical.
fn construct_twodelta_frame_from_mask_hint(
    previous: &[u16],
    current: &[u16],
    previous_masks: &mut HashMap<u16, Vec<usize>>,
) -> Result<TwoDeltaEncodeFrame> {
    for (&assign0, &assign1) in previous.iter().zip(current.iter()) {
        if assign0 != assign1 {
            return construct_twodelta_frame_from_pair_and_mask_hints(
                previous,
                current,
                (assign0, assign1),
                previous_masks,
            );
        }
    }

    return Err(Error::from(EncodeError::TwoDeltaIdentical));
}

/// Build a TwoDelta frame by scanning both assignment vectors from scratch, with no
/// hints from the caller.
///
/// Scans to the first changed position to discover the raw pair values, then makes
/// a second pass from position 0 to build run lengths over all pair positions.
/// `enc_pair.0` is determined lazily at the first pair position encountered in the
/// second pass (which may precede the first changed position), guaranteeing
/// `run_lengths[0] >= 1` with no leading zero.
///
/// # Arguments
///
/// * `previous` - The full assignment vector from the preceding sample.
/// * `current` - The full assignment vector for the sample being encoded.
///
/// # Returns
///
/// A `TwoDeltaEncodeFrame` for the transition, or an error if more than two distinct ids
/// appear across all changed positions.
fn construct_twodelta_frame_from_scratch(
    previous: &[u16],
    current: &[u16],
) -> Result<TwoDeltaEncodeFrame> {
    // Find the pair at the first changed position.
    let first_change = previous
        .iter()
        .zip(current.iter())
        .position(|(&p, &c)| p != c)
        .ok_or_else(|| Error::from(EncodeError::TwoDeltaIdentical))?;

    let (a, b) = (previous[first_change], current[first_change]);

    // Scan all positions: build run lengths for pair positions in previous.
    // enc_pair ordering is determined lazily at the first pair position encountered:
    // curr_val there is enc_pair.0, which may precede first_change for unchanged pair positions.
    let mut enc_pair = (0u16, 0u16);
    let mut enc_pair_known = false;
    let mut run_lengths: Vec<u16> = Vec::new();
    let mut run_value = 0u16;
    let mut run_count = 0u16;

    for (&prev_val, &curr_val) in previous.iter().zip(current.iter()) {
        if prev_val == a || prev_val == b {
            if curr_val != a && curr_val != b {
                return Err(Error::from(EncodeError::TwoDeltaTooManyIds));
            }
            if !enc_pair_known {
                enc_pair = (curr_val, if curr_val == a { b } else { a });
                run_value = enc_pair.0;
                enc_pair_known = true;
            }
            if curr_val == run_value {
                run_count += 1;
            } else {
                run_lengths.push(run_count);
                run_value = curr_val;
                run_count = 1;
            }
        } else if prev_val != curr_val {
            return Err(Error::from(EncodeError::TwoDeltaTooManyIds));
        }
    }
    run_lengths.push(run_count);

    Ok(TwoDeltaEncodeFrame::from_run_lengths(enc_pair, run_lengths))
}

/// Encode a transition between two assignment vectors as a TwoDelta frame.
///
/// This is the unhinted entry point. It falls back to a full scan of both
/// assignment vectors to discover the pair and compute run lengths. Prefer
/// `encode_twodelta_frame_with_hint` when previous_masks are available, as it avoids
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
) -> Result<TwoDeltaEncodeFrame> {
    encode_twodelta_frame_with_hint(previous_assignment, new_assignment, None, None)
}
