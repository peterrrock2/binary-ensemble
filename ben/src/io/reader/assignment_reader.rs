use super::errors::DecoderInitError;
use crate::codec::{BenDecodeFrame, BenEncodeFrame};
use crate::format::banners::{variant_from_banner, BANNER_LEN};
use crate::progress::Spinner;
use crate::BenVariant;
use serde_json::json;
use std::io::{self, Read, Write};

/// Iterator over decoded assignments in an uncompressed BEN stream.
pub struct AssignmentReader<R: Read> {
    reader: R,
    sample_count: usize,
    variant: BenVariant,
    previous_assignment: Option<Vec<u16>>,
    twodelta_consumed_first_frame: bool,
    silent: bool,
    spinner: Option<Spinner>,
}

fn zero_count_frame_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "BEN frame count must be greater than zero",
    )
}

impl<R: Read> AssignmentReader<R> {
    /// Create a decoder for an uncompressed BEN stream.
    ///
    /// The reader must begin with one of the BEN banners such as
    /// `STANDARD BEN FILE` or `MKVCHAIN BEN FILE`.
    ///
    /// # Arguments
    ///
    /// * `reader` - The input BEN stream, including its 17-byte banner.
    ///
    /// # Returns
    ///
    /// Returns a new decoder positioned at the first BEN frame.
    pub fn new(mut reader: R) -> Result<Self, DecoderInitError> {
        let mut check_buffer = [0u8; BANNER_LEN];

        if let Err(e) = reader.read_exact(&mut check_buffer) {
            return Err(DecoderInitError::Io(e));
        }

        match variant_from_banner(&check_buffer) {
            Some(variant) => Ok(AssignmentReader {
                reader,
                sample_count: 0,
                variant,
                previous_assignment: None,
                twodelta_consumed_first_frame: false,
                silent: false,
                spinner: None,
            }),
            None => Err(DecoderInitError::InvalidFileFormat(check_buffer.to_vec())),
        }
    }

    /// Suppress progress output from this decoder's iterator.
    pub fn silent(mut self, silent: bool) -> Self {
        self.silent = silent;
        if silent {
            self.spinner = None;
        }
        self
    }

    /// Return the BEN variant detected from the stream banner.
    pub fn variant(&self) -> BenVariant {
        self.variant
    }

    /// Decode the remaining BEN stream and write it as JSONL.
    ///
    /// Each decoded sample is written as a JSON object containing an
    /// `assignment` vector and a 1-based `sample` index.
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

    /// Read the next frame from the underlying BEN stream.
    ///
    /// In a `TwoDelta` stream the first frame is encoded in `MkvChain` wire
    /// format; this method tracks that state so the frame module stays
    /// variant-clean.
    ///
    /// Returns `Some(Ok(...))` for the next frame, `Some(Err(...))` for a read
    /// failure, or `None` at a clean end of stream.
    fn pop_frame_from_reader(&mut self) -> Option<io::Result<BenDecodeFrame>> {
        let read_variant = if self.variant == BenVariant::TwoDelta
            && !self.twodelta_consumed_first_frame
        {
            self.twodelta_consumed_first_frame = true;
            BenVariant::MkvChain
        } else {
            self.variant
        };

        BenDecodeFrame::from_reader(&mut self.reader, read_variant).transpose()
    }

    /// Consume this decoder and iterate over raw BEN frames instead of
    /// materialized assignments.
    pub fn into_frames(self) -> AssignmentFrameReader<R> {
        AssignmentFrameReader { inner: self }
    }

    /// Count the number of samples remaining in the BEN stream.
    ///
    /// Walks frame boundaries rather than expanding every assignment.
    pub fn count_samples(self) -> io::Result<usize> {
        let mut this = self;
        let mut total = 0usize;
        while let Some(frame_res) = this.pop_frame_from_reader() {
            let count = frame_res?.count();
            if count == 0 {
                return Err(zero_count_frame_error());
            }
            total += count as usize;
        }
        Ok(total)
    }

    /// Decode assignments and pass each one to a callback by reference.
    ///
    /// Unlike the `Iterator` implementation, this avoids cloning the assignment
    /// buffer on every frame. The callback receives a borrowed slice and its
    /// repetition count. Return `true` to continue or `false` to stop early.
    pub fn for_each_assignment<F>(&mut self, mut f: F) -> io::Result<()>
    where
        F: FnMut(&[u16], u16) -> io::Result<bool>,
    {
        loop {
            let frame = match self.pop_frame_from_reader() {
                Some(Ok(frame)) => frame,
                Some(Err(e)) => return Err(e),
                None => return Ok(()),
            };

            let count = frame.count();
            if count == 0 {
                return Err(zero_count_frame_error());
            }

            let assignment = frame.expand(self.previous_assignment.take())?;

            let keep_going = f(&assignment, count)?;
            self.previous_assignment = Some(assignment);
            self.sample_count += count as usize;
            if !self.silent {
                self.spinner
                    .get_or_insert_with(|| Spinner::new("Decoding sample"))
                    .set_count(self.sample_count as u64);
            }
            if !keep_going {
                return Ok(());
            }
        }
    }
}

impl<R: Read> Iterator for AssignmentReader<R> {
    type Item = io::Result<super::subsample::MkvRecord>;

    fn next(&mut self) -> Option<io::Result<super::subsample::MkvRecord>> {
        let frame = match self.pop_frame_from_reader() {
            Some(Ok(frame)) => frame,
            Some(Err(e)) => return Some(Err(e)),
            None => return None,
        };
        let count = frame.count();
        if count == 0 {
            return Some(Err(zero_count_frame_error()));
        }
        let assignment = match frame.expand(self.previous_assignment.take()) {
            Ok(a) => a,
            Err(e) => return Some(Err(e)),
        };
        self.previous_assignment = Some(assignment.clone());
        self.sample_count += count as usize;
        if !self.silent {
            self.spinner
                .get_or_insert_with(|| Spinner::new("Decoding sample"))
                .set_count(self.sample_count as u64);
        }
        Some(Ok((assignment, count)))
    }
}

/// Iterator over raw BEN frames.
pub struct AssignmentFrameReader<R: Read> {
    pub(super) inner: AssignmentReader<R>,
}

impl<R: Read> AssignmentFrameReader<R> {
    /// Create a raw BEN frame iterator from a reader.
    pub fn new(reader: R) -> Result<Self, DecoderInitError> {
        Ok(Self {
            inner: AssignmentReader::new(reader)?,
        })
    }
}

impl<R: Read> Iterator for AssignmentFrameReader<R> {
    type Item = io::Result<(BenDecodeFrame, u16)>;

    /// Return the next raw BEN frame from the input stream paired with its
    /// repetition count.
    ///
    /// For `Standard` and `MkvChain` streams, returns the frame as read off
    /// the wire (with `count` taken from the frame for `MkvChain`, or `1`
    /// for `Standard`).
    ///
    /// For `TwoDelta` streams, materializes each assignment via `expand`
    /// and re-encodes it as a Standard-shaped decode frame so downstream
    /// subsampling consumers always see self-contained frames.
    fn next(&mut self) -> Option<Self::Item> {
        match self.inner.variant {
            BenVariant::Standard | BenVariant::MkvChain => {
                match self.inner.pop_frame_from_reader() {
                    Some(Ok(frame)) => {
                        let count = frame.count();
                        if count == 0 {
                            return Some(Err(zero_count_frame_error()));
                        }
                        Some(Ok((frame, count)))
                    }
                    Some(Err(e)) => Some(Err(e)),
                    None => None,
                }
            }
            BenVariant::TwoDelta => match self.inner.next() {
                Some(Ok((assignment, count))) => {
                    let encoded =
                        BenEncodeFrame::from_assignment(&assignment, BenVariant::Standard, None);
                    let (max_val_bit_count, max_len_bit_count, n_bytes, raw_bytes) = match encoded {
                        BenEncodeFrame::Standard {
                            max_val_bit_count,
                            max_len_bit_count,
                            n_bytes,
                            raw_bytes,
                            ..
                        } => (max_val_bit_count, max_len_bit_count, n_bytes, raw_bytes),
                        _ => unreachable!(
                            "BenEncodeFrame::from_assignment(Standard) always returns Standard"
                        ),
                    };
                    // Strip the 6-byte frame header so the emitted decode-side
                    // frame's raw_bytes matches the historical payload-only
                    // shape that BenDecodeFrame::Standard carries.
                    let payload_only = raw_bytes[6..].to_vec();
                    Some(Ok((
                        BenDecodeFrame::Standard {
                            max_val_bit_count,
                            max_len_bit_count,
                            n_bytes,
                            raw_bytes: payload_only,
                        },
                        count,
                    )))
                }
                Some(Err(err)) => Some(Err(err)),
                None => None,
            },
        }
    }
}

impl<R: Read + Send> AssignmentReader<R> {
    pub fn into_subsample_by_indices<T>(
        self,
        indices: T,
    ) -> super::subsample::SubsampleFrameDecoder<
        impl Iterator<Item = io::Result<(super::subsample::DecodeFrame, u16)>> + Send,
    >
    where
        T: IntoIterator<Item = usize>,
    {
        let frames = self
            .into_frames()
            .map(|res| res.map(|(f, cnt)| (super::subsample::DecodeFrame::Ben(f), cnt)));
        super::subsample::SubsampleFrameDecoder::by_indices(frames, indices)
    }

    pub fn into_subsample_by_range(
        self,
        start: usize,
        end: usize,
    ) -> super::subsample::SubsampleFrameDecoder<
        impl Iterator<Item = io::Result<(super::subsample::DecodeFrame, u16)>> + Send,
    > {
        let frames = self
            .into_frames()
            .map(|res| res.map(|(f, cnt)| (super::subsample::DecodeFrame::Ben(f), cnt)));
        super::subsample::SubsampleFrameDecoder::by_range(frames, start, end)
    }

    pub fn into_subsample_every(
        self,
        step: usize,
        offset: usize,
    ) -> super::subsample::SubsampleFrameDecoder<
        impl Iterator<Item = io::Result<(super::subsample::DecodeFrame, u16)>> + Send,
    > {
        let frames = self
            .into_frames()
            .map(|res| res.map(|(f, cnt)| (super::subsample::DecodeFrame::Ben(f), cnt)));
        super::subsample::SubsampleFrameDecoder::every(frames, step, offset)
    }
}
