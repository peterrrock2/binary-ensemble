use crate::codec::decode::{decode_ben32_line, decode_ben_line};
use crate::util::rle::rle_to_vec;
use crate::{progress, BenVariant};
use byteorder::{BigEndian, ReadBytesExt};
use serde_json::json;
use std::fs::File;
use std::io::{self, BufReader, Cursor, Read, Write};
use std::iter::Peekable;
use std::path::{Path, PathBuf};
use xz2::read::XzDecoder;

/// A decoded assignment together with the number of times it repeats.
pub type MkvRecord = (Vec<u16>, u16);
/// A raw ben32 frame together with the number of times it repeats.
pub type Ben32Frame = (Vec<u8>, u16);
/// A boxed iterator over generic BEN/XBEN frames used by subsampling helpers.
pub type FrameIter = Box<dyn Iterator<Item = io::Result<(Frame, u16)>> + Send>;

#[derive(Debug)]
/// Errors produced while validating the header of a decoder input stream.
pub enum DecoderInitError {
    /// The leading bytes did not match any supported BEN banner.
    InvalidFileFormat(Vec<u8>),
    /// An I/O error occurred while reading the header.
    Io(io::Error),
}

/// Check whether a header prefix matches the XZ file signature.
///
/// # Arguments
///
/// * `h` - The bytes to inspect.
///
/// # Returns
///
/// Returns `true` when `h` begins with the standard XZ magic bytes.
fn is_xz_header(h: &[u8]) -> bool {
    h.len() >= 6 && &h[..6] == b"\xFD\x37\x7A\x58\x5A\x00"
}

/// Convert a byte slice into a space-separated uppercase hex string.
///
/// # Arguments
///
/// * `bytes` - The bytes to render.
///
/// # Returns
///
/// Returns the formatted hex string.
fn to_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{:02X}", b))
        .collect::<Vec<_>>()
        .join(" ")
}

impl std::fmt::Display for DecoderInitError {
    /// Format the decoder initialization error for display.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "IO error: {e}"),
            Self::InvalidFileFormat(header) => {
                if is_xz_header(header) {
                    write!(
                        f,
                        "Invalid file format: Compressed header detected (hex: {}). \
                     This reader expects an uncompressed .ben file. \
                     Decompress this file using the BEN cli `ben -m decode <file_name>.xben` tool \
                     or the `decode_xben_to_ben` function in this library.",
                        to_hex(header)
                    )
                } else {
                    let lossy = String::from_utf8_lossy(header);
                    write!(
                        f,
                        "Invalid file format. Found header (utf8-lossy: {lossy:?}, hex: {})",
                        to_hex(header)
                    )
                }
            }
        }
    }
}

impl std::error::Error for DecoderInitError {
    /// Return the underlying source error when one exists.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DecoderInitError::Io(e) => Some(e),
            DecoderInitError::InvalidFileFormat(_) => None,
        }
    }
}

impl From<io::Error> for DecoderInitError {
    /// Wrap a plain I/O error as a decoder initialization error.
    fn from(error: io::Error) -> Self {
        DecoderInitError::Io(error)
    }
}

impl From<DecoderInitError> for io::Error {
    /// Convert a decoder initialization error into a plain I/O error.
    fn from(error: DecoderInitError) -> Self {
        match error {
            DecoderInitError::Io(e) => e,
            DecoderInitError::InvalidFileFormat(msg) => {
                io::Error::new(io::ErrorKind::InvalidData, format!("{msg:?}"))
            }
        }
    }
}

/// Iterator over decoded assignments in an uncompressed BEN stream.
pub struct BenDecoder<R: Read> {
    reader: R,
    sample_count: usize,
    variant: BenVariant,
}

#[derive(Clone)]
/// A single raw BEN frame.
///
/// `raw_data` contains only the packed `(value, run_length)` payload and does
/// not include the outer frame header fields.
pub struct BenFrame {
    /// Number of bits used to encode each label value in `raw_data`.
    pub max_val_bits: u8,
    /// Number of bits used to encode each run length in `raw_data`.
    pub max_len_bits: u8,
    /// Number of repeated samples represented by this frame.
    pub count: u16,
    /// Length in bytes of the packed payload stored in `raw_data`.
    pub n_bytes: u32,
    /// Packed BEN payload for this frame.
    pub raw_data: Vec<u8>,
}

impl<R: Read> BenDecoder<R> {
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
        let mut check_buffer = [0u8; 17];

        if let Err(e) = reader.read_exact(&mut check_buffer) {
            return Err(DecoderInitError::Io(e));
        }

        match &check_buffer {
            b"STANDARD BEN FILE" => Ok(BenDecoder {
                reader,
                sample_count: 0,
                variant: BenVariant::Standard,
            }),
            b"MKVCHAIN BEN FILE" => Ok(BenDecoder {
                reader,
                sample_count: 0,
                variant: BenVariant::MkvChain,
            }),
            _ => Err(DecoderInitError::InvalidFileFormat(check_buffer.to_vec())),
        }
    }

    /// Decode the remaining BEN stream and write it as JSONL.
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
        while let Some(result_tuple) = self.next() {
            match result_tuple {
                Ok((assignment, count)) => {
                    for _ in 0..count {
                        self.sample_count += 1;
                        let line = json!({
                            "assignment": assignment,
                            "sample": self.sample_count,
                        })
                        .to_string()
                            + "\n";
                        writer.write_all(line.as_bytes()).unwrap();
                    }
                }
                Err(e) => {
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    /// Read and return the next raw BEN frame from the underlying stream.
    ///
    /// # Returns
    ///
    /// Returns `Some(Ok(...))` for the next frame, `Some(Err(...))` for a read
    /// failure, or `None` at a clean end of stream.
    fn pop_frame_from_reader(&mut self) -> Option<io::Result<BenFrame>> {
        let mut b1 = [0u8; 1];
        let max_val_bits = match self.reader.read_exact(&mut b1) {
            Ok(()) => b1[0],
            Err(e) => {
                if e.kind() == io::ErrorKind::UnexpectedEof {
                    tracing::trace!("");
                    tracing::trace!("Done!");
                    return None;
                }
                return Some(Err(e));
            }
        };

        let mut b2 = [0u8; 1];
        if let Err(e) = self.reader.read_exact(&mut b2) {
            return Some(Err(e));
        }
        let max_len_bits = b2[0];

        let n_bytes = match self.reader.read_u32::<BigEndian>() {
            Ok(n) => n,
            Err(e) => return Some(Err(e)),
        };

        let mut raw_assignment = vec![0u8; n_bytes as usize];
        if let Err(e) = self.reader.read_exact(&mut raw_assignment) {
            return Some(Err(e));
        }

        let count = if self.variant == BenVariant::MkvChain {
            match self.reader.read_u16::<BigEndian>() {
                Ok(c) => c,
                Err(e) => return Some(Err(e)),
            }
        } else {
            1
        };

        Some(Ok(BenFrame {
            max_val_bits,
            max_len_bits,
            n_bytes,
            raw_data: raw_assignment,
            count,
        }))
    }

    /// Consume this decoder and iterate over raw BEN frames instead of
    /// materialized assignments.
    ///
    /// # Returns
    ///
    /// Returns an iterator that yields raw BEN frames from the remaining input.
    pub fn into_frames(self) -> BenFrameDecoeder<R> {
        BenFrameDecoeder { inner: self }
    }

    /// Count the number of samples remaining in the BEN stream.
    ///
    /// This consumes the decoder but only walks frame boundaries rather than
    /// expanding every assignment into a full vector.
    ///
    /// # Returns
    ///
    /// Returns the number of remaining samples in the stream.
    pub fn count_samples(self) -> io::Result<usize> {
        let mut total = 0usize;
        for frame_res in self.into_frames() {
            let f = frame_res?;
            total += f.count as usize;
        }
        Ok(total)
    }
}

/// Decode a raw BEN frame into a full assignment vector.
///
/// # Arguments
///
/// * `frame` - The raw BEN frame to decode.
///
/// # Returns
///
/// Returns the expanded assignment vector.
fn decode_ben_frame_to_assignment(frame: &BenFrame) -> io::Result<Vec<u16>> {
    decode_ben_line(
        Cursor::new(&frame.raw_data),
        frame.max_val_bits,
        frame.max_len_bits,
        frame.n_bytes,
    )
    .map(rle_to_vec)
}

impl<R: Read> Iterator for BenDecoder<R> {
    type Item = io::Result<MkvRecord>;

    /// Decode and return the next assignment from the BEN stream.
    fn next(&mut self) -> Option<io::Result<MkvRecord>> {
        let ben_frame = match self.pop_frame_from_reader() {
            Some(Ok(frame)) => frame,
            Some(Err(e)) => return Some(Err(e)),
            None => return None,
        };
        let assignment = match decode_ben_frame_to_assignment(&ben_frame) {
            Ok(assgn) => assgn,
            Err(e) => return Some(Err(e)),
        };
        progress!(
            "Decoding sample: {}\r",
            self.sample_count + ben_frame.count as usize
        );
        Some(Ok((assignment, ben_frame.count)))
    }
}

/// Iterator over raw BEN frames.
pub struct BenFrameDecoeder<R: Read> {
    inner: BenDecoder<R>,
}

impl<R: Read> BenFrameDecoeder<R> {
    /// Create a raw BEN frame iterator from a reader.
    ///
    /// # Arguments
    ///
    /// * `reader` - The input BEN stream, including its 17-byte banner.
    ///
    /// # Returns
    ///
    /// Returns an iterator over raw BEN frames.
    pub fn new(reader: R) -> io::Result<Self> {
        Ok(Self {
            inner: BenDecoder::new(reader)?,
        })
    }
}

impl<R: Read> Iterator for BenFrameDecoeder<R> {
    type Item = io::Result<BenFrame>;

    /// Return the next raw BEN frame from the input stream.
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.pop_frame_from_reader()
    }
}

/// Iterator over decoded assignments in an XBEN stream.
pub struct XBenDecoder<R: Read> {
    xz: BufReader<XzDecoder<R>>,
    /// Variant encoded in the XBEN banner.
    pub variant: BenVariant,
    overflow: Vec<u8>,
    buf: Box<[u8]>,
}

impl<R: Read> XBenDecoder<R> {
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
    pub fn new(reader: R) -> io::Result<Self> {
        let xz = XzDecoder::new(reader);
        let mut xz = BufReader::with_capacity(1 << 20, xz);

        let mut first = [0u8; 17];
        xz.read_exact(&mut first)?;
        let variant = match &first {
            b"STANDARD BEN FILE" => BenVariant::Standard,
            b"MKVCHAIN BEN FILE" => BenVariant::MkvChain,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Invalid .xben header (expecting STANDARD/MKVCHAIN BEN FILE)",
                ));
            }
        };

        Ok(Self {
            xz,
            variant,
            overflow: Vec::with_capacity(1 << 20),
            buf: vec![0u8; 1 << 20].into_boxed_slice(),
        })
    }

    /// Try to extract one complete ben32 frame from the buffered overflow.
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
        match self.variant {
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
        }
    }

    /// Consume this decoder and iterate over raw ben32 frames instead of
    /// materialized assignments.
    ///
    /// # Returns
    ///
    /// Returns an iterator that yields raw ben32 frames from the remaining
    /// input.
    pub fn into_frames(self) -> XBenFrameDecoder<R> {
        XBenFrameDecoder { inner: self }
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
fn decode_xben_frame_to_assignment(
    frame_bytes: &[u8],
    variant: BenVariant,
) -> io::Result<Vec<u16>> {
    let cursor = Cursor::new(frame_bytes);
    let (assignment, _) = decode_ben32_line(cursor, variant)?;
    Ok(assignment)
}

impl<R: Read> Iterator for XBenDecoder<R> {
    type Item = io::Result<MkvRecord>;

    /// Decode and return the next assignment from the XBEN stream.
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some((frame_bytes, consumed, count)) =
                self.pop_frame_from_overflow(&self.overflow)
            {
                let res = match decode_xben_frame_to_assignment(frame_bytes, self.variant) {
                    Ok(assignment) => Ok((assignment, count)),
                    Err(e) => Err(e),
                };
                self.overflow.drain(..consumed);
                return Some(res);
            }

            let read = match self.xz.read(&mut self.buf) {
                Ok(0) => {
                    if self.overflow.is_empty() {
                        return None;
                    } else {
                        return Some(Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "truncated .xben stream (partial frame at EOF)",
                        )));
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
pub struct XBenFrameDecoder<R: Read> {
    inner: XBenDecoder<R>,
}

impl<R: Read> XBenFrameDecoder<R> {
    /// Create a raw XBEN frame iterator from a reader.
    ///
    /// # Arguments
    ///
    /// * `reader` - The compressed XBEN input stream.
    ///
    /// # Returns
    ///
    /// Returns an iterator over raw ben32 frames.
    pub fn new(reader: R) -> io::Result<Self> {
        Ok(Self {
            inner: XBenDecoder::new(reader)?,
        })
    }
}

impl<R: Read> Iterator for XBenFrameDecoder<R> {
    type Item = io::Result<Ben32Frame>;

    /// Return the next raw ben32 frame from the input stream.
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some((frame, consumed, count)) =
                self.inner.pop_frame_from_overflow(&self.inner.overflow)
            {
                let out = frame.to_vec();
                self.inner.overflow.drain(..consumed);
                return Some(Ok((out, count)));
            }

            let read = match self.inner.xz.read(&mut self.inner.buf) {
                Ok(0) => {
                    if self.inner.overflow.is_empty() {
                        return None;
                    } else {
                        return Some(Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "truncated .xben stream (partial frame at EOF)",
                        )));
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

#[derive(Clone)]
/// A generalized frame type used by the subsampling machinery.
pub enum Frame {
    /// A raw BEN frame.
    Ben(BenFrame),
    /// A raw ben32 frame from an XBEN stream together with its variant.
    XBen(Vec<u8>, BenVariant),
}

/// A selection strategy for extracting only part of a frame stream.
pub enum Selection {
    /// Select explicit 1-based indices.
    Indices(Peekable<std::vec::IntoIter<usize>>),
    /// Select every `step` samples starting at the 1-based `offset`.
    Every { step: usize, offset: usize },
    /// Select the inclusive 1-based range `[start, end]`.
    Range { start: usize, end: usize },
}

/// Decode a generic frame into a full assignment vector.
///
/// # Arguments
///
/// * `frame` - Either a BEN frame or an XBEN ben32 frame.
///
/// # Returns
///
/// Returns the expanded assignment vector.
fn decode_frame_to_assignment(frame: &Frame) -> io::Result<Vec<u16>> {
    match frame {
        Frame::Ben(f) => decode_ben_frame_to_assignment(f),
        Frame::XBen(bytes, variant) => decode_xben_frame_to_assignment(bytes, *variant),
    }
}

/// Iterator adaptor that decodes only selected samples from a frame stream.
pub struct SubsampleFrameDecoder<I>
where
    I: Iterator<Item = io::Result<(Frame, u16)>>,
{
    inner: I,
    selection: Selection,
    sample: usize,
}

impl<I> SubsampleFrameDecoder<I>
where
    I: Iterator<Item = io::Result<(Frame, u16)>>,
{
    /// Create a subsampling iterator from a lower-level frame iterator.
    ///
    /// # Arguments
    ///
    /// * `inner` - The source iterator yielding frames and repetition counts.
    /// * `selection` - The sample-selection rule to apply.
    ///
    /// # Returns
    ///
    /// Returns a decoder that yields only the selected samples.
    pub fn new(inner: I, selection: Selection) -> Self {
        Self {
            inner,
            selection,
            sample: 0,
        }
    }

    /// Select a set of 1-based sample indices.
    ///
    /// Indices are sorted and deduplicated before iteration begins.
    ///
    /// # Arguments
    ///
    /// * `inner` - The source iterator yielding frames and repetition counts.
    /// * `indices` - A collection of 1-based sample indices.
    ///
    /// # Returns
    ///
    /// Returns a decoder that yields only the selected samples.
    pub fn by_indices<T>(inner: I, indices: T) -> Self
    where
        T: IntoIterator<Item = usize>,
    {
        let mut v: Vec<usize> = indices.into_iter().collect();
        v.sort_unstable();
        v.dedup();
        Self::new(inner, Selection::Indices(v.into_iter().peekable()))
    }

    /// Select the inclusive 1-based range `[start, end]`.
    ///
    /// # Arguments
    ///
    /// * `inner` - The source iterator yielding frames and repetition counts.
    /// * `start` - The first 1-based sample index to include.
    /// * `end` - The last 1-based sample index to include.
    ///
    /// # Returns
    ///
    /// Returns a decoder that yields only the selected samples.
    pub fn by_range(inner: I, start: usize, end: usize) -> Self {
        assert!(
            start >= 1 && end >= start,
            "range must be 1-based and end >= start"
        );
        Self::new(inner, Selection::Range { start, end })
    }

    /// Select every `step` samples beginning from the 1-based `offset`.
    ///
    /// # Arguments
    ///
    /// * `inner` - The source iterator yielding frames and repetition counts.
    /// * `step` - The stride between selected samples.
    /// * `offset` - The 1-based index of the first selected sample.
    ///
    /// # Returns
    ///
    /// Returns a decoder that yields only the selected samples.
    pub fn every(inner: I, step: usize, offset: usize) -> Self {
        assert!(step >= 1 && offset >= 1, "step and offset must be >= 1");
        Self::new(inner, Selection::Every { step, offset })
    }

    /// Count how many selected samples fall within an inclusive sample interval.
    ///
    /// # Arguments
    ///
    /// * `lo` - The first 1-based sample index covered by the current frame.
    /// * `hi` - The last 1-based sample index covered by the current frame.
    ///
    /// # Returns
    ///
    /// Returns the number of selected samples represented by the frame.
    fn count_selected_in(&mut self, lo: usize, hi: usize) -> u16 {
        match &mut self.selection {
            Selection::Indices(iter) => {
                let mut taken = 0u16;
                while let Some(&next) = iter.peek() {
                    if next < lo {
                        iter.next();
                        continue;
                    }
                    if next > hi {
                        break;
                    }
                    iter.next();
                    taken = taken.saturating_add(1);
                }
                taken
            }
            Selection::Every { step, offset } => {
                let start = lo.max(*offset);
                if start > hi {
                    return 0;
                }
                let r = (start as isize - *offset as isize).rem_euclid(*step as isize) as usize;
                let first = start + ((*step - r) % *step);
                if first > hi {
                    0
                } else {
                    (1 + (hi - first) / *step) as u16
                }
            }
            Selection::Range { start, end } => {
                if hi < *start || lo > *end {
                    0
                } else {
                    let a = lo.max(*start);
                    let b = hi.min(*end);
                    (b - a + 1) as u16
                }
            }
        }
    }
}

impl<I> Iterator for SubsampleFrameDecoder<I>
where
    I: Iterator<Item = io::Result<(Frame, u16)>>,
{
    type Item = io::Result<MkvRecord>;

    /// Return the next decoded sample selected by the subsampling rule.
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Selection::Range { end, .. } = self.selection {
                if self.sample >= end {
                    return None;
                }
            }
            if let Selection::Indices(ref mut it) = self.selection {
                if it.peek().is_none() {
                    return None;
                }
            }

            let (frame, count) = match self.inner.next()? {
                Ok(x) => x,
                Err(e) => return Some(Err(e)),
            };

            let lo = self.sample + 1;
            let hi = self.sample + count as usize;
            let selected = self.count_selected_in(lo, hi);

            self.sample = hi;

            if selected > 0 {
                match decode_frame_to_assignment(&frame) {
                    Ok(assignment) => return Some(Ok((assignment, selected))),
                    Err(e) => return Some(Err(e)),
                }
            }
        }
    }
}

/// Build a generic frame iterator from a BEN or XBEN file path.
///
/// Frame iteration is useful for subsampling and counting because it avoids
/// decoding every sample into a full assignment vector.
///
/// # Arguments
///
/// * `file_path` - Path to a `.ben` or `.xben` file.
/// * `mode` - Either `"ben"` or `"xben"`.
///
/// # Returns
///
/// Returns a boxed iterator over generic frames and their repetition counts.
pub fn build_frame_iter(file_path: &PathBuf, mode: &str) -> io::Result<FrameIter> {
    let file = File::options().read(true).open(file_path)?;
    let reader = BufReader::new(file);

    match mode {
        "ben" => {
            let frames = BenFrameDecoeder::new(reader)?;
            let mapped = frames.map(|res| {
                res.map(|f| {
                    let cnt = f.count;
                    (Frame::Ben(f), cnt)
                })
            });
            Ok(Box::new(mapped))
        }
        "xben" => {
            let x = XBenDecoder::new(reader)?;
            let variant = x.variant;
            let frames = x.into_frames();
            let mapped =
                frames.map(move |res| res.map(|(bytes, cnt)| (Frame::XBen(bytes, variant), cnt)));
            Ok(Box::new(mapped))
        }
        _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "Unknown mode")),
    }
}

impl<R: Read + Send> BenDecoder<R> {
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
    ) -> SubsampleFrameDecoder<impl Iterator<Item = io::Result<(Frame, u16)>> + Send>
    where
        T: IntoIterator<Item = usize>,
    {
        let frames = self.into_frames().map(|res| {
            res.map(|f| {
                let count = f.count;
                (Frame::Ben(f), count)
            })
        });
        SubsampleFrameDecoder::by_indices(frames, indices)
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
    ) -> SubsampleFrameDecoder<impl Iterator<Item = io::Result<(Frame, u16)>> + Send> {
        let frames = self.into_frames().map(|res| {
            res.map(|f| {
                let cnt = f.count;
                (Frame::Ben(f), cnt)
            })
        });
        SubsampleFrameDecoder::by_range(frames, start, end)
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
    ) -> SubsampleFrameDecoder<impl Iterator<Item = io::Result<(Frame, u16)>> + Send> {
        let frames = self.into_frames().map(|res| {
            res.map(|f| {
                let cnt = f.count;
                (Frame::Ben(f), cnt)
            })
        });
        SubsampleFrameDecoder::every(frames, step, offset)
    }
}

impl<R: Read + Send> XBenDecoder<R> {
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
    ) -> SubsampleFrameDecoder<impl Iterator<Item = io::Result<(Frame, u16)>> + Send>
    where
        T: IntoIterator<Item = usize>,
    {
        let variant = self.variant;
        let frames = self
            .into_frames()
            .map(move |res| res.map(|(bytes, cnt)| (Frame::XBen(bytes, variant), cnt)));
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
    ) -> SubsampleFrameDecoder<impl Iterator<Item = io::Result<(Frame, u16)>> + Send> {
        let variant = self.variant;
        let frames = self
            .into_frames()
            .map(move |res| res.map(|(bytes, cnt)| (Frame::XBen(bytes, variant), cnt)));
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
    ) -> SubsampleFrameDecoder<impl Iterator<Item = io::Result<(Frame, u16)>> + Send> {
        let variant = self.variant;
        let frames = self
            .into_frames()
            .map(move |res| res.map(|(bytes, cnt)| (Frame::XBen(bytes, variant), cnt)));
        SubsampleFrameDecoder::every(Box::new(frames), step, offset)
    }
}

/// Count the number of samples in a BEN or XBEN file on disk.
///
/// The file is walked frame-by-frame, so this is linear in file size but avoids
/// materializing full assignment vectors.
///
/// # Arguments
///
/// * `path` - Path to a `.ben` or `.xben` file.
/// * `mode` - Either `"ben"` or `"xben"`.
///
/// # Returns
///
/// Returns the number of samples in the file.
pub fn count_samples_from_file(path: &Path, mode: &str) -> io::Result<usize> {
    let iter = build_frame_iter(&path.to_path_buf(), mode)?;
    let mut total = 0usize;
    for item in iter {
        let (_frame, cnt) = item?;
        total += cnt as usize;
    }
    Ok(total)
}
