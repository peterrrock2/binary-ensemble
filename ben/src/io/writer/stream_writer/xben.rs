//! XBEN encode logic for the unified stream writer.

use std::collections::HashMap;
use std::io::{self, BufRead, Read, Write};

use byteorder::{BigEndian, ReadBytesExt};
use xz2::write::XzEncoder;

use crate::codec::decode::decode_ben_line;
use crate::codec::encode::errors::is_twodelta_run_too_long;
use crate::codec::encode::{encode_ben32_assignments, encode_twodelta_frame_with_hint};
use crate::codec::frames::{check_payload_len, check_twodelta_run_width};
use crate::codec::translate::ben_to_ben32_lines;
use crate::codec::BenEncodeFrame;
use crate::format::banners::{has_known_banner_prefix, BANNER_LEN};
use crate::progress::Spinner;
use crate::BenVariant;

use super::super::frames::BufferedDeltaFrame;
use super::super::twodelta::{
    classify_transition, pair_has_masks, twodelta_repeat_runs, TransitionKind,
    BEN_TWODELTA_DELTA_TAG, BEN_TWODELTA_SNAPSHOT_TAG, XBEN_TWODELTA_CHUNK_TAG,
    XBEN_TWODELTA_FULL_TAG,
};
use super::super::utils::encode_xben_twodelta_full_frame;

/// XBEN-arm state. Owns the xz encoder and a per-variant inner state.
pub(super) struct XBenInner<W: Write> {
    pub(super) encoder: XzEncoder<W>,
    pub(super) state: XBenState,
}

/// Per-variant inner state. Variant lives here as the single source of truth.
pub(super) enum XBenState {
    Standard,
    MkvChain {
        pending_assignment: Option<Vec<u16>>,
        pending_count: u16,
    },
    TwoDelta {
        previous_assignment: Vec<u16>,
        previous_masks: HashMap<u16, Vec<usize>>,
        /// A full frame buffered awaiting its final repetition count. Used both for the initial
        /// anchor and for mid-stream snapshots (>2-district transitions). A full frame writes its
        /// count *after* the payload, so it cannot be emitted until a distinct assignment arrives.
        pending_full_assignment: Option<Vec<u16>>,
        pending_full_count: u16,
        twodelta_chunk_size: usize,
        chunk_buffer: Vec<BufferedDeltaFrame>,
    },
}

impl XBenState {
    pub(super) fn new(variant: BenVariant, twodelta_chunk_size: usize) -> Self {
        match variant {
            BenVariant::Standard => XBenState::Standard,
            BenVariant::MkvChain => XBenState::MkvChain {
                pending_assignment: None,
                pending_count: 0,
            },
            BenVariant::TwoDelta => XBenState::TwoDelta {
                previous_assignment: Vec::new(),
                previous_masks: HashMap::new(),
                pending_full_assignment: None,
                pending_full_count: 0,
                twodelta_chunk_size,
                chunk_buffer: Vec::new(),
            },
        }
    }

    pub(super) fn variant(&self) -> BenVariant {
        match self {
            XBenState::Standard => BenVariant::Standard,
            XBenState::MkvChain { .. } => BenVariant::MkvChain,
            XBenState::TwoDelta { .. } => BenVariant::TwoDelta,
        }
    }
}

impl<W: Write> XBenInner<W> {
    pub(super) fn new(
        encoder: XzEncoder<W>,
        variant: BenVariant,
        twodelta_chunk_size: usize,
    ) -> Self {
        Self {
            encoder,
            state: XBenState::new(variant, twodelta_chunk_size),
        }
    }

    pub(super) fn variant(&self) -> BenVariant {
        self.state.variant()
    }

    pub(super) fn write_assignment(&mut self, assign_vec: Vec<u16>) -> io::Result<()> {
        match &mut self.state {
            XBenState::Standard => {
                let encoded = encode_ben32_assignments(&assign_vec)?;
                self.encoder.write_all(&encoded)?;
            }
            XBenState::MkvChain {
                pending_assignment,
                pending_count,
            } => {
                if pending_assignment.as_deref() == Some(assign_vec.as_slice()) {
                    if *pending_count == u16::MAX {
                        flush_mkv_pending(&mut self.encoder, pending_assignment, pending_count)?;
                        *pending_assignment = Some(assign_vec);
                        *pending_count = 1;
                        return Ok(());
                    }
                    *pending_count += 1;
                    return Ok(());
                }
                flush_mkv_pending(&mut self.encoder, pending_assignment, pending_count)?;
                *pending_assignment = Some(assign_vec);
                *pending_count = 1;
            }
            XBenState::TwoDelta {
                previous_assignment,
                previous_masks,
                pending_full_assignment,
                pending_full_count,
                twodelta_chunk_size,
                chunk_buffer,
            } => {
                // First assignment ever: buffer as the initial full frame.
                if pending_full_assignment.is_none() && previous_assignment.is_empty() {
                    *pending_full_assignment = Some(assign_vec);
                    *pending_full_count = 1;
                    return Ok(());
                }
                // Repeat of the pending full frame (initial anchor or a mid-stream snapshot).
                if pending_full_assignment.as_deref() == Some(assign_vec.as_slice()) {
                    if *pending_full_count == u16::MAX {
                        flush_twodelta_full(
                            &mut self.encoder,
                            pending_full_assignment,
                            pending_full_count,
                            previous_assignment,
                            previous_masks,
                        )?;
                        match twodelta_repeat_buffered_frame(&assign_vec, 1) {
                            Ok(repeat) => {
                                chunk_buffer.push(repeat);
                                *previous_assignment = assign_vec;
                            }
                            // The pair-projected run exceeds u16::MAX, so the repeat cannot be a
                            // delta-shaped frame; re-buffer it as a fresh pending full frame,
                            // which splits long runs natively and keeps merging later repeats.
                            Err(e) if is_twodelta_run_too_long(&e) => {
                                *pending_full_assignment = Some(assign_vec);
                                *pending_full_count = 1;
                            }
                            Err(e) => return Err(e),
                        }
                        return Ok(());
                    }
                    *pending_full_count += 1;
                    return Ok(());
                }
                // Repeat of the last delta frame in the current chunk.
                if !chunk_buffer.is_empty()
                    && previous_assignment.as_slice() == assign_vec.as_slice()
                {
                    if chunk_buffer.last().unwrap().count == u16::MAX {
                        flush_chunk_inner(&mut self.encoder, chunk_buffer)?;
                        match twodelta_repeat_buffered_frame(&assign_vec, 1) {
                            Ok(repeat) => chunk_buffer.push(repeat),
                            // Same representability limit as the pending-full repeat path: defer
                            // as a pending full frame (the chunk was just flushed, so the full
                            // frame correctly follows the chunk's deltas in the stream).
                            Err(e) if is_twodelta_run_too_long(&e) => {
                                *pending_full_assignment = Some(assign_vec);
                                *pending_full_count = 1;
                            }
                            Err(e) => return Err(e),
                        }
                    } else {
                        chunk_buffer.last_mut().unwrap().count += 1;
                    }
                    return Ok(());
                }
                // New distinct assignment: flush a pending full frame so it precedes the new body.
                if pending_full_assignment.is_some() {
                    flush_twodelta_full(
                        &mut self.encoder,
                        pending_full_assignment,
                        pending_full_count,
                        previous_assignment,
                        previous_masks,
                    )?;
                }
                // Classify the transition; fall back to a deferred snapshot when >2 ids change.
                match classify_transition(previous_assignment, &assign_vec)? {
                    // `previous == assign_vec` only reaches here when the chunk was just flushed
                    // (so the repeat-of-last-delta fast path above was skipped). Encode it as a
                    // repeat delta against the previous frame.
                    TransitionKind::Repeat => {
                        match twodelta_repeat_buffered_frame(&assign_vec, 1) {
                            Ok(repeat) => {
                                chunk_buffer.push(repeat);
                                *previous_assignment = assign_vec;
                            }
                            // Same representability limit as the saturation paths: defer as a
                            // pending full frame. `previous_assignment`
                            // already equals the repeated value.
                            Err(e) if is_twodelta_run_too_long(&e) => {
                                flush_chunk_inner(&mut self.encoder, chunk_buffer)?;
                                *pending_full_assignment = Some(assign_vec);
                                *pending_full_count = 1;
                            }
                            Err(e) => return Err(e),
                        }
                    }
                    // Clean 2-swap where both districts already exist: cheap delta.
                    TransitionKind::Delta(a, b) if pair_has_masks(previous_masks, a, b) => {
                        match encode_twodelta_frame_with_hint(
                            &*previous_assignment,
                            &assign_vec,
                            Some((a, b)),
                            Some(previous_masks),
                            None,
                        ) {
                            Ok(frame) => {
                                let (pair, run_lengths) = match frame {
                                    BenEncodeFrame::TwoDelta {
                                        pair,
                                        run_length_vector,
                                        ..
                                    } => (pair, run_length_vector),
                                    _ => unreachable!(
                                        "encode_twodelta_frame_with_hint always returns the \
                                         TwoDelta arm"
                                    ),
                                };
                                chunk_buffer.push(BufferedDeltaFrame {
                                    pair,
                                    run_lengths,
                                    count: 1,
                                });
                                *previous_assignment = assign_vec;
                                if chunk_buffer.len() >= *twodelta_chunk_size {
                                    flush_chunk_inner(&mut self.encoder, chunk_buffer)?;
                                }
                            }
                            // The delta's pair-projected run exceeds u16::MAX: defer as a pending
                            // full frame, exactly like the Snapshot arm below. The failed encode
                            // leaves `previous_masks` untouched, and `flush_twodelta_full`
                            // reseeds them when the full frame is emitted.
                            Err(e) if is_twodelta_run_too_long(&e) => {
                                flush_chunk_inner(&mut self.encoder, chunk_buffer)?;
                                *pending_full_assignment = Some(assign_vec);
                                *pending_full_count = 1;
                            }
                            Err(e) => return Err(e),
                        }
                    }
                    // A >2-district transition, or a 2-id transition introducing a district absent
                    // from the previous assignment: defer it as a pending full frame. Flush the
                    // current chunk first so its deltas precede the snapshot in the stream. The
                    // full frame's count is written after its payload, so it cannot be emitted
                    // until a following distinct assignment (or `flush`) settles the count.
                    TransitionKind::Delta(..) | TransitionKind::Snapshot => {
                        flush_chunk_inner(&mut self.encoder, chunk_buffer)?;
                        *pending_full_assignment = Some(assign_vec);
                        *pending_full_count = 1;
                    }
                }
            }
        }
        Ok(())
    }

    /// Flush all buffered XBEN state in preparation for `try_finish`.
    pub(super) fn flush(&mut self) -> io::Result<()> {
        match &mut self.state {
            XBenState::Standard => Ok(()),
            XBenState::MkvChain {
                pending_assignment,
                pending_count,
            } => flush_mkv_pending(&mut self.encoder, pending_assignment, pending_count),
            XBenState::TwoDelta {
                previous_assignment,
                previous_masks,
                pending_full_assignment,
                pending_full_count,
                chunk_buffer,
                ..
            } => {
                // At most one of these is non-empty: buffering a pending full frame always flushes
                // the chunk first, and pushing a delta clears any pending full frame.
                flush_twodelta_full(
                    &mut self.encoder,
                    pending_full_assignment,
                    pending_full_count,
                    previous_assignment,
                    previous_masks,
                )?;
                flush_chunk_inner(&mut self.encoder, chunk_buffer)
            }
        }
    }

    /// Translate a BEN TwoDelta stream directly to XBEN TwoDelta without materializing full
    /// assignment vectors.
    fn translate_ben_twodelta_to_xben(&mut self, mut reader: impl Read) -> io::Result<()> {
        let chunk_size = match &self.state {
            XBenState::TwoDelta {
                twodelta_chunk_size,
                ..
            } => *twodelta_chunk_size,
            _ => unreachable!("translate_ben_twodelta_to_xben requires TwoDelta state"),
        };
        let chunk_buffer = match &mut self.state {
            XBenState::TwoDelta { chunk_buffer, .. } => chunk_buffer,
            _ => unreachable!(),
        };

        let mut sample_count = 0usize;
        let spinner = Spinner::new("Encoding line");

        // Each BEN frame is prefixed with a per-frame tag. This path keeps no masks and
        // materializes no assignments, so there is nothing to reset across a snapshot — it simply
        // mirrors the BEN framing onto the XBEN columnar layout.
        loop {
            let tag = match reader.read_u8() {
                Ok(t) => t,
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e),
            };

            match tag {
                // Snapshot: a MkvChain-formatted body → XBEN full frame. Flush the current chunk
                // first so its deltas precede the full frame in the stream.
                BEN_TWODELTA_SNAPSHOT_TAG => {
                    flush_chunk_inner(&mut self.encoder, chunk_buffer)?;

                    let max_val_bits = reader.read_u8()?;
                    let max_len_bits = reader.read_u8()?;
                    let n_bytes = reader.read_u32::<BigEndian>()?;
                    let runs = decode_ben_line(&mut reader, max_val_bits, max_len_bits, n_bytes)?;
                    let count = reader.read_u16::<BigEndian>()?;

                    let mut encoded = Vec::with_capacity(1 + 4 + runs.len() * 4);
                    encoded.push(XBEN_TWODELTA_FULL_TAG);
                    encoded.extend_from_slice(&(runs.len() as u32).to_be_bytes());
                    for &(value, len) in &runs {
                        encoded.extend_from_slice(&value.to_be_bytes());
                        encoded.extend_from_slice(&len.to_be_bytes());
                    }
                    self.encoder.write_all(&encoded)?;
                    self.encoder.write_all(&count.to_be_bytes())?;

                    sample_count += count as usize;
                    spinner.set_count(sample_count as u64);
                }
                // Delta: unpack the bit-packed run lengths and buffer into the current chunk. The
                // input stream is untrusted, so the header fields are validated (bit width,
                // payload cap) before the payload buffer is allocated, and the strict constructor
                // rejects corrupt payloads instead of silently dropping zero run lengths.
                BEN_TWODELTA_DELTA_TAG => {
                    let pair_a = reader.read_u16::<BigEndian>()?;
                    let pair_b = reader.read_u16::<BigEndian>()?;
                    let delta_max_len_bits = reader.read_u8()?;
                    check_twodelta_run_width(delta_max_len_bits)?;
                    let delta_n_bytes = reader.read_u32::<BigEndian>()?;
                    check_payload_len(delta_n_bytes)?;

                    let mut payload = vec![0u8; delta_n_bytes as usize];
                    reader.read_exact(&mut payload)?;
                    let count = reader.read_u16::<BigEndian>()?;

                    let frame = BenEncodeFrame::try_from_parts(
                        (pair_a, pair_b),
                        delta_max_len_bits,
                        payload,
                        count,
                    )?;
                    let (pair, run_lengths) = match frame {
                        BenEncodeFrame::TwoDelta {
                            pair,
                            run_length_vector,
                            ..
                        } => (pair, run_length_vector),
                        _ => unreachable!("try_from_parts always returns TwoDelta"),
                    };

                    chunk_buffer.push(BufferedDeltaFrame {
                        pair,
                        run_lengths,
                        count,
                    });

                    if chunk_buffer.len() >= chunk_size {
                        flush_chunk_inner(&mut self.encoder, chunk_buffer)?;
                    }

                    sample_count += count as usize;
                    spinner.set_count(sample_count as u64);
                }
                other => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("unknown TwoDelta frame tag byte {other:#04x}"),
                    ))
                }
            }
        }

        flush_chunk_inner(&mut self.encoder, chunk_buffer)?;
        Ok(())
    }

    /// Crate-private direct ingest entry point.
    ///
    /// Standard/MkvChain accept bannered or bannerless input; TwoDelta requires a banner.
    pub(super) fn ingest_ben_stream(&mut self, mut reader: impl BufRead) -> io::Result<()> {
        let peek = reader.fill_buf()?;
        let has_banner = peek.len() >= BANNER_LEN && has_known_banner_prefix(peek);

        let variant = self.variant();

        if has_banner {
            if variant == BenVariant::TwoDelta {
                reader.consume(BANNER_LEN);
                return self.translate_ben_twodelta_to_xben(reader);
            }
            reader.consume(BANNER_LEN);
        }

        let xben_variant = match crate::XBenVariant::try_from(variant) {
            Ok(v) => v,
            Err(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "TwoDelta XBEN translation requires a BEN stream with its banner",
                ));
            }
        };

        ben_to_ben32_lines(&mut reader, &mut self.encoder, xben_variant)
    }
}

fn flush_mkv_pending<W: Write>(
    encoder: &mut XzEncoder<W>,
    pending_assignment: &mut Option<Vec<u16>>,
    pending_count: &mut u16,
) -> io::Result<()> {
    let pending = match pending_assignment.take() {
        Some(p) => p,
        None => return Ok(()),
    };
    let count = *pending_count;
    *pending_count = 0;
    let encoded = encode_ben32_assignments(&pending)?;
    encoder.write_all(&encoded)?;
    encoder.write_all(&count.to_be_bytes())?;
    Ok(())
}

/// Emit a buffered full frame (payload then trailing count) and rebase the delta state onto it.
///
/// Used for both the initial anchor and mid-stream snapshots, so `previous_masks` is cleared before
/// reseeding rather than only pushed onto — the map is non-empty after the first frame.
fn flush_twodelta_full<W: Write>(
    encoder: &mut XzEncoder<W>,
    pending_full_assignment: &mut Option<Vec<u16>>,
    pending_full_count: &mut u16,
    previous_assignment: &mut Vec<u16>,
    previous_masks: &mut HashMap<u16, Vec<usize>>,
) -> io::Result<()> {
    let pending = match pending_full_assignment.take() {
        Some(p) => p,
        None => return Ok(()),
    };
    let count = *pending_full_count;
    *pending_full_count = 0;

    previous_masks.clear();
    for (idx, &val) in pending.iter().enumerate() {
        previous_masks.entry(val).or_default().push(idx);
    }
    let encoded = encode_xben_twodelta_full_frame(&pending);
    encoder.write_all(&encoded)?;
    encoder.write_all(&count.to_be_bytes())?;
    *previous_assignment = pending;
    Ok(())
}

fn flush_chunk_inner<W: Write>(
    encoder: &mut XzEncoder<W>,
    chunk_buffer: &mut Vec<BufferedDeltaFrame>,
) -> io::Result<()> {
    if chunk_buffer.is_empty() {
        return Ok(());
    }

    let n = chunk_buffer.len() as u32;
    encoder.write_all(&[XBEN_TWODELTA_CHUNK_TAG])?;
    encoder.write_all(&n.to_be_bytes())?;

    for frame in chunk_buffer.iter() {
        encoder.write_all(&frame.pair.0.to_be_bytes())?;
        encoder.write_all(&frame.pair.1.to_be_bytes())?;
    }
    for frame in chunk_buffer.iter() {
        encoder.write_all(&frame.count.to_be_bytes())?;
    }
    for frame in chunk_buffer.iter() {
        encoder.write_all(&(frame.run_lengths.len() as u32).to_be_bytes())?;
    }
    for frame in chunk_buffer.iter() {
        for &rl in &frame.run_lengths {
            encoder.write_all(&rl.to_be_bytes())?;
        }
    }

    chunk_buffer.clear();
    Ok(())
}

pub(crate) fn twodelta_repeat_buffered_frame(
    assignment: &[u16],
    count: u16,
) -> io::Result<BufferedDeltaFrame> {
    let (pair, run_lengths) = twodelta_repeat_runs(assignment)?;
    Ok(BufferedDeltaFrame {
        pair,
        run_lengths,
        count,
    })
}
