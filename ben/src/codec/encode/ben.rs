use super::types::{BenFrame, IdVec, TwoDeltaFrame};
use serde_json::Value;
use std::io;

/// Encode a JSON assignment record into the ben32 frame representation used by
/// XBEN streams.
///
/// # Arguments
///
/// * `data` - A JSON object containing an `assignment` array.
///
/// # Returns
///
/// Returns the encoded ben32 frame bytes terminated by the four-byte `0`
/// sentinel.
pub(crate) fn encode_ben32_line(data: Value) -> io::Result<IdVec> {
    let assign_vec = data["assignment"].as_array().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "'assignment' field either missing or is not an array of integers",
        )
    })?;
    let mut prev_assign: u16 = 0;
    let mut count: u16 = 0;
    let mut first = true;

    let mut ret = Vec::new();

    for assignment in assign_vec {
        let assign_u64 = assignment.as_u64().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "The value '{}' could not be unwrapped as an unsigned 64 bit integer.",
                    assignment
                ),
            )
        })?;
        let assign = u16::try_from(assign_u64).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("The value '{}' is too large to fit in a u16.", assign_u64),
            )
        })?;
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
) -> io::Result<TwoDeltaFrame> {
    let previous_assignment = previous_assignment.as_ref();
    let new_assignment = new_assignment.as_ref();

    if previous_assignment.len() != new_assignment.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "TwoDelta requires assignment vectors of equal length",
        ));
    }

    let mut pair_ids = [0u16; 2];
    let mut pair_len = 0usize;
    for (&previous, &current) in previous_assignment.iter().zip(new_assignment.iter()) {
        if previous == current {
            continue;
        }
        for value in [previous, current] {
            if !pair_ids[..pair_len].contains(&value) {
                if pair_len == 2 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "TwoDelta transitions may involve at most two assignment ids",
                    ));
                }
                pair_ids[pair_len] = value;
                pair_len += 1;
            }
        }
    }

    if pair_len == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "TwoDelta cannot encode identical assignments as a delta frame",
        ));
    }

    let pair = if pair_len == 1 {
        (pair_ids[0], pair_ids[0])
    } else {
        (pair_ids[0], pair_ids[1])
    };

    let mut pair_positions = Vec::new();
    pair_positions.reserve(previous_assignment.len());
    for (idx, (&previous, &current)) in previous_assignment
        .iter()
        .zip(new_assignment.iter())
        .enumerate()
    {
        let previous_in_pair = previous == pair.0 || previous == pair.1;
        let current_in_pair = current == pair.0 || current == pair.1;

        if previous_in_pair != current_in_pair {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "TwoDelta requires the changed id pair to occupy the same positions",
            ));
        }

        if !previous_in_pair && previous != current {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "TwoDelta found a change outside the selected id pair",
            ));
        }

        if previous_in_pair {
            pair_positions.push(idx);
        }
    }

    if pair_positions.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "TwoDelta requires at least one occurrence of the selected id pair",
        ));
    }

    let first_value = new_assignment[pair_positions[0]];
    let second_value = if pair.0 == pair.1 {
        pair.0
    } else if first_value == pair.0 {
        pair.1
    } else {
        pair.0
    };
    let ordered_pair = (first_value, second_value);

    let mut run_lengths = Vec::new();
    let mut current_value = first_value;
    let mut current_run = 0u16;

    for &idx in &pair_positions {
        let value = new_assignment[idx];
        if value != ordered_pair.0 && value != ordered_pair.1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "TwoDelta payload encountered an assignment outside the selected id pair",
            ));
        }

        if value == current_value {
            current_run += 1;
        } else {
            run_lengths.push(current_run);
            current_value = value;
            current_run = 1;
        }
    }

    if current_run > 0 {
        run_lengths.push(current_run);
    }

    Ok(TwoDeltaFrame::from_run_lengths(ordered_pair, run_lengths))
}
