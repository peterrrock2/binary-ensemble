use std::io;

pub(crate) const XBEN_TWODELTA_FULL_TAG: u8 = 0;
pub(crate) const XBEN_TWODELTA_CHUNK_TAG: u8 = 2;

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
