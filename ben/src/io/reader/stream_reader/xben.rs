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
            let payload_len = run_count * 4;
            let total_len = 1 + 4 + payload_len + 2;
            if overflow.len() < total_len {
                return None;
            }

            let mut runs = Vec::with_capacity(run_count);
            let mut cursor = 5usize;
            for _ in 0..run_count {
                let value = u16::from_be_bytes([overflow[cursor], overflow[cursor + 1]]);
                let len = u16::from_be_bytes([overflow[cursor + 2], overflow[cursor + 3]]);
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
/// If the overflow starts with the chunk tag and contains enough bytes for the full chunk, all
/// frames are decoded and pushed onto `chunk_queue`. Returns `true` on success, `false` when the
/// overflow is incomplete.
fn try_parse_twodelta_chunk<R: Read>(inner: &mut XBenInner<R>) -> bool {
    if inner.overflow.first() != Some(&XBEN_TWODELTA_CHUNK_TAG) {
        return false;
    }
    if inner.overflow.len() < 5 {
        return false;
    }

    let n_frames = u32::from_be_bytes([
        inner.overflow[1],
        inner.overflow[2],
        inner.overflow[3],
        inner.overflow[4],
    ]) as usize;

    let header_len: usize = 5;
    let pairs_len = n_frames * 4;
    let counts_len = n_frames * 2;
    let run_counts_len = n_frames * 4;
    let fixed_len = header_len + pairs_len + counts_len + run_counts_len;

    if inner.overflow.len() < fixed_len {
        return false;
    }

    let run_counts_start = header_len + pairs_len + counts_len;
    let mut total_runs = 0usize;
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
        total_runs += rc;
    }

    let run_data_len = total_runs * 2;
    let total_len = fixed_len + run_data_len;
    if inner.overflow.len() < total_len {
        return false;
    }

    let pairs_start = header_len;
    let counts_start = pairs_start + pairs_len;
    let run_data_start = run_counts_start + run_counts_len;

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
            run_lengths.push(u16::from_be_bytes([
                inner.overflow[run_cursor],
                inner.overflow[run_cursor + 1],
            ]));
            run_cursor += 2;
        }

        inner.chunk_queue.push_back((pair, run_lengths, count));
    }

    inner.overflow.drain(..total_len);
    true
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
                    let assignment = decode_xben_frame_to_assignment(frame_bytes, variant)
                        .expect("complete frame from pop_frame_from_overflow");
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

                if try_parse_twodelta_chunk(inner) {
                    continue;
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
