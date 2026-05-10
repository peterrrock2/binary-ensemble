//! XBEN encode logic for the unified stream writer.

use std::collections::HashMap;
use std::io::{self, BufRead, Read, Write};

use byteorder::{BigEndian, ReadBytesExt};
use xz2::write::XzEncoder;

use crate::codec::decode::decode_ben_line;
use crate::codec::encode::{encode_ben32_assignments, encode_twodelta_frame_with_hint};
use crate::codec::translate::ben_to_ben32_lines;
use crate::codec::BenEncodeFrame;
use crate::format::banners::{has_known_banner_prefix, BANNER_LEN};
use crate::progress::Spinner;
use crate::BenVariant;

use super::super::frames::BufferedDeltaFrame;
use super::super::twodelta::{
    twodelta_repeat_runs, XBEN_TWODELTA_CHUNK_TAG, XBEN_TWODELTA_FULL_TAG,
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
        pending_initial_full_assignment: Option<Vec<u16>>,
        pending_initial_full_count: u16,
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
                pending_initial_full_assignment: None,
                pending_initial_full_count: 0,
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
    pub(super) fn new(encoder: XzEncoder<W>, variant: BenVariant, twodelta_chunk_size: usize) -> Self {
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
                pending_initial_full_assignment,
                pending_initial_full_count,
                twodelta_chunk_size,
                chunk_buffer,
            } => {
                // First assignment ever: buffer as the initial full frame.
                if pending_initial_full_assignment.is_none() && previous_assignment.is_empty() {
                    *pending_initial_full_assignment = Some(assign_vec);
                    *pending_initial_full_count = 1;
                    return Ok(());
                }
                // Repeat of the pending initial full frame.
                if pending_initial_full_assignment.as_deref() == Some(assign_vec.as_slice()) {
                    if *pending_initial_full_count == u16::MAX {
                        flush_twodelta_initial(
                            &mut self.encoder,
                            pending_initial_full_assignment,
                            pending_initial_full_count,
                            previous_assignment,
                            previous_masks,
                        )?;
                        let repeat = twodelta_repeat_buffered_frame(&assign_vec, 1)?;
                        chunk_buffer.push(repeat);
                        *previous_assignment = assign_vec;
                        return Ok(());
                    }
                    *pending_initial_full_count += 1;
                    return Ok(());
                }
                // Repeat of the last delta frame in the current chunk.
                if !chunk_buffer.is_empty()
                    && previous_assignment.as_slice() == assign_vec.as_slice()
                {
                    if chunk_buffer.last().unwrap().count == u16::MAX {
                        flush_chunk_inner(&mut self.encoder, chunk_buffer)?;
                        let repeat = twodelta_repeat_buffered_frame(&assign_vec, 1)?;
                        chunk_buffer.push(repeat);
                    } else {
                        chunk_buffer.last_mut().unwrap().count += 1;
                    }
                    return Ok(());
                }
                // New distinct assignment: flush the initial full frame if pending.
                if pending_initial_full_assignment.is_some() {
                    flush_twodelta_initial(
                        &mut self.encoder,
                        pending_initial_full_assignment,
                        pending_initial_full_count,
                        previous_assignment,
                        previous_masks,
                    )?;
                }
                // Encode the delta frame and add it to the chunk buffer.
                let frame = encode_twodelta_frame_with_hint(
                    &*previous_assignment,
                    &assign_vec,
                    None,
                    Some(previous_masks),
                    None,
                )?;
                let (pair, run_lengths) = match frame {
                    BenEncodeFrame::TwoDelta {
                        pair,
                        run_length_vector,
                        ..
                    } => (pair, run_length_vector),
                    _ => unreachable!(
                        "encode_twodelta_frame_with_hint always returns the TwoDelta arm"
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
                pending_initial_full_assignment,
                pending_initial_full_count,
                chunk_buffer,
                ..
            } => {
                flush_twodelta_initial(
                    &mut self.encoder,
                    pending_initial_full_assignment,
                    pending_initial_full_count,
                    previous_assignment,
                    previous_masks,
                )?;
                flush_chunk_inner(&mut self.encoder, chunk_buffer)
            }
        }
    }

    /// Translate a BEN TwoDelta stream directly to XBEN TwoDelta without
    /// materializing full assignment vectors.
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

        // First frame: standard BEN RLE → XBEN full frame.
        let max_val_bits = reader.read_u8()?;
        let max_len_bits = reader.read_u8()?;
        let n_bytes = reader.read_u32::<BigEndian>()?;
        let runs = decode_ben_line(&mut reader, max_val_bits, max_len_bits, n_bytes)?;
        let first_count = reader.read_u16::<BigEndian>()?;

        let mut encoded = Vec::with_capacity(1 + 4 + runs.len() * 4);
        encoded.push(XBEN_TWODELTA_FULL_TAG);
        encoded.extend_from_slice(&(runs.len() as u32).to_be_bytes());
        for &(value, len) in &runs {
            encoded.extend_from_slice(&value.to_be_bytes());
            encoded.extend_from_slice(&len.to_be_bytes());
        }
        self.encoder.write_all(&encoded)?;
        self.encoder.write_all(&first_count.to_be_bytes())?;

        let mut sample_count = first_count as usize;
        let spinner = Spinner::new("Encoding line");
        spinner.set_count(sample_count as u64);

        // Delta frames: unpack bitpacked run lengths and buffer into chunks.
        loop {
            let pair_a = match reader.read_u16::<BigEndian>() {
                Ok(v) => v,
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e),
            };
            let pair_b = reader.read_u16::<BigEndian>()?;
            let delta_max_len_bits = reader.read_u8()?;
            let delta_n_bytes = reader.read_u32::<BigEndian>()?;

            let mut payload = vec![0u8; delta_n_bytes as usize];
            reader.read_exact(&mut payload)?;
            let count = reader.read_u16::<BigEndian>()?;

            let (pair, run_lengths) = match BenEncodeFrame::from_parts(
                (pair_a, pair_b),
                delta_max_len_bits,
                payload,
                count,
            ) {
                BenEncodeFrame::TwoDelta {
                    pair,
                    run_length_vector,
                    ..
                } => (pair, run_length_vector),
                _ => unreachable!("BenEncodeFrame::from_parts always returns TwoDelta"),
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

        flush_chunk_inner(&mut self.encoder, chunk_buffer)?;
        Ok(())
    }

    /// Crate-private direct ingest entry point.
    ///
    /// Standard/MkvChain accept bannered or bannerless input; TwoDelta
    /// requires a banner.
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

fn flush_twodelta_initial<W: Write>(
    encoder: &mut XzEncoder<W>,
    pending_initial_full_assignment: &mut Option<Vec<u16>>,
    pending_initial_full_count: &mut u16,
    previous_assignment: &mut Vec<u16>,
    previous_masks: &mut HashMap<u16, Vec<usize>>,
) -> io::Result<()> {
    let pending = match pending_initial_full_assignment.take() {
        Some(p) => p,
        None => return Ok(()),
    };
    let count = *pending_initial_full_count;
    *pending_initial_full_count = 0;

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
