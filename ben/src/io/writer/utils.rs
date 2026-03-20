use super::ben::AssignmentHints;
use super::twodelta::XBEN_TWODELTA_FULL_TAG;
use crate::util::rle::assign_to_rle;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{self, Result};

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
pub(super) fn is_repeated_assignment(previous_sample: &[u16], assign_vec: &[u16]) -> bool {
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
pub(super) fn analyze_twodelta_transition(
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
pub(super) fn parse_json_assignment(data: Value) -> Result<Vec<u16>> {
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
pub(super) fn encode_xben_twodelta_full_frame(assignments: &[u16]) -> Vec<u8> {
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
