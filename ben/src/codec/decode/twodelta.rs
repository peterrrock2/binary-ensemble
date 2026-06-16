use super::errors::DecodeError;
use crate::codec::BenEncodeFrame;
use std::collections::HashMap;
use std::io;

/// Ordered position masks for the labels in the current TwoDelta assignment.
///
/// Plain TwoDelta BEN replay repeatedly updates only the positions occupied by the two labels in
/// the delta pair. Keeping those masks avoids a full-assignment scan for every delta frame.
#[derive(Debug, Clone)]
pub(crate) struct TwoDeltaMaskIndex {
    masks: HashMap<u16, Vec<usize>>,
}

impl TwoDeltaMaskIndex {
    pub(crate) fn from_assignment(assignment: &[u16]) -> Self {
        let mut masks: HashMap<u16, Vec<usize>> = HashMap::new();
        for (pos, &label) in assignment.iter().enumerate() {
            masks.entry(label).or_default().push(pos);
        }
        Self { masks }
    }

    pub(crate) fn apply_runs(
        &mut self,
        assignment: &mut [u16],
        pair: (u16, u16),
        run_lengths: &[u16],
    ) -> io::Result<()> {
        reject_zero_run_lengths(run_lengths)?;

        let (first, second) = pair;
        let first_positions = self.masks.remove(&first).unwrap_or_default();
        let second_positions = self.masks.remove(&second).unwrap_or_default();
        let mut next_first = Vec::with_capacity(first_positions.len());
        let mut next_second = Vec::with_capacity(second_positions.len());

        let mut first_iter = first_positions.into_iter().peekable();
        let mut second_iter = second_positions.into_iter().peekable();
        let mut run_idx = 0usize;
        let mut remaining_in_run: u16 = *run_lengths.first().unwrap_or(&0);
        let mut current_value = first;

        while first_iter.peek().is_some() || second_iter.peek().is_some() {
            if remaining_in_run == 0 {
                run_idx += 1;
                if run_idx >= run_lengths.len() {
                    return Err(io::Error::from(DecodeError::TwoDeltaRunsExhausted {
                        run_idx,
                        pos: next_position_for_error(&mut first_iter, &mut second_iter),
                    }));
                }
                remaining_in_run = run_lengths[run_idx];
                current_value = if current_value == first {
                    second
                } else {
                    first
                };
            }

            let pos = next_mask_position(&mut first_iter, &mut second_iter);
            assignment[pos] = current_value;
            if current_value == first {
                next_first.push(pos);
            } else {
                next_second.push(pos);
            }
            remaining_in_run -= 1;
        }

        reject_unconsumed_runs(remaining_in_run, run_lengths, run_idx)?;

        if !next_first.is_empty() {
            self.masks.insert(first, next_first);
        }
        if !next_second.is_empty() {
            self.masks.insert(second, next_second);
        }

        Ok(())
    }
}

fn next_mask_position<I, J>(
    first_iter: &mut std::iter::Peekable<I>,
    second_iter: &mut std::iter::Peekable<J>,
) -> usize
where
    I: Iterator<Item = usize>,
    J: Iterator<Item = usize>,
{
    match (first_iter.peek().copied(), second_iter.peek().copied()) {
        (Some(a), Some(b)) if a <= b => first_iter.next().unwrap(),
        (Some(_), Some(_)) => second_iter.next().unwrap(),
        (Some(_), None) => first_iter.next().unwrap(),
        (None, Some(_)) => second_iter.next().unwrap(),
        (None, None) => unreachable!("caller checked that at least one mask iterator has data"),
    }
}

fn next_position_for_error<I, J>(
    first_iter: &mut std::iter::Peekable<I>,
    second_iter: &mut std::iter::Peekable<J>,
) -> usize
where
    I: Iterator<Item = usize>,
    J: Iterator<Item = usize>,
{
    first_iter
        .peek()
        .copied()
        .into_iter()
        .chain(second_iter.peek().copied())
        .min()
        .unwrap_or(0)
}

/// Reject a zero run length. The encoder never emits one, and the paint loops assume none exist: a
/// zero reaching them would underflow `remaining_in_run` and silently mispaint positions. Every
/// unpacker rejects zeros upstream; this keeps the invariant local so no future caller can
/// reintroduce the hazard.
fn reject_zero_run_lengths(run_lengths: &[u16]) -> io::Result<()> {
    if run_lengths.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "TwoDelta run lengths contain a zero; the encoder never emits zero-length runs",
        ));
    }
    Ok(())
}

/// Reject run lengths that outlast the relevant positions: either the current run still has count
/// left, or some later run is nonzero.
fn reject_unconsumed_runs(
    remaining_in_run: u16,
    run_lengths: &[u16],
    run_idx: usize,
) -> io::Result<()> {
    if remaining_in_run > 0 || run_lengths.iter().skip(run_idx + 1).any(|&run| run > 0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "TwoDelta run lengths exceed the number of positions in the assignment",
        ));
    }
    Ok(())
}

/// Apply decoded TwoDelta run lengths to produce a new assignment vector.
///
/// Positions in `assignment` that hold either value of `pair` are overwritten according to the
/// alternating run-length encoding. `pair.0` fills the first run, `pair.1` the second, and so on.
///
/// # Arguments
///
/// * `assignment` - The assignment from the preceding frame (consumed and returned).
/// * `pair` - The two label values that participate in the delta.
/// * `run_lengths` - Alternating run lengths starting with the first value of `pair`.
///
/// # Returns
///
/// Returns the updated assignment vector, or an error if the run lengths are exhausted before all
/// relevant positions are covered or any run length is zero.
pub(crate) fn apply_twodelta_runs_to_assignment(
    mut assignment: Vec<u16>,
    pair: (u16, u16),
    run_lengths: &[u16],
) -> io::Result<Vec<u16>> {
    reject_zero_run_lengths(run_lengths)?;

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

    reject_unconsumed_runs(remaining_in_run, run_lengths, run_idx)?;

    Ok(assignment)
}

/// Decode a TwoDelta frame by applying its delta to the previous assignment.
///
/// # Arguments
///
/// * `previous` - The assignment vector from the preceding frame.
/// * `frame` - A TwoDelta-arm [`BenEncodeFrame`] containing the pair and run-length vector.
///
/// # Returns
///
/// Returns the updated assignment vector, or an error if `frame` is not the `TwoDelta` arm.
pub fn decode_twodelta_frame(previous: Vec<u16>, frame: &BenEncodeFrame) -> io::Result<Vec<u16>> {
    match frame {
        BenEncodeFrame::TwoDelta {
            pair,
            run_length_vector,
            ..
        } => apply_twodelta_runs_to_assignment(previous, *pair, run_length_vector),
        other => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "decode_twodelta_frame called with non-TwoDelta variant: {:?}",
                other.variant()
            ),
        )),
    }
}
