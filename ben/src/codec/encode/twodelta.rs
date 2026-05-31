use super::errors::EncodeError;
use crate::codec::BenEncodeFrame;
use std::collections::HashMap;
use std::io::{Error, ErrorKind, Result};

/// Encode a transition between two assignment vectors as a TwoDelta frame, optionally using
/// caller-supplied hints to accelerate encoding.
///
/// # Arguments
///
/// * `previous_assignment` - The full assignment vector from the preceding sample.
/// * `new_assignment` - The full assignment vector for the sample being encoded.
/// * `delta_pair` - An optional hint asserting which pair of ids is involved in the transition.
///   Must be provided together with `previous_masks`, and the two ids must be distinct.
/// * `previous_masks` - An optional mutable map from district id to the sorted list of positions it
///   occupies in `previous_assignment`. When provided, the map is updated in-place to reflect
///   `new_assignment` before returning.
///
/// # Returns
///
/// A `BenEncodeFrame` describing the transition from `previous_assignment` to `new_assignment`.
///
/// # TwoDelta encoding
///
/// A TwoDelta frame is valid only when every position that changes between `previous_assignment`
/// and `new_assignment` involves exactly two district ids (call them A and B), and no position
/// outside that pair changes. The frame stores the pair and the lengths of alternating runs of A
/// and B over the positions occupied by the pair, ordered by position. The first run always
/// corresponds to whichever id occupies the lowest-indexed position.
///
/// # Hints
///
/// Two optional hints can be provided to avoid scanning the full assignment vector:
///
/// - `delta_pair`: The caller asserts that exactly this pair of ids is involved in the transition.
///   Must be provided together with `previous_masks`. The pair must have two distinct ids — passing
///   `(x, x)` is an error.
///
/// - `previous_masks`: A mutable map from district id to the sorted list of positions it occupies
///   in `previous_assignment`. When provided, the function reads positions directly from the map
///   instead of scanning the assignment vector, and updates the map in-place to reflect
///   `new_assignment` before returning. The previous_masks must cover every id that appears in the
///   pair; a missing or empty entry is an error.
///
/// The hints are not independent: `delta_pair` requires `previous_masks`. Providing
/// `previous_masks` without `delta_pair` is allowed — the function will infer the pair from the
/// first differing position and then use the previous_masks from there.
///
/// When no hints are provided the function falls back to a full scan of both assignment vectors.
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
    count: Option<u16>,
) -> Result<BenEncodeFrame> {
    let previous_assignment = previous_assignment.as_ref();
    let new_assignment = new_assignment.as_ref();

    if previous_assignment.len() != new_assignment.len() {
        return Err(Error::from(EncodeError::LengthMismatch {
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
            count,
        ),
        (None, Some(masks)) => construct_twodelta_frame_from_mask_hint(
            previous_assignment,
            new_assignment,
            masks,
            count,
        ),
        _ => construct_twodelta_frame_from_scratch(previous_assignment, new_assignment, count),
    }
}

/// A district pair ordered so that the first element is the district occupying the **first pair
/// position in the current assignment** — i.e. the district whose run is emitted first.
///
/// This ordering is not mere numeric or positional sorting of the two ids; it is the
/// round-trip-determinism invariant TwoDelta depends on. The decoder replays the alternating runs
/// starting from that same first position, so a pair ordered the other way would silently decode to
/// a different assignment. Constructing the pair only through [`Self::from_first_pair_position`]
/// makes that broken ordering unrepresentable.
#[derive(Clone, Copy)]
struct FirstRunDistrictPair {
    first_run_district: u16,
    second_run_district: u16,
}

impl FirstRunDistrictPair {
    /// Order `pair` so that the first-run district is whichever id the current assignment places at
    /// `first_pair_pos` (the lowest position held by either id). `current[first_pair_pos]` must be
    /// one of the two ids in `pair`.
    fn from_first_pair_position(pair: (u16, u16), first_pair_pos: usize, current: &[u16]) -> Self {
        if current[first_pair_pos] == pair.0 {
            FirstRunDistrictPair {
                first_run_district: pair.0,
                second_run_district: pair.1,
            }
        } else {
            FirstRunDistrictPair {
                first_run_district: pair.1,
                second_run_district: pair.0,
            }
        }
    }

    /// The district whose run is emitted first (it holds the lowest pair position in `current`).
    fn first_run_district(&self) -> u16 {
        self.first_run_district
    }

    /// The other district in the pair.
    fn second_run_district(&self) -> u16 {
        self.second_run_district
    }

    /// The ordered `(first_run_district, second_run_district)` tuple expected by
    /// [`BenEncodeFrame::from_run_lengths`].
    fn as_ordered_pair(&self) -> (u16, u16) {
        (self.first_run_district, self.second_run_district)
    }
}

/// Validate that `previous_masks` contains non-empty entries for both ids in `pair` and return them
/// as a [`FirstRunDistrictPair`] ordered by their first position in `current`.
///
/// Ordering by first position ensures that the run-length sequence produced during encoding always
/// begins with the id whose positions come first in the assignment vector, which is required for
/// deterministic round-trip decoding.
///
/// # Arguments
///
/// * `pair` - The two district ids to validate and order.
/// * `previous_masks` - The position mask map to look up entries in.
///
/// # Returns
///
/// A [`FirstRunDistrictPair`] whose first-run district has a smaller first position in `current`,
/// or an error if either id is absent from `previous_masks` or has an empty position list.
fn validate_masks_and_order_pairs_for_twodelta(
    pair: (u16, u16),
    masks: &HashMap<u16, Vec<usize>>,
    current: &[u16],
) -> Result<FirstRunDistrictPair> {
    let mask_a = match masks.get(&pair.0) {
        Some(m) => m,
        None => return Err(Error::from(EncodeError::TwoDeltaMissingMask { id: pair.0 })),
    };

    let mask_b = match masks.get(&pair.1) {
        Some(m) => m,
        None => return Err(Error::from(EncodeError::TwoDeltaMissingMask { id: pair.1 })),
    };

    if mask_a.is_empty() {
        return Err(Error::from(EncodeError::TwoDeltaEmptyMask { id: pair.0 }));
    }

    if mask_b.is_empty() {
        return Err(Error::from(EncodeError::TwoDeltaEmptyMask { id: pair.1 }));
    }

    // Order so that the first-run district is the value the new assignment places at the first pair
    // position (the lowest index held by either mask). This guarantees run_lengths[0] >= 1 with no
    // leading-zero sentinel.
    let first_pair_pos = mask_a[0].min(mask_b[0]);
    Ok(FirstRunDistrictPair::from_first_pair_position(
        pair,
        first_pair_pos,
        current,
    ))
}

/// Build a TwoDelta frame using both a known pair and pre-computed position masks.
///
/// This is the fast path used during recombination-aware encoding, where the caller already knows
/// which two ids are swapping and has maintained a mask for each id.
///
/// The function merges the two sorted position lists from `previous_masks` to produce the
/// interleaved sequence of positions, validates that every referenced position in `previous` and
/// `current` belongs to the pair, computes the run lengths over `current`, and then updates
/// `previous_masks` in-place to reflect the new positions of each id in `current`.
///
/// # Arguments
///
/// * `previous` - The full assignment vector from the preceding sample.
/// * `current` - The full assignment vector for the sample being encoded.
/// * `delta_pair` - The pair of ids asserted to be involved in the transition.
/// * `previous_masks` - Mutable position mask map for both ids in the pair. Updated in-place to
///   reflect `current` before returning.
///
/// # Returns
///
/// A `BenEncodeFrame` for the transition, or `BenEncodeError::RepeatedSample` if no position
/// actually changed value (signalling the frame can be deduplicated), or another error if a mask
/// entry is inconsistent with the assignment data.
fn construct_twodelta_frame_from_pair_and_mask_hints(
    previous: &[u16],
    current: &[u16],
    delta_pair: (u16, u16),
    previous_masks: &mut HashMap<u16, Vec<usize>>,
    count: Option<u16>,
) -> Result<BenEncodeFrame> {
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
        .get(&pair.first_run_district())
        .expect("Failed to get mask for first-run district after validation");
    let mask_b = previous_masks
        .get(&pair.second_run_district())
        .expect("Failed to get mask for second-run district after validation");

    let new_capacity = mask_a.len() + mask_b.len();
    let mut run_lengths = Vec::with_capacity(new_capacity);
    // Accumulate updated masks reflecting positions in `current`.
    let mut new_mask_a = Vec::with_capacity(new_capacity);
    let mut new_mask_b = Vec::with_capacity(new_capacity);

    let (mut i, mut j) = (0usize, 0usize);
    // The first-run district is guaranteed to equal current[first_pair_pos] by
    // validate_masks_and_order_pairs_for_twodelta, so the first iteration always hits the
    // `new_val == active_district` branch and increments the run length — no special-case
    // initialization needed.
    let mut active_district = pair.first_run_district();
    let mut active_run_length = 0u16;
    let mut saw_changed_assignment_position = false;

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

        if previous_value != pair.first_run_district() && previous_value != pair.second_run_district()
        {
            return Err(Error::from(EncodeError::TwoDeltaMaskOutOfPair {
                pos: idx,
                actual: previous_value,
                a: pair.first_run_district(),
                b: pair.second_run_district(),
            }));
        }
        if new_val != pair.first_run_district() && new_val != pair.second_run_district() {
            return Err(Error::from(EncodeError::TwoDeltaMaskOutOfPair {
                pos: idx,
                actual: new_val,
                a: pair.first_run_district(),
                b: pair.second_run_district(),
            }));
        }
        if new_val != previous_value {
            saw_changed_assignment_position = true;
        }

        if new_val == active_district {
            if active_run_length == u16::MAX {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    "TwoDelta run length exceeds u16::MAX",
                ));
            }
            active_run_length += 1;
        } else {
            run_lengths.push(active_run_length);
            active_district = new_val;
            active_run_length = 1;
        }

        if new_val == pair.first_run_district() {
            new_mask_a.push(idx);
        } else {
            new_mask_b.push(idx);
        }
    }
    run_lengths.push(active_run_length);

    // Special error that signals that we can reuse the last TwoDelta frame
    if !saw_changed_assignment_position {
        return Err(Error::from(EncodeError::TwoDeltaIdentical));
    }

    previous_masks.insert(pair.first_run_district(), new_mask_a);
    previous_masks.insert(pair.second_run_district(), new_mask_b);
    Ok(BenEncodeFrame::from_run_lengths(
        pair.as_ordered_pair(),
        run_lengths,
        count,
    ))
}

/// Build a TwoDelta frame using only pre-computed position masks, inferring the pair from the first
/// differing position between `previous` and `current`.
///
/// Scans until it finds a position where the two assignments differ, then delegates to
/// `construct_twodelta_frame_from_pair_and_mask_hints` with that pair. If no difference is found
/// the assignments are identical and `BenEncodeError::RepeatedSample` is returned.
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
/// A `BenEncodeFrame` for the transition, or `BenEncodeError::RepeatedSample` if the two
/// assignments are identical.
fn construct_twodelta_frame_from_mask_hint(
    previous: &[u16],
    current: &[u16],
    previous_masks: &mut HashMap<u16, Vec<usize>>,
    count: Option<u16>,
) -> Result<BenEncodeFrame> {
    for (&assign0, &assign1) in previous.iter().zip(current.iter()) {
        if assign0 != assign1 {
            return construct_twodelta_frame_from_pair_and_mask_hints(
                previous,
                current,
                (assign0, assign1),
                previous_masks,
                count,
            );
        }
    }

    return Err(Error::from(EncodeError::TwoDeltaIdentical));
}

/// Build a TwoDelta frame by scanning both assignment vectors from scratch, with no hints from the
/// caller.
///
/// Scans to the first changed position to discover the raw pair values, then makes a second pass
/// from position 0 to build run lengths over all pair positions. `enc_pair.0` is determined lazily
/// at the first pair position encountered in the second pass (which may precede the first changed
/// position), guaranteeing `run_lengths[0] >= 1` with no leading zero.
///
/// # Arguments
///
/// * `previous` - The full assignment vector from the preceding sample.
/// * `current` - The full assignment vector for the sample being encoded.
///
/// # Returns
///
/// A `BenEncodeFrame` for the transition, or an error if more than two distinct ids appear across
/// all changed positions.
fn construct_twodelta_frame_from_scratch(
    previous: &[u16],
    current: &[u16],
    count: Option<u16>,
) -> Result<BenEncodeFrame> {
    // Find the pair at the first changed position.
    let first_change = previous
        .iter()
        .zip(current.iter())
        .position(|(&p, &c)| p != c)
        .ok_or_else(|| Error::from(EncodeError::TwoDeltaIdentical))?;

    let (a, b) = (previous[first_change], current[first_change]);

    // Scan all positions: build run lengths for pair positions in previous. enc_pair ordering is
    // determined lazily at the first pair position encountered: curr_val there is enc_pair.0, which
    // may precede first_change for unchanged pair positions.
    let mut enc_pair = (0u16, 0u16);
    let mut enc_pair_known = false;
    let mut run_lengths: Vec<u16> = Vec::new();
    let mut active_district = 0u16;
    let mut active_run_length = 0u16;

    for (&prev_val, &curr_val) in previous.iter().zip(current.iter()) {
        if prev_val == a || prev_val == b {
            if curr_val != a && curr_val != b {
                return Err(Error::from(EncodeError::TwoDeltaTooManyIds));
            }
            if !enc_pair_known {
                enc_pair = (curr_val, if curr_val == a { b } else { a });
                active_district = enc_pair.0;
                enc_pair_known = true;
            }
            if curr_val == active_district {
                if active_run_length == u16::MAX {
                    return Err(Error::new(
                        ErrorKind::InvalidInput,
                        "TwoDelta run length exceeds u16::MAX",
                    ));
                }
                active_run_length += 1;
            } else {
                run_lengths.push(active_run_length);
                active_district = curr_val;
                active_run_length = 1;
            }
        } else if prev_val != curr_val {
            return Err(Error::from(EncodeError::TwoDeltaTooManyIds));
        }
    }
    run_lengths.push(active_run_length);

    Ok(BenEncodeFrame::from_run_lengths(
        enc_pair,
        run_lengths,
        count,
    ))
}

/// Encode a transition between two assignment vectors as a TwoDelta frame.
///
/// This is the unhinted entry point. It falls back to a full scan of both assignment vectors to
/// discover the pair and compute run lengths. Prefer `encode_twodelta_frame_with_hint` when
/// previous_masks are available, as it avoids the scan entirely.
///
/// The transition is valid only when all changed positions involve exactly two district ids and
/// positions outside that pair remain unchanged.
///
/// # Arguments
///
/// * `previous_assignment` - The previous full assignment vector.
/// * `new_assignment` - The next full assignment vector.
///
/// # Returns
///
/// Returns a TwoDelta frame describing the transition, or an error if the transition involves more
/// than two ids or the assignments are identical.
pub fn encode_twodelta_frame(
    previous_assignment: impl AsRef<[u16]>,
    new_assignment: impl AsRef<[u16]>,
    count: Option<u16>,
) -> Result<BenEncodeFrame> {
    encode_twodelta_frame_with_hint(previous_assignment, new_assignment, None, None, count)
}
