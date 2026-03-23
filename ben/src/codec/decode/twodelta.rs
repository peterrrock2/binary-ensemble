use super::errors::DecodeError;
use crate::codec::TwoDeltaEncodeFrame;
use std::io;

/// Apply decoded TwoDelta run lengths to produce a new assignment vector.
///
/// Positions in `assignment` that hold either value of `pair` are overwritten
/// according to the alternating run-length encoding. `pair.0` fills the first
/// run, `pair.1` the second, and so on.
///
/// # Arguments
///
/// * `assignment` - The assignment from the preceding frame (consumed and returned).
/// * `pair` - The two label values that participate in the delta.
/// * `run_lengths` - Alternating run lengths starting with the first value of `pair`.
///
/// # Returns
///
/// Returns the updated assignment vector, or an error if the run lengths are
/// exhausted before all relevant positions are covered.
pub(crate) fn apply_twodelta_runs_to_assignment(
    mut assignment: Vec<u16>,
    pair: (u16, u16),
    run_lengths: &[u16],
) -> io::Result<Vec<u16>> {
    let (first, second) = pair;

    let mut run_idx = 0usize;
    let mut remaining_in_run: u16 = *run_lengths.first().unwrap_or(&0);
    let mut current_value = first;

    for (pos, val) in assignment.iter_mut().enumerate() {
        if *val == first || *val == second {
            if remaining_in_run == 0 {
                run_idx += 1;
                if run_idx >= run_lengths.len() {
                    return Err(io::Error::from(DecodeError::TwoDeltaRunsExhausted {
                        run_idx,
                        pos,
                    }));
                }
                remaining_in_run = run_lengths[run_idx];
                current_value = if current_value == first {
                    second
                } else {
                    first
                };
            }
            *val = current_value;
            remaining_in_run -= 1;
        }
    }

    Ok(assignment)
}

/// Decode a TwoDelta frame by applying its delta to the previous assignment.
///
/// # Arguments
///
/// * `previous` - The assignment vector from the preceding frame.
/// * `frame` - The TwoDelta frame containing the pair and run-length vector.
///
/// # Returns
///
/// Returns the updated assignment vector.
pub fn decode_twodelta_frame(
    previous: Vec<u16>,
    frame: &TwoDeltaEncodeFrame,
) -> io::Result<Vec<u16>> {
    apply_twodelta_runs_to_assignment(previous, frame.pair, &frame.run_length_vector)
}
