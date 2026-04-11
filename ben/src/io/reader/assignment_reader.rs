use super::errors::DecoderInitError;
use crate::codec::decode::{apply_twodelta_runs_to_assignment, decode_ben_line, DecodeError};
use crate::codec::{
    BenConstruct, BenDecode, BenDecodeFrame, BenEncodeFrame, MkvBenDecodeFrame, TwoDeltaDecodeFrame,
};
use crate::format::banners::{variant_from_banner, BANNER_LEN};
use crate::util::rle::rle_to_vec;
use crate::{progress, BenVariant};
use serde_json::json;
use std::io::{self, Cursor, Read, Write};

/// Iterator over decoded assignments in an uncompressed BEN stream.
pub struct AssignmentReader<R: Read> {
    reader: R,
    sample_count: usize,
    variant: BenVariant,
    previous_assignment: Option<Vec<u16>>,
    twodelta_consumed_first_frame: bool,
    silent: bool,
}

/// Internal frame representation, one variant per BEN encoding type.
enum StoredBenFrame {
    /// A Standard BEN frame (count is always 1).
    Standard(BenDecodeFrame),
    /// An MkvChain BEN frame carrying its repetition count.
    MkvChain(MkvBenDecodeFrame),
    /// A TwoDelta delta frame carrying its pair, run lengths, and count.
    TwoDelta(TwoDeltaDecodeFrame),
}

impl StoredBenFrame {
    fn count(&self) -> u16 {
        match self {
            Self::Standard(_) => 1,
            Self::MkvChain(f) => f.count,
            Self::TwoDelta(f) => f.count,
        }
    }
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
            }),
            None => Err(DecoderInitError::InvalidFileFormat(check_buffer.to_vec())),
        }
    }

    /// Suppress progress output from this decoder's iterator.
    pub fn silent(mut self, silent: bool) -> Self {
        self.silent = silent;
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

    /// Read and return the next stored frame from the underlying BEN stream.
    ///
    /// Delegates to the appropriate `BenDecode::from_reader` implementation
    /// based on the variant and whether the first TwoDelta frame has been read.
    ///
    /// Returns `Some(Ok(...))` for the next frame, `Some(Err(...))` for a read
    /// failure, or `None` at a clean end of stream.
    fn pop_frame_from_reader(&mut self) -> Option<io::Result<StoredBenFrame>> {
        match self.variant {
            BenVariant::Standard => BenDecodeFrame::from_reader(&mut self.reader)
                .transpose()
                .map(|r| r.map(StoredBenFrame::Standard)),
            BenVariant::MkvChain => MkvBenDecodeFrame::from_reader(&mut self.reader)
                .transpose()
                .map(|r| r.map(StoredBenFrame::MkvChain)),
            BenVariant::TwoDelta => {
                if !self.twodelta_consumed_first_frame {
                    // First TwoDelta frame is encoded in MkvChain format.
                    self.twodelta_consumed_first_frame = true;
                    MkvBenDecodeFrame::from_reader(&mut self.reader)
                        .transpose()
                        .map(|r| r.map(StoredBenFrame::MkvChain))
                } else {
                    TwoDeltaDecodeFrame::from_reader(&mut self.reader)
                        .transpose()
                        .map(|r| r.map(StoredBenFrame::TwoDelta))
                }
            }
        }
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

            let assignment = match frame {
                StoredBenFrame::Standard(f) => decode_ben_frame_to_assignment(&f)?,
                StoredBenFrame::MkvChain(f) => decode_mkv_frame_to_assignment(&f)?,
                StoredBenFrame::TwoDelta(f) => {
                    let prev = self
                        .previous_assignment
                        .take()
                        .ok_or_else(|| io::Error::from(DecodeError::TwoDeltaNoAnchorFrame))?;
                    apply_twodelta_runs_to_assignment(prev, f.pair, &f.run_lengths)?
                }
            };

            let keep_going = f(&assignment, count)?;
            self.previous_assignment = Some(assignment);
            self.sample_count += count as usize;
            if !self.silent {
                progress!("Decoding sample: {}\r", self.sample_count);
            }
            if !keep_going {
                return Ok(());
            }
        }
    }
}

/// Decode a raw Standard BEN frame into a full assignment vector.
pub(super) fn decode_ben_frame_to_assignment(frame: &BenDecodeFrame) -> io::Result<Vec<u16>> {
    decode_ben_line(
        Cursor::new(&frame.raw_bytes),
        frame.max_val_bit_count,
        frame.max_len_bit_count,
        frame.n_bytes,
    )
    .map(rle_to_vec)
}

/// Decode a raw MkvChain BEN frame into a full assignment vector.
pub(super) fn decode_mkv_frame_to_assignment(frame: &MkvBenDecodeFrame) -> io::Result<Vec<u16>> {
    decode_ben_line(
        Cursor::new(&frame.raw_bytes),
        frame.max_val_bit_count,
        frame.max_len_bit_count,
        frame.n_bytes,
    )
    .map(rle_to_vec)
}

/// Decode a stored BEN frame into a full assignment vector.
fn decode_stored_frame_to_assignment(
    previous_assignment: &mut Option<Vec<u16>>,
    frame: &StoredBenFrame,
) -> io::Result<Vec<u16>> {
    match frame {
        StoredBenFrame::Standard(f) => decode_ben_frame_to_assignment(f),
        StoredBenFrame::MkvChain(f) => decode_mkv_frame_to_assignment(f),
        StoredBenFrame::TwoDelta(f) => {
            let prev = previous_assignment
                .take()
                .ok_or_else(|| io::Error::from(DecodeError::TwoDeltaNoAnchorFrame))?;
            apply_twodelta_runs_to_assignment(prev, f.pair, &f.run_lengths)
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
        let assignment =
            match decode_stored_frame_to_assignment(&mut self.previous_assignment, &frame) {
                Ok(assgn) => assgn,
                Err(e) => return Some(Err(e)),
            };
        self.previous_assignment = Some(assignment.clone());
        self.sample_count += count as usize;
        if !self.silent {
            progress!("Decoding sample: {}\r", self.sample_count);
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

    /// Return the next raw BEN frame from the input stream.
    ///
    /// For Standard and MkvChain streams, returns the raw decoded frame paired
    /// with its repetition count.
    /// For TwoDelta streams, materializes each assignment and re-encodes it.
    fn next(&mut self) -> Option<Self::Item> {
        match self.inner.variant {
            BenVariant::Standard | BenVariant::MkvChain => {
                match self.inner.pop_frame_from_reader() {
                    Some(Ok(StoredBenFrame::Standard(frame))) => Some(Ok((frame, 1))),
                    Some(Ok(StoredBenFrame::MkvChain(frame))) => {
                        let count = frame.count;
                        if count == 0 {
                            return Some(Err(zero_count_frame_error()));
                        }
                        Some(Ok((
                            BenDecodeFrame {
                                max_val_bit_count: frame.max_val_bit_count,
                                max_len_bit_count: frame.max_len_bit_count,
                                n_bytes: frame.n_bytes,
                                raw_bytes: frame.raw_bytes,
                            },
                            count,
                        )))
                    }
                    Some(Ok(StoredBenFrame::TwoDelta(_))) => {
                        Some(Err(io::Error::from(DecodeError::UnexpectedTwoDeltaFrame {
                            variant: self.inner.variant,
                        })))
                    }
                    Some(Err(err)) => Some(Err(err)),
                    None => None,
                }
            }
            BenVariant::TwoDelta => match self.inner.next() {
                Some(Ok((assignment, count))) => {
                    let encoded = BenEncodeFrame::from_assignment(&assignment, None);
                    Some(Ok((
                        BenDecodeFrame {
                            max_val_bit_count: encoded.max_val_bit_count,
                            max_len_bit_count: encoded.max_len_bit_count,
                            n_bytes: encoded.n_bytes,
                            raw_bytes: encoded.raw_bytes[6..].to_vec(),
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
