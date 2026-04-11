use super::errors::DecoderInitError;
use super::subsample::{Ben32Frame, DecodeFrame, MkvRecord, SubsampleFrameDecoder};
use super::twodelta::{
    XBenTwoDeltaFrame, XBEN_TWODELTA_CHUNK_TAG, XBEN_TWODELTA_DELTA_TAG, XBEN_TWODELTA_FULL_TAG,
};
use crate::codec::decode::{apply_twodelta_runs_to_assignment, decode_ben32_line, DecodeError};
use crate::codec::encode::encode_ben32_assignments;
use crate::format::banners::{variant_from_banner, BANNER_LEN};
use crate::util::rle::rle_to_vec;
use crate::{progress, BenVariant};
use serde_json::json;
use std::io::{self, BufReader, Cursor, Read, Write};
use xz2::read::XzDecoder;

/// Iterator over decoded assignments in an XBEN stream.
pub struct XZAssignmentReader<R: Read> {
    xz: BufReader<XzDecoder<R>>,
    /// Variant encoded in the XBEN banner (private; use `.variant()` accessor).
    inner_variant: BenVariant,
    overflow: Vec<u8>,
    buf: Box<[u8]>,
    previous_assignment: Option<Vec<u16>>,
    chunk_queue: std::collections::VecDeque<(XBenTwoDeltaFrame, u16)>,
    silent: bool,
}

impl<R: Read> XZAssignmentReader<R> {
    /// Create an XBEN decoder from an already-opened decompressed stream.
    ///
    /// # Arguments
    ///
    /// * `xz` - A buffered XZ decompression reader positioned past the banner.
    /// * `variant` - The BEN variant indicated by the banner.
    ///
    /// # Returns
    ///
    /// Returns a new decoder ready to yield frames from the stream.
    pub(crate) fn from_decompressed_stream(
        xz: BufReader<XzDecoder<R>>,
        variant: BenVariant,
    ) -> Self {
        Self {
            xz,
            inner_variant: variant,
            overflow: Vec::with_capacity(1 << 20),
            buf: vec![0u8; 1 << 20].into_boxed_slice(),
            previous_assignment: None,
            chunk_queue: std::collections::VecDeque::new(),
            silent: false,
        }
    }

    /// Create a decoder for an XBEN stream.
    ///
    /// # Arguments
    ///
    /// * `reader` - The compressed XBEN input stream.
    ///
    /// # Returns
    ///
    /// Returns a new decoder positioned at the first ben32 frame in the
    /// decompressed payload.
    pub fn new(reader: R) -> Result<Self, DecoderInitError> {
        let xz = XzDecoder::new(reader);
        let mut xz = BufReader::with_capacity(1 << 20, xz);

        let mut first = [0u8; BANNER_LEN];
        if let Err(e) = xz.read_exact(&mut first) {
            return Err(DecoderInitError::Io(e));
        }
        let variant = match variant_from_banner(&first) {
            Some(v) => v,
            None => return Err(DecoderInitError::InvalidFileFormat(first.to_vec())),
        };

        Ok(Self::from_decompressed_stream(xz, variant))
    }

    /// Return the BEN variant detected from the stream banner.
    pub fn variant(&self) -> BenVariant {
        self.inner_variant
    }

    /// Suppress progress output from this decoder's iterator.
    ///
    /// # Arguments
    ///
    /// * `silent` - When `true`, the decoder will not emit progress messages.
    ///
    /// # Returns
    ///
    /// Returns `self` for method chaining.
    pub fn silent(mut self, silent: bool) -> Self {
        self.silent = silent;
        self
    }

    /// Try to extract one complete ben32 frame from the buffered overflow.
    ///
    /// Scans `overflow` for a four-byte zero sentinel that terminates a ben32
    /// frame and, for MkvChain streams, reads the trailing repetition count.
    ///
    /// # Arguments
    ///
    /// * `overflow` - Buffered decompressed bytes that may contain one or more
    ///   complete ben32 frames.
    ///
    /// # Returns
    ///
    /// Returns the frame bytes, the number of consumed bytes, and the decoded
    /// repetition count when a complete frame is available.
    fn pop_frame_from_overflow<'a>(&self, overflow: &'a [u8]) -> Option<(&'a [u8], usize, u16)> {
        match self.inner_variant {
            BenVariant::Standard => {
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
            }
            BenVariant::MkvChain => {
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
            BenVariant::TwoDelta => None,
        }
    }

    /// Try to extract one complete TwoDelta frame from the buffered overflow.
    ///
    /// Inspects the leading tag byte to determine whether the frame is a full
    /// RLE frame or a delta frame, then reads the corresponding payload.
    ///
    /// # Arguments
    ///
    /// * `overflow` - Buffered decompressed bytes that may contain a complete
    ///   TwoDelta frame.
    ///
    /// # Returns
    ///
    /// Returns the parsed frame, the number of consumed bytes, and the
    /// repetition count when a complete frame is available.
    fn pop_twodelta_frame_from_overflow(
        &self,
        overflow: &[u8],
    ) -> Option<io::Result<(XBenTwoDeltaFrame, usize, u16)>> {
        let tag = *overflow.first()?;
        match tag {
            XBEN_TWODELTA_FULL_TAG => {
                if overflow.len() < 7 {
                    return None;
                }
                let run_count =
                    u32::from_be_bytes([overflow[1], overflow[2], overflow[3], overflow[4]])
                        as usize;
                let payload_len = run_count.checked_mul(4)?;
                let total_len = 1usize
                    .checked_add(4)?
                    .checked_add(payload_len)?
                    .checked_add(2)?;
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
                Some(Ok((XBenTwoDeltaFrame::Full { runs }, total_len, count)))
            }
            XBEN_TWODELTA_DELTA_TAG => {
                if overflow.len() < 11 {
                    return None;
                }
                let pair = (
                    u16::from_be_bytes([overflow[1], overflow[2]]),
                    u16::from_be_bytes([overflow[3], overflow[4]]),
                );
                let run_count =
                    u32::from_be_bytes([overflow[5], overflow[6], overflow[7], overflow[8]])
                        as usize;
                let payload_len = run_count.checked_mul(2)?;
                let total_len = 1usize
                    .checked_add(2)?
                    .checked_add(2)?
                    .checked_add(4)?
                    .checked_add(payload_len)?
                    .checked_add(2)?;
                if overflow.len() < total_len {
                    return None;
                }

                let mut run_lengths = Vec::with_capacity(run_count);
                let mut cursor = 9usize;
                for _ in 0..run_count {
                    run_lengths.push(u16::from_be_bytes([overflow[cursor], overflow[cursor + 1]]));
                    cursor += 2;
                }
                let count = u16::from_be_bytes([overflow[cursor], overflow[cursor + 1]]);
                Some(Ok((
                    XBenTwoDeltaFrame::Delta { pair, run_lengths },
                    total_len,
                    count,
                )))
            }
            XBEN_TWODELTA_CHUNK_TAG => None, // Handled by try_parse_twodelta_chunk.
            _ => Some(Err(io::Error::from(DecodeError::XBenUnknownFrameTag {
                tag,
            }))),
        }
    }

    /// Try to parse a columnar TwoDelta chunk from the overflow buffer.
    ///
    /// If the overflow starts with the chunk tag and contains enough bytes for
    /// the full chunk, all frames are decoded and pushed onto `chunk_queue`.
    /// Returns `Some(Ok(()))` on success, `Some(Err(...))` on a parse error,
    /// or `None` when the overflow is incomplete.
    fn try_parse_twodelta_chunk(&mut self) -> Option<io::Result<()>> {
        if self.overflow.first() != Some(&XBEN_TWODELTA_CHUNK_TAG) {
            return None;
        }
        if self.overflow.len() < 5 {
            return None;
        }

        let n_frames = u32::from_be_bytes([
            self.overflow[1],
            self.overflow[2],
            self.overflow[3],
            self.overflow[4],
        ]) as usize;

        // Calculate total chunk size: tag(1) + n_frames(4)
        //   + pairs(n*4) + counts(n*2) + run_counts(n*4) + run_data(variable)
        let header_len: usize = 5;
        let pairs_len = match n_frames.checked_mul(4) {
            Some(v) => v,
            None => return Some(Err(io::Error::from(DecodeError::XBenTruncated))),
        };
        let counts_len = match n_frames.checked_mul(2) {
            Some(v) => v,
            None => return Some(Err(io::Error::from(DecodeError::XBenTruncated))),
        };
        let run_counts_len = match n_frames.checked_mul(4) {
            Some(v) => v,
            None => return Some(Err(io::Error::from(DecodeError::XBenTruncated))),
        };
        let fixed_len = match header_len
            .checked_add(pairs_len)
            .and_then(|v| v.checked_add(counts_len))
            .and_then(|v| v.checked_add(run_counts_len))
        {
            Some(v) => v,
            None => return Some(Err(io::Error::from(DecodeError::XBenTruncated))),
        };

        if self.overflow.len() < fixed_len {
            return None;
        }

        // Read run-length counts to determine total run data size.
        let run_counts_start = header_len + pairs_len + counts_len;
        let mut total_runs = 0usize;
        let mut run_counts = Vec::with_capacity(n_frames);
        for i in 0..n_frames {
            let offset = run_counts_start + i * 4;
            let rc = u32::from_be_bytes([
                self.overflow[offset],
                self.overflow[offset + 1],
                self.overflow[offset + 2],
                self.overflow[offset + 3],
            ]) as usize;
            run_counts.push(rc);
            total_runs = match total_runs.checked_add(rc) {
                Some(v) => v,
                None => return Some(Err(io::Error::from(DecodeError::XBenTruncated))),
            };
        }

        let run_data_len = match total_runs.checked_mul(2) {
            Some(v) => v,
            None => return Some(Err(io::Error::from(DecodeError::XBenTruncated))),
        };
        let total_len = match fixed_len.checked_add(run_data_len) {
            Some(v) => v,
            None => return Some(Err(io::Error::from(DecodeError::XBenTruncated))),
        };
        if self.overflow.len() < total_len {
            return None;
        }

        // Parse pairs channel.
        let pairs_start = header_len;
        // Parse counts channel.
        let counts_start = pairs_start + pairs_len;
        // Run data starts after run counts.
        let run_data_start = run_counts_start + run_counts_len;

        let mut run_cursor = run_data_start;
        for i in 0..n_frames {
            let po = pairs_start + i * 4;
            let pair = (
                u16::from_be_bytes([self.overflow[po], self.overflow[po + 1]]),
                u16::from_be_bytes([self.overflow[po + 2], self.overflow[po + 3]]),
            );
            let co = counts_start + i * 2;
            let count = u16::from_be_bytes([self.overflow[co], self.overflow[co + 1]]);

            let rc = run_counts[i];
            let mut run_lengths = Vec::with_capacity(rc);
            for _ in 0..rc {
                run_lengths.push(u16::from_be_bytes([
                    self.overflow[run_cursor],
                    self.overflow[run_cursor + 1],
                ]));
                run_cursor += 2;
            }

            self.chunk_queue
                .push_back((XBenTwoDeltaFrame::Delta { pair, run_lengths }, count));
        }

        self.overflow.drain(..total_len);
        Some(Ok(()))
    }

    /// Consume this decoder and iterate over raw ben32 frames instead of
    /// materialized assignments.
    ///
    /// # Returns
    ///
    /// Returns an iterator that yields raw ben32 frames from the remaining
    /// input.
    pub fn into_frames(self) -> XZAssignmentFrameReader<R> {
        XZAssignmentFrameReader { inner: self }
    }

    /// Count the number of samples remaining in the XBEN stream.
    ///
    /// # Returns
    ///
    /// Returns the number of remaining samples in the stream.
    pub fn count_samples(self) -> io::Result<usize> {
        let mut total = 0usize;
        for frame_res in self.into_frames() {
            let (_bytes, cnt) = frame_res?;
            total += cnt as usize;
        }
        Ok(total)
    }

    /// Decode assignments and pass each one to a callback by reference.
    ///
    /// The callback receives a borrowed assignment slice and its repetition
    /// count. Return `true` to continue decoding or `false` to stop early.
    ///
    /// # Arguments
    ///
    /// * `f` - A callback invoked once per unique frame with `(&[u16], u16)`.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` after the stream is exhausted or the callback signals stop.
    pub fn for_each_assignment<F>(&mut self, mut f: F) -> io::Result<()>
    where
        F: FnMut(&[u16], u16) -> io::Result<bool>,
    {
        let mut sample_count = 0usize;
        loop {
            match self.next() {
                Some(Ok((assignment, count))) => {
                    sample_count += count as usize;
                    if !self.silent {
                        progress!("Decoding sample: {}\r", sample_count);
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

    /// Decode the remaining XBEN stream and write it as JSONL.
    ///
    /// # Arguments
    ///
    /// * `writer` - The destination that will receive one JSON object per
    ///   decoded sample.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` after the remaining stream has been fully decoded.
    pub fn write_all_jsonl(&mut self, mut writer: impl Write) -> io::Result<()> {
        let mut sample_number = 0usize;
        self.for_each_assignment(|assignment, count| {
            for _ in 0..count {
                sample_number += 1;
                let line = json!({
                    "assignment": assignment,
                    "sample": sample_number,
                })
                .to_string()
                    + "\n";
                writer.write_all(line.as_bytes())?;
            }
            Ok(true)
        })
    }
}

fn zero_count_frame_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "XBEN frame count must be greater than zero",
    )
}

/// Decode one raw ben32 frame from an XBEN stream into a full assignment vector.
///
/// # Arguments
///
/// * `frame_bytes` - The ben32 frame bytes.
/// * `variant` - The BEN variant used to interpret the frame tail.
///
/// # Returns
///
/// Returns the expanded assignment vector.
pub(super) fn decode_xben_frame_to_assignment(
    frame_bytes: &[u8],
    variant: BenVariant,
) -> io::Result<Vec<u16>> {
    let cursor = Cursor::new(frame_bytes);
    let (assignment, _) = decode_ben32_line(cursor, variant)?;
    Ok(assignment)
}

impl<R: Read> Iterator for XZAssignmentReader<R> {
    type Item = io::Result<MkvRecord>;

    /// Decode and return the next assignment from the XBEN stream.
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.inner_variant {
                BenVariant::Standard | BenVariant::MkvChain => {
                    if let Some((frame_bytes, consumed, count)) =
                        self.pop_frame_from_overflow(&self.overflow)
                    {
                        if count == 0 {
                            self.overflow.drain(..consumed);
                            return Some(Err(zero_count_frame_error()));
                        }
                        let res = match decode_xben_frame_to_assignment(
                            frame_bytes,
                            self.inner_variant,
                        ) {
                            Ok(assignment) => {
                                self.previous_assignment = Some(assignment.clone());
                                Ok((assignment, count))
                            }
                            Err(e) => Err(e),
                        };
                        self.overflow.drain(..consumed);
                        return Some(res);
                    }
                }
                BenVariant::TwoDelta => {
                    // Drain frames from a previously parsed chunk first.
                    if let Some((frame, count)) = self.chunk_queue.pop_front() {
                        if count == 0 {
                            return Some(Err(zero_count_frame_error()));
                        }
                        let assignment = match frame {
                            XBenTwoDeltaFrame::Full { runs } => Ok(rle_to_vec(runs)),
                            XBenTwoDeltaFrame::Delta { pair, run_lengths } => {
                                match self.previous_assignment.take() {
                                    Some(prev) => {
                                        apply_twodelta_runs_to_assignment(prev, pair, &run_lengths)
                                    }
                                    None => {
                                        Err(io::Error::from(DecodeError::TwoDeltaNoAnchorFrame))
                                    }
                                }
                            }
                        };
                        return Some(match assignment {
                            Ok(a) => {
                                self.previous_assignment = Some(a.clone());
                                Ok((a, count))
                            }
                            Err(e) => Err(e),
                        });
                    }

                    // Try to parse a columnar chunk.
                    if let Some(result) = self.try_parse_twodelta_chunk() {
                        match result {
                            Ok(()) => continue, // Loop to drain chunk_queue.
                            Err(e) => return Some(Err(e)),
                        }
                    }

                    // Try a single legacy frame (tag 0 or 1).
                    if let Some(parsed) = self.pop_twodelta_frame_from_overflow(&self.overflow) {
                        let res = match parsed {
                            Ok((frame, consumed, count)) => {
                                if count == 0 {
                                    self.overflow.drain(..consumed);
                                    return Some(Err(zero_count_frame_error()));
                                }
                                let assignment = match frame {
                                    XBenTwoDeltaFrame::Full { runs } => Ok(rle_to_vec(runs)),
                                    XBenTwoDeltaFrame::Delta { pair, run_lengths } => {
                                        match self.previous_assignment.take() {
                                            Some(previous_assignment) => {
                                                apply_twodelta_runs_to_assignment(
                                                    previous_assignment,
                                                    pair,
                                                    &run_lengths,
                                                )
                                            }
                                            None => Err(io::Error::from(
                                                DecodeError::TwoDeltaNoAnchorFrame,
                                            )),
                                        }
                                    }
                                };
                                match assignment {
                                    Ok(assignment) => {
                                        self.previous_assignment = Some(assignment.clone());
                                        self.overflow.drain(..consumed);
                                        Ok((assignment, count))
                                    }
                                    Err(err) => {
                                        self.overflow.drain(..consumed);
                                        Err(err)
                                    }
                                }
                            }
                            Err(err) => {
                                self.overflow.clear();
                                Err(err)
                            }
                        };
                        return Some(res);
                    }
                }
            }

            let read = match self.xz.read(&mut self.buf) {
                Ok(0) => {
                    if self.overflow.is_empty() {
                        return None;
                    } else {
                        return Some(Err(io::Error::from(DecodeError::XBenTruncated)));
                    }
                }
                Ok(n) => n,
                Err(e) => return Some(Err(e)),
            };
            self.overflow.extend_from_slice(&self.buf[..read]);
        }
    }
}

/// Iterator over raw ben32 frames inside an XBEN stream.
pub struct XZAssignmentFrameReader<R: Read> {
    pub(super) inner: XZAssignmentReader<R>,
}

impl<R: Read> XZAssignmentFrameReader<R> {
    /// Create a raw XBEN frame iterator from a reader.
    ///
    /// # Arguments
    ///
    /// * `reader` - The compressed XBEN input stream.
    ///
    /// # Returns
    ///
    /// Returns an iterator over raw ben32 frames.
    pub fn new(reader: R) -> Result<Self, DecoderInitError> {
        Ok(Self {
            inner: XZAssignmentReader::new(reader)?,
        })
    }
}

impl<R: Read> Iterator for XZAssignmentFrameReader<R> {
    type Item = io::Result<Ben32Frame>;

    /// Return the next raw ben32 frame from the input stream.
    fn next(&mut self) -> Option<Self::Item> {
        if self.inner.inner_variant == BenVariant::TwoDelta {
            return self.inner.next().map(|result| {
                result.and_then(|(assignment, count)| {
                    Ok((encode_ben32_assignments(&assignment)?, count))
                })
            });
        }

        loop {
            if let Some((frame, consumed, count)) =
                self.inner.pop_frame_from_overflow(&self.inner.overflow)
            {
                if count == 0 {
                    self.inner.overflow.drain(..consumed);
                    return Some(Err(zero_count_frame_error()));
                }
                let out = frame.to_vec();
                self.inner.overflow.drain(..consumed);
                return Some(Ok((out, count)));
            }

            let read = match self.inner.xz.read(&mut self.inner.buf) {
                Ok(0) => {
                    if self.inner.overflow.is_empty() {
                        return None;
                    } else {
                        return Some(Err(io::Error::from(DecodeError::XBenTruncated)));
                    }
                }
                Ok(n) => n,
                Err(e) => return Some(Err(e)),
            };
            self.inner
                .overflow
                .extend_from_slice(&self.inner.buf[..read]);
        }
    }
}

impl<R: Read + Send> XZAssignmentReader<R> {
    /// Convert this decoder into a subsampling iterator over explicit 1-based
    /// indices.
    ///
    /// # Arguments
    ///
    /// * `indices` - A collection of 1-based sample indices.
    ///
    /// # Returns
    ///
    /// Returns a decoder that yields only the selected samples.
    pub fn into_subsample_by_indices<T>(
        self,
        indices: T,
    ) -> SubsampleFrameDecoder<impl Iterator<Item = io::Result<(DecodeFrame, u16)>> + Send>
    where
        T: IntoIterator<Item = usize>,
    {
        let variant = self.inner_variant;
        let frames = self
            .into_frames()
            .map(move |res| res.map(|(bytes, cnt)| (DecodeFrame::XBen(bytes, variant), cnt)));
        SubsampleFrameDecoder::by_indices(Box::new(frames), indices)
    }

    /// Convert this decoder into a subsampling iterator over the inclusive
    /// 1-based range `[start, end]`.
    ///
    /// # Arguments
    ///
    /// * `start` - The first 1-based sample index to include.
    /// * `end` - The last 1-based sample index to include.
    ///
    /// # Returns
    ///
    /// Returns a decoder that yields only the selected samples.
    pub fn into_subsample_by_range(
        self,
        start: usize,
        end: usize,
    ) -> SubsampleFrameDecoder<impl Iterator<Item = io::Result<(DecodeFrame, u16)>> + Send> {
        let variant = self.inner_variant;
        let frames = self
            .into_frames()
            .map(move |res| res.map(|(bytes, cnt)| (DecodeFrame::XBen(bytes, variant), cnt)));
        SubsampleFrameDecoder::by_range(Box::new(frames), start, end)
    }

    /// Convert this decoder into a subsampling iterator that selects every
    /// `step` samples from the 1-based `offset`.
    ///
    /// # Arguments
    ///
    /// * `step` - The stride between selected samples.
    /// * `offset` - The 1-based index of the first selected sample.
    ///
    /// # Returns
    ///
    /// Returns a decoder that yields only the selected samples.
    pub fn into_subsample_every(
        self,
        step: usize,
        offset: usize,
    ) -> SubsampleFrameDecoder<impl Iterator<Item = io::Result<(DecodeFrame, u16)>> + Send> {
        let variant = self.inner_variant;
        let frames = self
            .into_frames()
            .map(move |res| res.map(|(bytes, cnt)| (DecodeFrame::XBen(bytes, variant), cnt)));
        SubsampleFrameDecoder::every(Box::new(frames), step, offset)
    }
}
