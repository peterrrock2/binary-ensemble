use super::frames::BufferedDeltaFrame;
use super::twodelta::{
    DEFAULT_TWODELTA_CHUNK_SIZE, XBEN_TWODELTA_CHUNK_TAG, XBEN_TWODELTA_FULL_TAG,
};
use super::utils::{encode_xben_twodelta_full_frame, parse_json_assignment};
use crate::codec::decode::decode_ben_line;
use crate::codec::encode::{encode_ben32_assignments, encode_twodelta_frame_with_hint};
use crate::codec::translate::ben_to_ben32_lines;
use crate::codec::TwoDeltaEncodeFrame;
use crate::format::banners::{banner_for_variant, has_known_banner_prefix, BANNER_LEN};
use crate::{progress, BenVariant};
use byteorder::{BigEndian, ReadBytesExt};
use serde_json::Value;
use std::collections::HashMap;
use std::io::{self, BufRead, Read, Result, Write};
use xz2::write::XzEncoder;

/// A struct to make the writing of XBEN files easier and more ergonomic.
pub struct XZAssignmentWriter<W: Write> {
    encoder: XzEncoder<W>,
    previous_assignment: Vec<u16>,
    previous_masks: HashMap<u16, Vec<usize>>,
    pending_assignment: Option<Vec<u16>>,
    count: u16,
    variant: BenVariant,
    chunk_size: usize,
    chunk_buffer: Vec<BufferedDeltaFrame>,
    complete: bool,
}

impl<W: Write> XZAssignmentWriter<W> {
    /// Encode and write the pending assignment with the accumulated count.
    ///
    /// For TwoDelta, builds the initial masks and writes the full frame followed
    /// by the count. For MkvChain, encodes the assignment and appends the count.
    /// This is a no-op when no assignment is pending.
    fn flush_pending_frame(&mut self) -> Result<()> {
        let pending = match self.pending_assignment.take() {
            Some(p) => p,
            None => return Ok(()),
        };

        // Standard writes each assignment immediately; MkvChain and TwoDelta buffer.
        if self.variant == BenVariant::MkvChain {
            let encoded = encode_ben32_assignments(&pending)?;
            self.encoder.write_all(&encoded)?;
            self.encoder.write_all(&self.count.to_be_bytes())?;
        } else {
            // TwoDelta
            for (idx, &val) in pending.iter().enumerate() {
                self.previous_masks.entry(val).or_default().push(idx);
            }
            let encoded = encode_xben_twodelta_full_frame(&pending);
            self.encoder.write_all(&encoded)?;
            self.encoder.write_all(&self.count.to_be_bytes())?;
        }

        self.previous_assignment = pending;
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
        Ok(XZAssignmentWriter {
            encoder,
            previous_assignment: Vec::new(),
            previous_masks: HashMap::new(),
            pending_assignment: None,
            count: 0,
            variant,
            chunk_size: DEFAULT_TWODELTA_CHUNK_SIZE,
            chunk_buffer: Vec::new(),
            complete: false,
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
            }
            BenVariant::MkvChain => {
                if self.pending_assignment.as_deref() == Some(assign_vec.as_slice()) {
                    if self.count == u16::MAX {
                        self.flush_pending_frame()?;
                        self.pending_assignment = Some(assign_vec);
                        self.count = 1;
                        return Ok(());
                    }
                    self.count += 1;
                    return Ok(());
                }
                self.flush_pending_frame()?;
                self.pending_assignment = Some(assign_vec);
                self.count = 1;
            }
            BenVariant::TwoDelta => {
                // First assignment ever: buffer as the initial full frame.
                if self.pending_assignment.is_none() && self.previous_assignment.is_empty() {
                    self.pending_assignment = Some(assign_vec);
                    self.count = 1;
                    return Ok(());
                }
                // Repeat of the pending initial full frame.
                if self.pending_assignment.as_deref() == Some(assign_vec.as_slice()) {
                    if self.count == u16::MAX {
                        self.flush_pending_frame()?;
                        let repeat = twodelta_repeat_buffered_frame(&assign_vec, 1)?;
                        self.chunk_buffer.push(repeat);
                        self.previous_assignment = assign_vec;
                        return Ok(());
                    }
                    self.count += 1;
                    return Ok(());
                }
                // Repeat of the last delta frame in the current chunk.
                if !self.chunk_buffer.is_empty()
                    && self.previous_assignment.as_slice() == assign_vec.as_slice()
                {
                    if self.chunk_buffer.last().unwrap().count == u16::MAX {
                        self.flush_chunk()?;
                        let repeat = twodelta_repeat_buffered_frame(&assign_vec, 1)?;
                        self.chunk_buffer.push(repeat);
                    } else {
                        self.chunk_buffer.last_mut().unwrap().count += 1;
                    }
                    return Ok(());
                }
                // New distinct assignment: flush the initial full frame if pending.
                if self.pending_assignment.is_some() {
                    self.flush_pending_frame()?;
                }
                // Encode the delta frame and add it to the chunk buffer.
                let frame = encode_twodelta_frame_with_hint(
                    &self.previous_assignment,
                    &assign_vec,
                    None,
                    Some(&mut self.previous_masks),
                    None,
                )?;
                self.chunk_buffer.push(BufferedDeltaFrame {
                    pair: frame.pair,
                    run_lengths: frame.run_length_vector,
                    count: 1,
                });
                self.previous_assignment = assign_vec;
                if self.chunk_buffer.len() >= self.chunk_size {
                    self.flush_chunk()?;
                }
            }
        }
        Ok(())
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

    /// Flush any buffered state to the underlying XZ encoder.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` once all buffered state has been flushed.
    pub fn finish(&mut self) -> Result<()> {
        if self.complete {
            return Ok(());
        }
        self.flush_pending_frame()?;
        self.flush_chunk()?;
        self.complete = true;
        Ok(())
    }

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
        self.encoder.write_all(&encoded)?;
        self.encoder.write_all(&first_count.to_be_bytes())?;

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
            let frame =
                TwoDeltaEncodeFrame::from_parts((pair_a, pair_b), delta_max_len_bits, payload);
            let run_lengths = frame.run_length_vector;

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

fn twodelta_repeat_buffered_frame(
    assignment: &[u16],
    count: u16,
) -> io::Result<BufferedDeltaFrame> {
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

    Ok(BufferedDeltaFrame {
        pair: (first, second),
        run_lengths,
        count,
    })
}

impl<W: Write> Drop for XZAssignmentWriter<W> {
    /// Flush any buffered XBEN state during drop.
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Read};
    use xz2::write::XzEncoder;

    #[test]
    fn twodelta_repeat_buffered_frame_run_exceeds_u16_max_errors() {
        let assign = vec![1u16; 65536];
        let result = twodelta_repeat_buffered_frame(&assign, 1);
        let err = result.err().expect("expected error");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("u16::MAX"));
    }

    #[test]
    fn translate_twodelta_non_eof_read_error_propagates() {
        // write_ben_file in TwoDelta mode calls translate_ben_twodelta_to_xben.
        // After reading the anchor frame it loops reading delta frames; a
        // non-EOF error on pair_a (first u16 read in the loop) must propagate.
        let mut xben = Vec::new();
        let encoder = XzEncoder::new(&mut xben, 1);
        let mut writer = XZAssignmentWriter::new(encoder, BenVariant::TwoDelta).unwrap();

        // Banner (17 bytes) + minimal anchor frame:
        //   max_val_bits=1, max_len_bits=1, n_bytes=0 (no payload), count=1
        let mut input: Vec<u8> = b"TWODELTA BEN FILE".to_vec();
        input.extend_from_slice(&[0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01]);

        // Append an error source after the anchor frame bytes.
        struct ErrorAfterEof;
        impl Read for ErrorAfterEof {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "broken"))
            }
        }

        let reader = std::io::BufReader::new(input.as_slice().chain(ErrorAfterEof));
        let err = writer.write_ben_file(reader).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    }
}
