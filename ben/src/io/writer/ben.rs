use super::frames::{AssignmentHints, BufferedBenFrame, BufferedDeltaFrame};
use super::twodelta::{
    DEFAULT_TWODELTA_CHUNK_SIZE, XBEN_TWODELTA_CHUNK_TAG, XBEN_TWODELTA_FULL_TAG,
};
use super::utils::{
    analyze_twodelta_transition, encode_xben_twodelta_full_frame, is_repeated_assignment,
    parse_json_assignment,
};
use crate::codec::decode::decode_ben_line;
use crate::codec::encode::{encode_ben32_assignments, encode_twodelta_frame_with_hint};
use crate::codec::translate::ben_to_ben32_lines;
use crate::codec::{BenEncodeFrame, FromAssign, TwoDeltaFrame};
use crate::format::banners::{banner_for_variant, has_known_banner_prefix, BANNER_LEN};
use crate::{progress, BenVariant};
use byteorder::{BigEndian, ReadBytesExt};
use serde_json::Value;
use std::collections::HashMap;
use std::io::{self, BufRead, Read, Result, Write};
use xz2::write::XzEncoder;

/// A struct to make the writing of BEN files easier and more ergonomic.
pub struct BenEncoder<W: Write> {
    writer: W,
    previous_sample: Vec<u16>,
    previous_masks: HashMap<u16, Vec<usize>>,
    previous_encoded_sample: Option<BufferedBenFrame>,
    sample_count: u16,
    variant: BenVariant,
    complete: bool,
}

impl<W: Write> BenEncoder<W> {
    /// Create a new BEN writer and immediately emit the BEN banner.
    ///
    /// # Arguments
    ///
    /// * `writer` - The destination that will receive the BEN stream.
    /// * `variant` - The BEN variant to encode.
    ///
    /// # Returns
    ///
    /// Returns a new encoder ready to accept assignments or RLE frames.
    pub fn new(mut writer: W, variant: BenVariant) -> io::Result<Self> {
        writer.write_all(banner_for_variant(variant))?;

        Ok(BenEncoder {
            writer,
            previous_sample: Vec::new(),
            previous_masks: HashMap::new(),
            previous_encoded_sample: None,
            sample_count: 0,
            complete: false,
            variant,
        })
    }

    /// Rebuild the value-to-position index map from the current previous sample.
    fn rebuild_previous_masks(&mut self) {
        self.previous_masks.clear();
        for (idx, &assignment) in self.previous_sample.iter().enumerate() {
            self.previous_masks.entry(assignment).or_default().push(idx);
        }
    }

    /// Store a new previous sample along with its encoded frame and repetition count.
    ///
    /// # Arguments
    ///
    /// * `sample` - The assignment vector to cache.
    /// * `encoded` - The already-encoded frame for this assignment.
    /// * `sample_count` - The initial repetition count for this sample.
    fn set_previous_sample(
        &mut self,
        sample: Vec<u16>,
        encoded: BufferedBenFrame,
        sample_count: u16,
    ) {
        self.previous_sample = sample;
        self.rebuild_previous_masks();
        self.previous_encoded_sample = Some(encoded);
        self.sample_count = sample_count;
    }

    /// Encode and write an assignment vector using pre-computed transition hints.
    ///
    /// The encoding strategy depends on the configured `BenVariant`. Repeated
    /// assignments may be deduplicated or counted, and two-delta hints enable
    /// compact delta frames when applicable.
    ///
    /// # Arguments
    ///
    /// * `assign_vec` - The assignment vector to encode.
    /// * `hints` - Pre-computed hints about repetition and delta-pair eligibility.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` after the assignment has been queued or written.
    fn write_assignment_with_hints(
        &mut self,
        assign_vec: Vec<u16>,
        hints: AssignmentHints,
    ) -> Result<()> {
        match self.variant {
            BenVariant::Standard => {
                let repeated = is_repeated_assignment(&self.previous_sample, &assign_vec);
                if hints.is_repeated {
                    if let Some(encoded) = self.previous_encoded_sample.as_ref() {
                        self.writer.write_all(encoded.as_slice())?;
                        self.previous_sample = assign_vec;
                        return Ok(());
                    }
                }

                if repeated {
                    if let Some(encoded) = self.previous_encoded_sample.as_ref() {
                        self.writer.write_all(encoded.as_slice())?;
                        self.previous_sample = assign_vec;
                        return Ok(());
                    }
                }

                let encoded = BenEncodeFrame::from_assignment(&assign_vec, None);
                self.writer.write_all(encoded.as_slice())?;
                self.set_previous_sample(assign_vec, BufferedBenFrame::Ben(encoded), 0);
                Ok(())
            }
            BenVariant::MkvChain => {
                if is_repeated_assignment(&self.previous_sample, &assign_vec) {
                    self.sample_count += 1;
                    return Ok(());
                }

                if self.sample_count > 0 {
                    self.flush_pending_frame()?;
                }

                let encoded = BenEncodeFrame::from_assignment(&assign_vec, None);
                self.set_previous_sample(assign_vec, BufferedBenFrame::Ben(encoded), 1);
                Ok(())
            }
            BenVariant::TwoDelta => {
                if self.previous_sample.is_empty() {
                    let encoded = BenEncodeFrame::from_assignment(&assign_vec, None);
                    self.set_previous_sample(assign_vec, BufferedBenFrame::Ben(encoded), 1);
                    return Ok(());
                }

                if hints.is_repeated {
                    self.sample_count += 1;
                    return Ok(());
                }

                let encoded = encode_twodelta_frame_with_hint(
                    &self.previous_sample,
                    &assign_vec,
                    hints.delta_pair,
                    Some(&mut self.previous_masks),
                )?;
                self.flush_pending_frame()?;

                self.previous_sample = assign_vec;
                self.rebuild_previous_masks();
                self.previous_encoded_sample = Some(BufferedBenFrame::TwoDelta(encoded));
                self.sample_count = 1;
                Ok(())
            }
        }
    }

    /// Flush the buffered frame and its repetition count to the underlying writer.
    ///
    /// For MkvChain and TwoDelta variants, the repetition count is appended
    /// after the encoded frame. This is a no-op when no samples are pending.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` once the pending frame has been written.
    fn flush_pending_frame(&mut self) -> Result<()> {
        if self.sample_count == 0 {
            return Ok(());
        }

        let encoded = self
            .previous_encoded_sample
            .as_ref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing previous BEN frame"))?;
        self.writer.write_all(encoded.as_slice())?;

        if matches!(self.variant, BenVariant::MkvChain | BenVariant::TwoDelta) {
            self.writer.write_all(&self.sample_count.to_be_bytes())?;
        }

        Ok(())
    }

    /// Record additional repetitions of the most recently written assignment.
    ///
    /// For MkvChain and TwoDelta variants the repetition count is incremented
    /// directly. For Standard, the cached encoded frame is re-emitted once per
    /// additional repeat.
    ///
    /// # Arguments
    ///
    /// * `additional` - The number of extra copies beyond the one already written.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` after all additional repeats have been recorded.
    pub fn repeat_previous(&mut self, additional: u16) -> Result<()> {
        match self.variant {
            BenVariant::Standard => {
                if let Some(encoded) = self.previous_encoded_sample.as_ref() {
                    for _ in 0..additional {
                        self.writer.write_all(encoded.as_slice())?;
                    }
                }
            }
            BenVariant::MkvChain | BenVariant::TwoDelta => {
                self.sample_count += additional;
            }
        }
        Ok(())
    }

    /// Encode and write a full assignment vector.
    ///
    /// # Arguments
    ///
    /// * `assign_vec` - The full assignment vector to encode.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` after the assignment has been queued or written.
    pub fn write_assignment(&mut self, assign_vec: Vec<u16>) -> Result<()> {
        let hints = if self.variant == BenVariant::TwoDelta {
            let masks = if self.previous_masks.is_empty() {
                None
            } else {
                Some(&self.previous_masks)
            };
            analyze_twodelta_transition(&self.previous_sample, &assign_vec, masks)
        } else {
            AssignmentHints::default()
        };
        self.write_assignment_with_hints(assign_vec, hints)
    }

    /// Encode and write a JSON assignment record.
    ///
    /// The input must contain an `assignment` array of integers. Other fields
    /// are ignored.
    ///
    /// # Arguments
    ///
    /// * `data` - A JSON object containing an `assignment` array.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` after the record has been validated and encoded.
    pub fn write_json_value(&mut self, data: Value) -> Result<()> {
        let new_assign = parse_json_assignment(data)?;
        self.write_assignment(new_assign)
    }

    /// Flush any buffered repetition state to the underlying writer.
    ///
    /// This matters for [`BenVariant::MkvChain`], where repeated consecutive
    /// samples are emitted only once together with their repetition count.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` once any buffered repetition state has been flushed.
    pub fn finish(&mut self) -> Result<()> {
        if self.complete {
            return Ok(());
        }
        self.flush_pending_frame()?;
        self.complete = true;
        Ok(())
    }
}

impl<W: Write> Drop for BenEncoder<W> {
    /// Flush any buffered BEN state during drop.
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

/// A struct to make the writing of XBEN files easier and more ergonomic.
pub struct XBenEncoder<W: Write> {
    encoder: XzEncoder<W>,
    previous_assignment: Vec<u16>,
    previous_masks: HashMap<u16, Vec<usize>>,
    previous_frame: Vec<u8>,
    count: u16,
    variant: BenVariant,
    chunk_size: usize,
    chunk_buffer: Vec<BufferedDeltaFrame>,
}

impl<W: Write> XBenEncoder<W> {
    /// Rebuild the value-to-position index map from the current previous assignment.
    fn rebuild_previous_masks(&mut self) {
        self.previous_masks.clear();
        for (idx, &assignment) in self.previous_assignment.iter().enumerate() {
            self.previous_masks.entry(assignment).or_default().push(idx);
        }
    }

    /// Store a new previous assignment along with its encoded frame and repetition count.
    ///
    /// # Arguments
    ///
    /// * `assignment` - The assignment vector to cache.
    /// * `frame` - The already-encoded frame bytes for this assignment.
    /// * `count` - The initial repetition count for this assignment.
    fn set_previous_assignment(&mut self, assignment: Vec<u16>, frame: Vec<u8>, count: u16) {
        self.previous_assignment = assignment;
        self.rebuild_previous_masks();
        self.previous_frame = frame;
        self.count = count;
    }

    /// Update the value-to-position masks incrementally for a two-delta transition.
    ///
    /// Instead of rebuilding the entire mask HashMap, only the positions belonging
    /// to the two swapped values are repartitioned. This is O(pair_positions)
    /// rather than O(assignment_length).
    ///
    /// # Arguments
    ///
    /// * `new_sample` - The new assignment vector after the transition.
    /// * `pair` - The two values involved in the delta swap.
    #[allow(dead_code)]
    fn update_masks_for_delta(&mut self, new_sample: &[u16], pair: (u16, u16)) {
        if pair.0 == pair.1 {
            return;
        }

        let pos_a = self.previous_masks.remove(&pair.0).unwrap_or_default();
        let pos_b = self.previous_masks.remove(&pair.1).unwrap_or_default();

        let mut new_a = Vec::with_capacity(pos_a.len() + pos_b.len());
        let mut new_b = Vec::with_capacity(pos_a.len() + pos_b.len());

        let (mut i, mut j) = (0, 0);
        while i < pos_a.len() || j < pos_b.len() {
            let pos = if j >= pos_b.len() || (i < pos_a.len() && pos_a[i] < pos_b[j]) {
                let p = pos_a[i];
                i += 1;
                p
            } else {
                let p = pos_b[j];
                j += 1;
                p
            };
            if new_sample[pos] == pair.0 {
                new_a.push(pos);
            } else {
                new_b.push(pos);
            }
        }

        if !new_a.is_empty() {
            self.previous_masks.insert(pair.0, new_a);
        }
        if !new_b.is_empty() {
            self.previous_masks.insert(pair.1, new_b);
        }
    }

    /// Flush the buffered frame and its repetition count to the XZ encoder.
    ///
    /// For MkvChain and TwoDelta variants, the repetition count is appended
    /// after the encoded frame. This is a no-op when no samples are pending.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` once the pending frame has been written.
    fn flush_pending_frame(&mut self) -> Result<()> {
        if self.count == 0 {
            return Ok(());
        }

        self.encoder.write_all(&self.previous_frame)?;
        if matches!(self.variant, BenVariant::MkvChain | BenVariant::TwoDelta) {
            self.encoder.write_all(&self.count.to_be_bytes())?;
        }
        self.count = 0;
        Ok(())
    }

    /// Write all buffered delta frames as a single columnar chunk.
    ///
    /// The chunk layout groups same-type fields together so XZ's dictionary
    /// compression can exploit the resulting byte-level regularity:
    ///
    /// ```text
    /// [chunk_tag=2]  [n_frames: u32]
    /// [pairs channel:       (pair_a u16, pair_b u16) × n_frames]
    /// [counts channel:      count u16 × n_frames]
    /// [run-length counts:   n_runs u32 × n_frames]
    /// [run-length data:     u16 × total_runs]
    /// ```
    fn flush_chunk(&mut self) -> Result<()> {
        if self.chunk_buffer.is_empty() {
            return Ok(());
        }

        let n = self.chunk_buffer.len() as u32;
        self.encoder.write_all(&[XBEN_TWODELTA_CHUNK_TAG])?;
        self.encoder.write_all(&n.to_be_bytes())?;

        // Pairs channel.
        for frame in &self.chunk_buffer {
            self.encoder.write_all(&frame.pair.0.to_be_bytes())?;
            self.encoder.write_all(&frame.pair.1.to_be_bytes())?;
        }

        // Counts channel.
        for frame in &self.chunk_buffer {
            self.encoder.write_all(&frame.count.to_be_bytes())?;
        }

        // Run-length counts channel.
        for frame in &self.chunk_buffer {
            self.encoder
                .write_all(&(frame.run_lengths.len() as u32).to_be_bytes())?;
        }

        // Run-length data channel.
        for frame in &self.chunk_buffer {
            for &rl in &frame.run_lengths {
                self.encoder.write_all(&rl.to_be_bytes())?;
            }
        }

        self.chunk_buffer.clear();
        Ok(())
    }

    /// Create a new XBEN writer around an already-configured XZ encoder.
    ///
    /// # Arguments
    ///
    /// * `encoder` - The configured XZ encoder that will receive the ben32
    ///   payload.
    /// * `variant` - The BEN variant to encode inside the compressed stream.
    ///
    /// # Returns
    ///
    /// Returns a new XBEN encoder ready to accept assignments or BEN frames.
    pub fn new(mut encoder: XzEncoder<W>, variant: BenVariant) -> io::Result<Self> {
        encoder.write_all(banner_for_variant(variant))?;
        Ok(XBenEncoder {
            encoder,
            previous_assignment: Vec::new(),
            previous_masks: HashMap::new(),
            previous_frame: Vec::new(),
            count: 0,
            variant,
            chunk_size: DEFAULT_TWODELTA_CHUNK_SIZE,
            chunk_buffer: Vec::new(),
        })
    }

    /// Set the number of delta frames per columnar chunk.
    ///
    /// Only affects TwoDelta variant encoding. Larger chunks give XZ more
    /// same-type data to compress together; smaller chunks reduce peak memory.
    ///
    /// # Arguments
    ///
    /// * `size` - Number of delta frames per chunk.
    ///
    /// # Returns
    ///
    /// Returns `self` for method chaining.
    pub fn with_chunk_size(mut self, size: usize) -> Self {
        self.chunk_size = size.max(1);
        self
    }

    /// Encode and write a full assignment vector into the compressed XBEN stream.
    ///
    /// # Arguments
    ///
    /// * `assign_vec` - The full assignment vector to encode.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` after the assignment has been queued or written.
    pub fn write_assignment(&mut self, assign_vec: Vec<u16>) -> Result<()> {
        match self.variant {
            BenVariant::Standard => {
                let encoded = encode_ben32_assignments(&assign_vec)?;
                self.encoder.write_all(&encoded)?;
                self.previous_assignment = assign_vec;
                self.previous_frame = encoded;
                Ok(())
            }
            BenVariant::MkvChain => {
                if is_repeated_assignment(&self.previous_assignment, &assign_vec) {
                    self.count += 1;
                    return Ok(());
                }

                self.flush_pending_frame()?;
                let encoded = encode_ben32_assignments(&assign_vec)?;
                self.set_previous_assignment(assign_vec, encoded, 1);
                Ok(())
            }
            BenVariant::TwoDelta => {
                if self.previous_assignment.is_empty() {
                    let encoded = encode_xben_twodelta_full_frame(&assign_vec);
                    self.set_previous_assignment(assign_vec, encoded, 1);
                    return Ok(());
                }

                let masks = if self.previous_masks.is_empty() {
                    None
                } else {
                    Some(&self.previous_masks)
                };
                let hints =
                    analyze_twodelta_transition(&self.previous_assignment, &assign_vec, masks);
                if hints.is_repeated {
                    if self.chunk_buffer.is_empty() {
                        self.count += 1;
                    } else {
                        self.chunk_buffer.last_mut().unwrap().count += 1;
                    }
                    return Ok(());
                }

                // Flush the initial full frame before the first delta.
                if self.chunk_buffer.is_empty() {
                    self.flush_pending_frame()?;
                }

                let encoded_frame: TwoDeltaFrame = match encode_twodelta_frame_with_hint(
                    &self.previous_assignment,
                    &assign_vec,
                    hints.delta_pair,
                    Some(&mut self.previous_masks),
                ) {
                    Ok(frame) => frame,
                    Err(e) => {
                        return Err(e);
                    }
                };

                self.chunk_buffer.push(BufferedDeltaFrame {
                    pair: encoded_frame.pair,
                    run_lengths: encoded_frame.run_length_vector,
                    count: 1,
                });

                if self.chunk_buffer.len() >= self.chunk_size {
                    self.flush_chunk()?;
                }
                Ok(())
            }
        }
    }

    /// Encode and write a JSON assignment record into the compressed XBEN stream.
    ///
    /// # Arguments
    ///
    /// * `data` - A JSON object containing an `assignment` array.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` after the record has been validated and encoded.
    pub fn write_json_value(&mut self, data: Value) -> Result<()> {
        self.write_assignment(parse_json_assignment(data)?)
    }

    /// Read BEN frames from `reader` and write them into this XBEN stream.
    ///
    /// If the source still contains the 17-byte BEN banner, it is consumed and
    /// replaced by the banner already written by this encoder.
    ///
    /// # Arguments
    ///
    /// * `reader` - The BEN input stream, with or without its banner.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` after the BEN stream has been translated into XBEN.
    /// Translate a BEN TwoDelta stream directly to XBEN TwoDelta without
    /// materializing full assignment vectors.
    ///
    /// The first frame (standard BEN RLE) is decoded to RLE runs and written as
    /// an XBEN full frame. Subsequent delta frames have their bitpacked run
    /// lengths unpacked and written as XBEN delta frames with raw u16 runs.
    /// This avoids O(N) assignment reconstruction per frame entirely.
    ///
    /// # Arguments
    ///
    /// * `reader` - The BEN TwoDelta stream positioned after the banner.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` after the stream has been fully translated.
    fn translate_ben_twodelta_to_xben(&mut self, mut reader: impl Read) -> Result<()> {
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
        self.previous_frame = encoded;
        self.count = first_count;

        let mut sample_count = first_count as usize;
        progress!("Encoding line: {}\r", sample_count);

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

            // Unpack bitpacked run lengths.
            let frame = TwoDeltaFrame::from_parts((pair_a, pair_b), delta_max_len_bits, payload);
            let run_lengths = frame.run_length_vector;

            // Flush the initial full frame before the first delta chunk.
            if self.chunk_buffer.is_empty() && self.count > 0 {
                self.flush_pending_frame()?;
            }

            self.chunk_buffer.push(BufferedDeltaFrame {
                pair: frame.pair,
                run_lengths,
                count,
            });

            if self.chunk_buffer.len() >= self.chunk_size {
                self.flush_chunk()?;
            }

            sample_count += count as usize;
            progress!("Encoding line: {}\r", sample_count);
        }

        // Flush remaining partial chunk (Drop will also catch this, but be explicit).
        self.flush_chunk()?;

        tracing::trace!("");
        tracing::trace!("Done!");
        Ok(())
    }

    pub fn write_ben_file(&mut self, mut reader: impl BufRead) -> Result<()> {
        let peek = reader.fill_buf()?;
        let has_banner = peek.len() >= BANNER_LEN && has_known_banner_prefix(peek);

        if has_banner {
            if self.variant == BenVariant::TwoDelta {
                reader.consume(BANNER_LEN);
                return self.translate_ben_twodelta_to_xben(reader);
            }
            reader.consume(BANNER_LEN);
        }

        if self.variant == BenVariant::TwoDelta {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "TwoDelta XBEN translation requires a BEN stream with its banner",
            ));
        }

        ben_to_ben32_lines(&mut reader, &mut self.encoder, self.variant)
    }
}

impl<W: Write> Drop for XBenEncoder<W> {
    /// Flush any buffered XBEN repetition state during drop.
    fn drop(&mut self) {
        if matches!(self.variant, BenVariant::MkvChain | BenVariant::TwoDelta) && self.count > 0 {
            let _ = self.flush_pending_frame();
        }
        if !self.chunk_buffer.is_empty() {
            let _ = self.flush_chunk();
        }
    }
}
