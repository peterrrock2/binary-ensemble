use super::errors::BenEncodeError;
use super::types::{BenFrame, IdVec, TwoDeltaFrame};
use serde_json::Value;
use std::collections::HashMap;
use std::io::{Error, ErrorKind, Result};

pub(crate) type TwoDeltaRuns = ((u16, u16), Vec<u16>);

/// Encode a JSON assignment record into the ben32 frame representation used by
/// XBEN streams.
///
/// Note: This is a helper function that is only used in the testing suite.
///
/// # Arguments
///
/// * `data` - A JSON object containing an `assignment` array.
///
/// # Returns
///
/// Returns the encoded ben32 frame bytes terminated by the four-byte `0`
/// sentinel.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn encode_ben32_line(data: Value) -> Result<IdVec> {
    let assign_vec = data["assignment"].as_array().ok_or_else(|| {
        Error::new(
            ErrorKind::InvalidData,
            "'assignment' field either missing or is not an array of integers",
        )
    })?;
    encode_ben32_assignments(
        assign_vec
            .iter()
            .map(|assignment| {
                let assign_u64 = assignment.as_u64().ok_or_else(|| {
                    Error::new(
                        ErrorKind::InvalidData,
                        format!(
                            "The value '{}' could not be unwrapped as an unsigned 64 bit integer.",
                            assignment
                        ),
                    )
                })?;
                u16::try_from(assign_u64).map_err(|_| {
                    Error::new(
                        ErrorKind::InvalidData,
                        format!("The value '{}' is too large to fit in a u16.", assign_u64),
                    )
                })
            })
            .collect::<Result<Vec<u16>>>()?,
    )
}

/// Encode an assignment vector into a Ben32 vector
///
/// # Arguments
///
/// * `assign_vec` - The full assignment vector to encode
///
/// # Returns
///
/// Returns the encoded BEN32 frame byte vector.
pub(crate) fn encode_ben32_assignments(assign_vec: impl AsRef<[u16]>) -> Result<IdVec> {
    let assign_vec = assign_vec.as_ref();
    let mut prev_assign: u16 = 0;
    let mut count: u16 = 0;
    let mut first = true;

    let mut ret = Vec::new();

    for &assign in assign_vec {
        if first {
            prev_assign = assign;
            count = 1;
            first = false;
            continue;
        }
        if assign == prev_assign {
            count += 1;
        } else {
            let encoded = (prev_assign as u32) << 16 | count as u32;
            ret.extend(&encoded.to_be_bytes());
            prev_assign = assign;
            count = 1;
        }
    }

    if count > 0 {
        let encoded = (prev_assign as u32) << 16 | count as u32;
        ret.extend(&encoded.to_be_bytes());
    }

    ret.extend([0, 0, 0, 0]);
    Ok(IdVec::U8(ret))
}

/// Encode a full assignment vector into a single BEN frame.
///
/// # Arguments
///
/// * `assign_vec` - The full assignment vector to encode.
///
/// # Returns
///
/// Returns the encoded BEN frame bytes, including the per-frame header.
pub fn encode_ben_vec_from_assign(assign_vec: impl AsRef<[u16]>) -> BenFrame {
    BenFrame::from_assignment(assign_vec)
}

/// Encode a run-length encoded assignment vector into a BEN frame.
///
/// The returned byte vector contains the per-frame BEN header followed by the
/// packed `(value, run_length)` payload.
///
/// # Arguments
///
/// * `rle_vec` - The run-length encoded assignment vector as `(value, count)`
///   pairs.
///
/// # Returns
///
/// Returns the encoded BEN frame bytes, including the per-frame header.
pub fn encode_ben_vec_from_rle(rle_vec: Vec<(u16, u16)>) -> BenFrame {
    BenFrame::from_rle(rle_vec)
}

/// Encode a sample transition as a TwoDelta frame.
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
/// Returns a serialized TwoDelta frame describing the transition.
pub fn encode_twodelta_vec(
    previous_assignment: impl AsRef<[u16]>,
    new_assignment: impl AsRef<[u16]>,
) -> Result<TwoDeltaFrame> {
    encode_twodelta_vec_with_hint(previous_assignment, new_assignment, None, None)
}

/// Encode a sample transition as a TwoDelta frame, using hints to help speed up the
/// encoding process.
///
/// In the case that the delta pair exists, we will take it as gospel that the pairs that were
/// swapped were the ones in the delta pair. This is a hyper optimization included to improve
/// encoding speed of recombination algorithms, in particular.
///
/// # Arguments
///
/// * `previous_assignment` - The previous full assignment vector.
/// * `new_assignment` - The next full assignment vector.
/// * `delta_pair` - An optional pair of assignment ids that are expected to be involved in
///  the transition. If provided, the function will check that only these two ids are involved in
///  the changes between the previous and new assignments, and that they occupy the same positions.
/// * `masks` - An optional mapping from assignment ids to their positions in the previous
/// assignment vector. If provided, the function will use these masks to efficiently compute the
/// positions of the changed ids, and will validate that they are consistent with the actual
/// changes between the previous and new assignments.
///
/// # Returns
/// A serialized TwoDelta frame describing the transition, or an error if the hints are
/// invalid
pub(crate) fn encode_twodelta_vec_with_hint(
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
        (Some(pair), Some(masks)) => determine_twodelta_frame_from_pair_and_mask_hints(
            previous_assignment,
            new_assignment,
            pair,
            masks,
        ),
        (None, Some(masks)) => {
            determine_twodelta_frame_from_mask_hint(previous_assignment, new_assignment, masks)
        }
        _ => determine_twodelta_frame_from_scratch(previous_assignment, new_assignment),
    }

    // Ok(TwoDeltaFrame::from_run_lengths(ordered_pair, run_lengths))
}

fn validate_masks_and_order_pairs(
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

fn determine_twodelta_frame_from_pair_and_mask_hints(
    previous: &[u16],
    current: &[u16],
    delta_pair: (u16, u16),
    masks: &mut HashMap<u16, Vec<usize>>,
) -> Result<TwoDeltaFrame> {
    let pair = match validate_masks_and_order_pairs(delta_pair, masks) {
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
    let mut new_mask_a = Vec::with_capacity(new_capacity);
    let mut new_mask_b = Vec::with_capacity(new_capacity);

    let (mut i, mut j) = (0usize, 0usize);
    let mut current_mask_count = 0u16;
    let mut current_value = pair.0;

    let mut found_assignment_change = false;

    while i < mask_a.len() || j < mask_b.len() {
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

fn determine_twodelta_frame_from_mask_hint(
    previous: &[u16],
    current: &[u16],
    masks: &mut HashMap<u16, Vec<usize>>,
) -> Result<TwoDeltaFrame> {
    for (&assign0, &assign1) in previous.iter().zip(current.iter()) {
        if assign0 != assign1 {
            return determine_twodelta_frame_from_pair_and_mask_hints(
                previous,
                current,
                (assign0, assign1),
                masks,
            );
        }
    }

    return Err(BenEncodeError::RepeatedSample.into());
}

fn determine_twodelta_frame_from_scratch(
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
