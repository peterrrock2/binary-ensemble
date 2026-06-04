use std::collections::HashMap;
use std::io;

pub(crate) const XBEN_TWODELTA_FULL_TAG: u8 = 0;
pub(crate) const XBEN_TWODELTA_CHUNK_TAG: u8 = 2;

// Per-frame discriminator prepended to every frame of a plain-BEN `TwoDelta` stream. This is a
// distinct namespace from the XBEN columnar tags above: a BEN stream interleaves self-describing
// snapshot and delta frames, so the wire format is `[tag u8][body]`. The reader copy of these
// constants lives in `io::reader::twodelta`; the two must stay in agreement.
pub(crate) const BEN_TWODELTA_SNAPSHOT_TAG: u8 = 0x00;
pub(crate) const BEN_TWODELTA_DELTA_TAG: u8 = 0x01;

/// Default number of delta frames per columnar chunk in XBEN TwoDelta.
pub const DEFAULT_TWODELTA_CHUNK_SIZE: usize = 10_000;

/// Walk a TwoDelta repeat-eligible assignment and emit the `(pair, run_lengths)` describing it.
///
/// Used by both the BEN and XBEN writers to construct the body of a TwoDelta "repeat" frame: each
/// writer wraps the result in its own frame type. Returns an `InvalidInput` error if any run
/// exceeds `u16::MAX` in length.
pub(crate) fn twodelta_repeat_runs(assignment: &[u16]) -> io::Result<((u16, u16), Vec<u16>)> {
    let first = assignment.first().copied().unwrap_or(0);
    let second = assignment
        .iter()
        .copied()
        .find(|&value| value != first)
        .unwrap_or_else(|| if first == u16::MAX { 0 } else { first + 1 });

    let mut run_lengths = Vec::new();
    let mut current = first;
    let mut run_len = 0u16;

    for &value in assignment {
        if value != first && value != second {
            continue;
        }
        if value == current {
            if run_len == u16::MAX {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "TwoDelta repeat frame contains a run longer than u16::MAX",
                ));
            }
            run_len += 1;
        } else {
            if run_len > 0 {
                run_lengths.push(run_len);
            }
            current = value;
            run_len = 1;
        }
    }
    if run_len > 0 {
        run_lengths.push(run_len);
    }

    Ok(((first, second), run_lengths))
}

/// How an inter-sample transition should be framed in a `TwoDelta` stream.
pub(crate) enum TransitionKind {
    /// No position changed value; the sample repeats the previous one.
    Repeat,
    /// Every changed position swaps between exactly these two district ids; encode as a delta.
    Delta(u16, u16),
    /// More than two distinct district ids change; fall back to a full snapshot frame.
    Snapshot,
}

/// Classify a transition between two equal-length assignment vectors in a single O(n) scan.
///
/// Walks `previous` against `current`, collecting the set of district ids seen at changed positions
/// and short-circuiting to [`TransitionKind::Snapshot`] as soon as a third distinct id appears. A
/// union of exactly two ids is necessarily a clean 2-swap: every changed position has both its old
/// and new value within the pair, so positions outside the pair cannot have moved (that would
/// introduce a third id). No change at all is a [`TransitionKind::Repeat`].
///
/// A full scan is required for correctness: the mask-hint fast path only walks the inferred pair's
/// positions and would silently miss a third district changing at an out-of-pair position.
///
/// `zip` would silently truncate to the shorter vector, so the length is checked explicitly,
/// preserving the validation the strict single-frame encoder performs.
pub(crate) fn classify_transition(previous: &[u16], current: &[u16]) -> io::Result<TransitionKind> {
    if previous.len() != current.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "TwoDelta transition length mismatch: previous has {} positions, current has {}",
                previous.len(),
                current.len()
            ),
        ));
    }

    let mut first: Option<u16> = None;
    let mut second: Option<u16> = None;
    let mut changed = false;

    for (&prev_val, &curr_val) in previous.iter().zip(current.iter()) {
        if prev_val == curr_val {
            continue;
        }
        changed = true;
        for val in [prev_val, curr_val] {
            match (first, second) {
                (None, _) => first = Some(val),
                (Some(f), _) if f == val => {}
                (Some(_), None) => second = Some(val),
                (Some(_), Some(s)) if s == val => {}
                (Some(_), Some(_)) => return Ok(TransitionKind::Snapshot),
            }
        }
    }

    if !changed {
        return Ok(TransitionKind::Repeat);
    }

    match (first, second) {
        // A changed position contributes two distinct ids, so once `changed` is set both slots
        // are filled.
        (Some(a), Some(b)) => Ok(TransitionKind::Delta(a, b)),
        _ => unreachable!("a differing position yields two distinct ids"),
    }
}

/// Whether both ids of a classified delta pair have a usable position mask, i.e. both districts
/// already appear in the previous assignment.
///
/// The mask-hint encoder requires this. A 2-id transition can still introduce a district that was
/// absent from the previous assignment (e.g. an empty district that just gained nodes); it has no
/// mask to delta against and must be encoded via a snapshot instead.
pub(crate) fn pair_has_masks(masks: &HashMap<u16, Vec<usize>>, a: u16, b: u16) -> bool {
    masks.get(&a).is_some_and(|m| !m.is_empty()) && masks.get(&b).is_some_and(|m| !m.is_empty())
}
