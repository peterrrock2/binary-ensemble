//! XBEN iteration logic for the unified stream reader.

use std::io::{self, Cursor, Read};

use super::{zero_count_frame_error, XBenInner};
use crate::codec::decode::{apply_twodelta_runs_to_assignment, decode_ben32_line, DecodeError};
use crate::io::reader::subsample::MkvRecord;
use crate::io::reader::twodelta::{XBEN_TWODELTA_CHUNK_TAG, XBEN_TWODELTA_FULL_TAG};
use crate::progress::Spinner;
use crate::util::rle::rle_to_vec;
use crate::BenVariant;

/// Try to extract one complete ben32 frame from the buffered overflow.
///
/// Scans `overflow` for a four-byte zero sentinel that terminates a ben32 frame and, for MkvChain
/// streams, reads the trailing repetition count.
pub(super) fn pop_frame_from_overflow(
    variant: BenVariant,
    overflow: &[u8],
) -> Option<(&[u8], usize, u16)> {
    if variant == BenVariant::Standard {
        if overflow.len() < 4 {
            return None;
        }
        for i in (3..overflow.len()).step_by(4) {
            if overflow[i - 3..=i] == [0, 0, 0, 0] {
                let end = i + 1;
                let frame = &overflow[..end];
                return Some((frame, end, 1));
            }
        }
        None
    } else {
        if overflow.len() < 6 {
            return None;
        }
        // MkvChain records are `[4-byte-aligned ben32 body + 4-byte zero sentinel] + 2-byte
        // repetition count`. The trailing 2-byte count shifts each subsequent record off the 4-byte
        // ben32 grid, so a sentinel can begin at any even offset; the scan must step by 2, not 4,
        // to find them all. A false 4-zero window at an off-grid offset would require a
        // zero-count run, which is rejected upstream.
        for i in (3..overflow.len().saturating_sub(2)).step_by(2) {
            if overflow[i - 3..=i] == [0, 0, 0, 0] {
                let count_hi = overflow[i + 1];
                let count_lo = overflow[i + 2];
                let count = u16::from_be_bytes([count_hi, count_lo]);
                let end = i + 3;
                let frame = &overflow[..end];
                return Some((frame, end, count));
            }
        }
        None
    }
}

/// A TwoDelta frame popped from the overflow buffer: its `(value, run_length)` pairs, the number of
/// overflow bytes the frame consumed, and its repetition count.
type PoppedTwoDeltaFrame = (Vec<(u16, u16)>, usize, u16);

/// A chunk's accumulated `total_runs` is a sum of up to ~4.3 billion `u32`-derived run counts from
/// an untrusted stream, so on a corrupt or malicious chunk it can wrap `usize` even on a 64-bit
/// target. A wrapped `total_len` would then slip past the bounds check and panic on an out-of-range
/// index, so a wrap is reported as `InvalidData` instead. (Single `u32 * small constant` lengths
/// elsewhere can't overflow a 64-bit `usize`, which this build assumes, so they stay plain.)
fn twodelta_len_overflow() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "XBEN TwoDelta chunk: run-count total overflowed usize",
    )
}

/// Try to extract one complete TwoDelta frame from the buffered overflow.
fn pop_twodelta_frame_from_overflow(overflow: &[u8]) -> Option<io::Result<PoppedTwoDeltaFrame>> {
    let tag = *overflow.first()?;
    match tag {
        XBEN_TWODELTA_FULL_TAG => {
            if overflow.len() < 7 {
                return None;
            }
            let run_count =
                u32::from_be_bytes([overflow[1], overflow[2], overflow[3], overflow[4]]) as usize;
            // tag (1) + run_count field (4) + run_count * 4 payload bytes + trailing count (2).
            let total_len = 1 + 4 + run_count * 4 + 2;
            if overflow.len() < total_len {
                return None;
            }

            let mut runs = Vec::with_capacity(run_count);
            let mut cursor = 5usize;
            let mut expanded_len = 0u64;
            for _ in 0..run_count {
                let value = u16::from_be_bytes([overflow[cursor], overflow[cursor + 1]]);
                let len = u16::from_be_bytes([overflow[cursor + 2], overflow[cursor + 3]]);
                // The encoder never emits zero-length runs; tolerating one here would silently
                // diverge from the bit-packed TwoDelta path, which rejects them.
                if len == 0 {
                    return Some(Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "XBEN TwoDelta full frame contains a zero-length run for value {value}"
                        ),
                    )));
                }
                // Expansion sanity bound: each run can demand up to 65,535 elements, so the sum is
                // what a frame can force the reader to allocate.
                expanded_len += u64::from(len);
                if expanded_len > crate::codec::decode::MAX_ASSIGNMENT_LEN {
                    return Some(Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "XBEN TwoDelta full frame expands past the {} element sanity bound",
                            crate::codec::decode::MAX_ASSIGNMENT_LEN
                        ),
                    )));
                }
                runs.push((value, len));
                cursor += 4;
            }
            let count = u16::from_be_bytes([overflow[cursor], overflow[cursor + 1]]);
            Some(Ok((runs, total_len, count)))
        }
        XBEN_TWODELTA_CHUNK_TAG => None,
        _ => Some(Err(io::Error::from(DecodeError::XBenUnknownFrameTag {
            tag,
        }))),
    }
}

/// Try to parse a columnar TwoDelta chunk from the overflow buffer.
///
/// Returns `Some(Ok(()))` when a full chunk was decoded and pushed onto `chunk_queue`,
/// `Some(Err(..))` on a corrupt chunk (length arithmetic that would wrap, or a zero-length run),
/// and `None` when the overflow does not start with the chunk tag or does not yet hold the whole
/// chunk.
fn try_parse_twodelta_chunk<R: Read>(inner: &mut XBenInner<R>) -> Option<io::Result<()>> {
    if inner.overflow.first() != Some(&XBEN_TWODELTA_CHUNK_TAG) {
        return None;
    }
    if inner.overflow.len() < 5 {
        return None;
    }

    let n_frames = u32::from_be_bytes([
        inner.overflow[1],
        inner.overflow[2],
        inner.overflow[3],
        inner.overflow[4],
    ]) as usize;

    // Each frame contributes a 4-byte pair, a 2-byte count, and a 4-byte run count to the fixed
    // region. These are single `u32 * small constant` products, which cannot overflow a 64-bit
    // usize.
    let header_len: usize = 5;
    let pairs_len = n_frames * 4;
    let counts_len = n_frames * 2;
    let run_counts_len = n_frames * 4;
    let fixed_len = header_len + pairs_len + counts_len + run_counts_len;
    if inner.overflow.len() < fixed_len {
        return None;
    }

    let run_counts_start = header_len + pairs_len + counts_len;
    let mut total_runs: usize = 0;
    let mut run_counts = Vec::with_capacity(n_frames);
    for i in 0..n_frames {
        let offset = run_counts_start + i * 4;
        let rc = u32::from_be_bytes([
            inner.overflow[offset],
            inner.overflow[offset + 1],
            inner.overflow[offset + 2],
            inner.overflow[offset + 3],
        ]) as usize;
        run_counts.push(rc);
        // `total_runs` sums up to ~4.3 billion u32 values, so this is the one place the length math
        // can wrap a 64-bit usize; a wrap here would make `total_len` small and panic below.
        total_runs = match total_runs.checked_add(rc) {
            Some(t) => t,
            None => return Some(Err(twodelta_len_overflow())),
        };
    }

    let total_len = match total_runs
        .checked_mul(2)
        .and_then(|run_data| run_data.checked_add(fixed_len))
    {
        Some(n) => n,
        None => return Some(Err(twodelta_len_overflow())),
    };
    if inner.overflow.len() < total_len {
        return None;
    }

    let pairs_start = header_len;
    let counts_start = pairs_start + pairs_len;
    let run_data_start = run_counts_start + run_counts_len;

    // Decode every frame into a local buffer first, so a corrupt run length leaves `chunk_queue`
    // untouched rather than half-populated.
    let mut parsed = Vec::with_capacity(n_frames);
    let mut run_cursor = run_data_start;
    for (i, &rc) in run_counts.iter().enumerate() {
        let po = pairs_start + i * 4;
        let pair = (
            u16::from_be_bytes([inner.overflow[po], inner.overflow[po + 1]]),
            u16::from_be_bytes([inner.overflow[po + 2], inner.overflow[po + 3]]),
        );
        let co = counts_start + i * 2;
        let count = u16::from_be_bytes([inner.overflow[co], inner.overflow[co + 1]]);

        let mut run_lengths = Vec::with_capacity(rc);
        for _ in 0..rc {
            let len =
                u16::from_be_bytes([inner.overflow[run_cursor], inner.overflow[run_cursor + 1]]);
            // Match the full-frame path: reject a zero-length run at parse time rather than
            // deferring it to `apply_twodelta_runs_to_assignment`.
            if len == 0 {
                return Some(Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "XBEN TwoDelta chunk frame contains a zero-length run",
                )));
            }
            run_lengths.push(len);
            run_cursor += 2;
        }

        parsed.push((pair, run_lengths, count));
    }

    for frame in parsed {
        inner.chunk_queue.push_back(frame);
    }
    inner.overflow.drain(..total_len);
    Some(Ok(()))
}

/// Decode one raw ben32 frame from an XBEN stream into a full assignment vector.
fn decode_xben_frame_to_assignment(
    frame_bytes: &[u8],
    variant: BenVariant,
) -> io::Result<Vec<u16>> {
    let (assignment, _) = decode_ben32_line(Cursor::new(frame_bytes), variant)?;
    Ok(assignment)
}

pub(super) fn next_record_xben<R: Read>(
    inner: &mut XBenInner<R>,
    variant: BenVariant,
) -> Option<io::Result<MkvRecord>> {
    loop {
        match variant {
            BenVariant::Standard | BenVariant::MkvChain => {
                if let Some((frame_bytes, consumed, count)) =
                    pop_frame_from_overflow(variant, &inner.overflow)
                {
                    if count == 0 {
                        inner.overflow.drain(..consumed);
                        return Some(Err(zero_count_frame_error("XBEN")));
                    }
                    // The popped frame is structurally complete (sentinel found), but its runs can
                    // still be semantically corrupt (zero-length run, oversized expansion), so the
                    // decode is fallible.
                    let assignment = match decode_xben_frame_to_assignment(frame_bytes, variant) {
                        Ok(assignment) => assignment,
                        Err(e) => {
                            inner.overflow.drain(..consumed);
                            return Some(Err(e));
                        }
                    };
                    inner.previous_assignment = Some(assignment.clone());
                    inner.overflow.drain(..consumed);
                    return Some(Ok((assignment, count)));
                }
            }
            BenVariant::TwoDelta => {
                if let Some((pair, run_lengths, count)) = inner.chunk_queue.pop_front() {
                    if count == 0 {
                        return Some(Err(zero_count_frame_error("XBEN")));
                    }
                    let assignment = match inner.previous_assignment.take() {
                        Some(prev) => apply_twodelta_runs_to_assignment(prev, pair, &run_lengths),
                        None => Err(io::Error::from(DecodeError::TwoDeltaNoAnchorFrame)),
                    };
                    return Some(match assignment {
                        Ok(a) => {
                            inner.previous_assignment = Some(a.clone());
                            Ok((a, count))
                        }
                        Err(e) => Err(e),
                    });
                }

                match try_parse_twodelta_chunk(inner) {
                    Some(Ok(())) => continue,
                    Some(Err(e)) => return Some(Err(e)),
                    None => {}
                }

                if let Some(parsed) = pop_twodelta_frame_from_overflow(&inner.overflow) {
                    let res = match parsed {
                        Ok((runs, consumed, count)) => {
                            if count == 0 {
                                inner.overflow.drain(..consumed);
                                return Some(Err(zero_count_frame_error("XBEN")));
                            }
                            let assignment = rle_to_vec(runs);
                            inner.previous_assignment = Some(assignment.clone());
                            inner.overflow.drain(..consumed);
                            Ok((assignment, count))
                        }
                        Err(err) => {
                            inner.overflow.clear();
                            Err(err)
                        }
                    };
                    return Some(res);
                }
            }
        }

        let read = match inner.xz.read(&mut inner.buf) {
            Ok(0) => {
                if inner.overflow.is_empty() {
                    return None;
                } else {
                    return Some(Err(io::Error::from(DecodeError::XBenTruncated)));
                }
            }
            Ok(n) => n,
            Err(e) => return Some(Err(e)),
        };
        inner.overflow.extend_from_slice(&inner.buf[..read]);
    }
}

pub(super) fn for_each_assignment_xben<R: Read, F>(
    inner: &mut XBenInner<R>,
    variant: BenVariant,
    silent: bool,
    mut f: F,
) -> io::Result<()>
where
    F: FnMut(&[u16], u16) -> io::Result<bool>,
{
    let mut sample_count = 0usize;
    let spinner = (!silent).then(|| Spinner::new("Decoding sample"));
    loop {
        match next_record_xben(inner, variant) {
            Some(Ok((assignment, count))) => {
                sample_count += count as usize;
                if let Some(spinner) = &spinner {
                    spinner.set_count(sample_count as u64);
                }
                let keep_going = f(&assignment, count)?;
                if !keep_going {
                    return Ok(());
                }
            }
            Some(Err(e)) => return Err(e),
            None => return Ok(()),
        }
    }
}

pub(super) fn count_samples_xben<R: Read>(
    inner: XBenInner<R>,
    variant: BenVariant,
) -> io::Result<usize> {
    use super::frames::next_frame_xben;
    let mut inner = inner;
    let mut total = 0usize;
    while let Some(item) = next_frame_xben(&mut inner, variant) {
        let (_bytes, cnt) = item?;
        total += cnt as usize;
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::{pop_twodelta_frame_from_overflow, try_parse_twodelta_chunk};
    use crate::io::reader::stream_reader::XBenInner;
    use crate::io::reader::twodelta::{XBEN_TWODELTA_CHUNK_TAG, XBEN_TWODELTA_FULL_TAG};
    use std::collections::VecDeque;
    use std::io::{BufReader, Cursor, ErrorKind};
    use xz2::read::XzDecoder;

    fn inner_with_overflow(overflow: Vec<u8>) -> XBenInner<Cursor<Vec<u8>>> {
        // The xz reader is never touched: every test pre-fills `overflow` with the whole chunk.
        XBenInner {
            xz: BufReader::new(XzDecoder::new(Cursor::new(Vec::new()))),
            overflow,
            buf: vec![0u8; 64].into_boxed_slice(),
            previous_assignment: None,
            chunk_queue: VecDeque::new(),
        }
    }

    /// Build the columnar bytes for a TwoDelta chunk from `(pair, count, run_lengths)` frames.
    fn build_chunk(frames: &[((u16, u16), u16, Vec<u16>)]) -> Vec<u8> {
        let mut v = vec![XBEN_TWODELTA_CHUNK_TAG];
        v.extend_from_slice(&(frames.len() as u32).to_be_bytes());
        for ((a, b), _, _) in frames {
            v.extend_from_slice(&a.to_be_bytes());
            v.extend_from_slice(&b.to_be_bytes());
        }
        for (_, count, _) in frames {
            v.extend_from_slice(&count.to_be_bytes());
        }
        for (_, _, runs) in frames {
            v.extend_from_slice(&(runs.len() as u32).to_be_bytes());
        }
        for (_, _, runs) in frames {
            for &r in runs {
                v.extend_from_slice(&r.to_be_bytes());
            }
        }
        v
    }

    #[test]
    fn try_parse_twodelta_chunk_decodes_valid_chunk() {
        let overflow = build_chunk(&[((1u16, 2u16), 1u16, vec![3u16, 4u16])]);
        let mut inner = inner_with_overflow(overflow);
        assert!(matches!(try_parse_twodelta_chunk(&mut inner), Some(Ok(()))));
        assert_eq!(inner.chunk_queue.len(), 1);
        assert_eq!(inner.chunk_queue[0], ((1u16, 2u16), vec![3u16, 4u16], 1u16));
        assert!(
            inner.overflow.is_empty(),
            "the whole chunk should be drained"
        );
    }

    #[test]
    fn try_parse_twodelta_chunk_rejects_zero_run() {
        // A single zero-length run must be rejected at parse, and must leave `chunk_queue` empty
        // (the decode is transactional).
        let overflow = build_chunk(&[((1u16, 2u16), 1u16, vec![0u16])]);
        let mut inner = inner_with_overflow(overflow);
        match try_parse_twodelta_chunk(&mut inner) {
            Some(Err(e)) => assert_eq!(e.kind(), ErrorKind::InvalidData),
            other => panic!("expected InvalidData error, got {other:?}"),
        }
        assert!(inner.chunk_queue.is_empty());
    }

    #[test]
    fn try_parse_twodelta_chunk_huge_n_frames_is_incomplete_not_a_panic() {
        // n_frames = u32::MAX makes the fixed region enormous; with only a few bytes buffered the
        // parser must report "incomplete" without allocating or panicking on the length arithmetic.
        let mut overflow = vec![XBEN_TWODELTA_CHUNK_TAG];
        overflow.extend_from_slice(&u32::MAX.to_be_bytes());
        overflow.extend_from_slice(&[0u8; 4]);
        let mut inner = inner_with_overflow(overflow);
        assert!(try_parse_twodelta_chunk(&mut inner).is_none());
    }

    #[test]
    fn pop_twodelta_full_frame_huge_run_count_is_incomplete_not_a_panic() {
        let mut overflow = vec![XBEN_TWODELTA_FULL_TAG];
        overflow.extend_from_slice(&u32::MAX.to_be_bytes());
        overflow.extend_from_slice(&[0u8; 4]);
        assert!(pop_twodelta_frame_from_overflow(&overflow).is_none());
    }
}
