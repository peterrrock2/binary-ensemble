use crate::codec::decode::{decode_ben32_line, decode_ben_line};
use crate::codec::encode::{encode_ben32_assignments, encode_ben_vec_from_assign, TwoDeltaFrame};
use crate::format::banners::{variant_from_banner, BANNER_LEN};
use crate::util::rle::rle_to_vec;
use crate::{progress, BenVariant};
use byteorder::{BigEndian, ReadBytesExt};
use serde_json::json;
use std::fs::File;
use std::io::{self, BufReader, Cursor, Read, Write};
use std::iter::Peekable;
use std::path::{Path, PathBuf};
use xz2::read::XzDecoder;

const XBEN_TWODELTA_FULL_TAG: u8 = 0;
const XBEN_TWODELTA_DELTA_TAG: u8 = 1;
const XBEN_TWODELTA_CHUNK_TAG: u8 = 2;

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
    previous_assignment: Option<Vec<u16>>,
    twodelta_consumed_first_frame: bool,
    silent: bool,
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

enum StoredBenFrame {
    Ben(BenFrame),
    TwoDelta { frame: TwoDeltaFrame, count: u16 },
}

enum XBenTwoDeltaFrame {
    Full {
        runs: Vec<(u16, u16)>,
    },
    Delta {
        pair: (u16, u16),
        run_lengths: Vec<u16>,
    },
}

impl StoredBenFrame {
    fn count(&self) -> u16 {
        match self {
            Self::Ben(frame) => frame.count,
            Self::TwoDelta { count, .. } => *count,
        }
    }
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
        let mut check_buffer = [0u8; BANNER_LEN];

        if let Err(e) = reader.read_exact(&mut check_buffer) {
            return Err(DecoderInitError::Io(e));
        }

        match variant_from_banner(&check_buffer) {
            Some(variant) => Ok(BenDecoder {
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

    /// Read and return the next raw BEN frame stored in standard BEN layout.
    ///
    /// # Arguments
    ///
    /// * `with_count` - When `true`, read a trailing `u16` repetition count;
    ///   otherwise the count defaults to `1`.
    ///
    /// # Returns
    ///
    /// Returns `Some(Ok(...))` for the next frame, `Some(Err(...))` for a read
    /// failure, or `None` at a clean end of stream.
    fn pop_standard_frame_from_reader(&mut self, with_count: bool) -> Option<io::Result<BenFrame>> {
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

        let count = if with_count {
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

    /// Read and return the next raw TwoDelta frame from the underlying stream.
    ///
    /// # Returns
    ///
    /// Returns `Some(Ok(...))` for the next TwoDelta frame, `Some(Err(...))`
    /// for a read failure, or `None` at a clean end of stream.
    fn pop_twodelta_frame_from_reader(&mut self) -> Option<io::Result<StoredBenFrame>> {
        let pair_a = match self.reader.read_u16::<BigEndian>() {
            Ok(value) => value,
            Err(e) => {
                if e.kind() == io::ErrorKind::UnexpectedEof {
                    tracing::trace!("");
                    tracing::trace!("Done!");
                    return None;
                }
                return Some(Err(e));
            }
        };

        let pair_b = match self.reader.read_u16::<BigEndian>() {
            Ok(value) => value,
            Err(e) => return Some(Err(e)),
        };

        let mut bits = [0u8; 1];
        if let Err(e) = self.reader.read_exact(&mut bits) {
            return Some(Err(e));
        }
        let max_len_bits = bits[0];

        let n_bytes = match self.reader.read_u32::<BigEndian>() {
            Ok(value) => value,
            Err(e) => return Some(Err(e)),
        };

        let mut payload = vec![0u8; n_bytes as usize];
        if let Err(e) = self.reader.read_exact(&mut payload) {
            return Some(Err(e));
        }

        let count = match self.reader.read_u16::<BigEndian>() {
            Ok(value) => value,
            Err(e) => return Some(Err(e)),
        };

        Some(Ok(StoredBenFrame::TwoDelta {
            frame: TwoDeltaFrame::from_parts((pair_a, pair_b), max_len_bits, payload),
            count,
        }))
    }

    /// Read and return the next stored frame from the underlying BEN stream.
    ///
    /// # Arguments
    ///
    /// * `&mut self` - The decoder whose internal reader is advanced.
    ///
    /// # Returns
    ///
    /// Returns `Some(Ok(...))` for the next frame, `Some(Err(...))` for a read
    /// failure, or `None` at a clean end of stream.
    fn pop_frame_from_reader(&mut self) -> Option<io::Result<StoredBenFrame>> {
        match self.variant {
            BenVariant::Standard => self
                .pop_standard_frame_from_reader(false)
                .map(|res| res.map(StoredBenFrame::Ben)),
            BenVariant::MkvChain => self
                .pop_standard_frame_from_reader(true)
                .map(|res| res.map(StoredBenFrame::Ben)),
            BenVariant::TwoDelta => {
                if !self.twodelta_consumed_first_frame {
                    self.twodelta_consumed_first_frame = true;
                    self.pop_standard_frame_from_reader(true)
                        .map(|res| res.map(StoredBenFrame::Ben))
                } else {
                    self.pop_twodelta_frame_from_reader()
                }
            }
        }
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
        let mut this = self;
        let mut total = 0usize;
        while let Some(frame_res) = this.pop_frame_from_reader() {
            total += frame_res?.count() as usize;
        }
        Ok(total)
    }

    /// Decode assignments and pass each one to a callback by reference.
    ///
    /// Unlike the `Iterator` implementation, this avoids cloning the assignment
    /// buffer on every frame. The decoder owns a single buffer, mutates it in
    /// place for TwoDelta frames, and lends `&[u16]` to the callback. This
    /// eliminates one full-length memcpy per frame.
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
        loop {
            let frame = match self.pop_frame_from_reader() {
                Some(Ok(frame)) => frame,
                Some(Err(e)) => return Err(e),
                None => return Ok(()),
            };

            let count = frame.count();

            match frame {
                StoredBenFrame::Ben(ben_frame) => {
                    let assignment = decode_ben_frame_to_assignment(&ben_frame)?;
                    let keep_going = f(&assignment, count)?;
                    self.previous_assignment = Some(assignment);
                    if !keep_going {
                        return Ok(());
                    }
                }
                StoredBenFrame::TwoDelta { frame, count } => {
                    let assignment = self.previous_assignment.take().ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            "TwoDelta frame encountered before an initial BEN frame",
                        )
                    })?;
                    let run_lengths = decode_twodelta_run_lengths(&frame)?;
                    let assignment =
                        apply_twodelta_runs_to_assignment(assignment, frame.pair, &run_lengths)?;
                    let keep_going = f(&assignment, count)?;
                    self.previous_assignment = Some(assignment);
                    if !keep_going {
                        return Ok(());
                    }
                }
            }

            self.sample_count += count as usize;
            if !self.silent {
                progress!("Decoding sample: {}\r", self.sample_count);
            }
        }
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

/// Decode the run-length payload of a TwoDelta frame.
///
/// # Arguments
///
/// * `frame` - The TwoDelta frame whose packed payload is decoded.
///
/// # Returns
///
/// Returns the sequence of non-zero run lengths extracted from the payload.
pub(crate) fn decode_twodelta_run_lengths(frame: &TwoDeltaFrame) -> io::Result<Vec<u16>> {
    let mut items = Vec::new();
    let mut buffer: u32 = 0;
    let mut n_bits_in_buff: u16 = 0;
    let mut current: Option<u16> = None;

    for &byte in frame.payload() {
        buffer |= (byte as u32).to_be() >> n_bits_in_buff;
        n_bits_in_buff += 8;

        if n_bits_in_buff >= frame.max_len_bits as u16 && current.is_none() {
            current = Some((buffer >> (32 - frame.max_len_bits)) as u16);
            buffer <<= frame.max_len_bits;
            n_bits_in_buff -= frame.max_len_bits as u16;
        }

        if let Some(item) = current.take() {
            if item > 0 {
                items.push(item);
            }
        }

        while n_bits_in_buff >= frame.max_len_bits as u16 {
            let item = (buffer >> (32 - frame.max_len_bits)) as u16;
            buffer <<= frame.max_len_bits;
            n_bits_in_buff -= frame.max_len_bits as u16;
            if item > 0 {
                items.push(item);
            }
        }
    }

    Ok(items)
}

/// Apply decoded TwoDelta run lengths to produce a new assignment vector.
///
/// Positions in `previous_assignment` that hold either value of `pair` are
/// overwritten according to the alternating run-length encoding.
///
/// # Arguments
///
/// * `assignment` - The assignment from the preceding frame (mutated in place).
/// * `pair` - The two label values that participate in the delta.
/// * `run_lengths` - Alternating run lengths starting with the first value of `pair`.
///
/// # Returns
///
/// Returns the updated assignment vector.
fn apply_twodelta_runs_to_assignment(
    mut assignment: Vec<u16>,
    pair: (u16, u16),
    run_lengths: &[u16],
) -> io::Result<Vec<u16>> {
    let (first, second) = pair;

    let mut run_idx = 0usize;
    let mut remaining_in_run: u16 = *run_lengths.first().unwrap_or(&0);
    let mut current_value = first;

    for val in assignment.iter_mut() {
        if *val == first || *val == second {
            if remaining_in_run == 0 {
                run_idx += 1;
                if run_idx >= run_lengths.len() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "TwoDelta payload exhausted before all pair positions were covered",
                    ));
                }
                remaining_in_run = run_lengths[run_idx];
                current_value = if current_value == first {
                    second
                } else {
                    first
                };
            }
            *val = current_value;
            remaining_in_run -= 1;
        }
    }

    Ok(assignment)
}

/// Decode a raw TwoDelta frame into a full assignment vector.
///
/// Unpacks the bitpacked run lengths from the frame payload, then applies
/// them in a single pass over the assignment.
///
/// # Arguments
///
/// * `assignment` - The assignment from the preceding frame (mutated in place).
/// * `frame` - The TwoDelta frame whose packed payload is decoded and applied.
///
/// # Returns
///
/// Returns the updated assignment vector.
fn decode_twodelta_frame_to_assignment(
    assignment: Vec<u16>,
    frame: &TwoDeltaFrame,
) -> io::Result<Vec<u16>> {
    let run_lengths = decode_twodelta_run_lengths(frame)?;
    apply_twodelta_runs_to_assignment(assignment, frame.pair, &run_lengths)
}

/// Decode a stored BEN frame into a full assignment vector.
///
/// # Arguments
///
/// * `previous_assignment` - The assignment from the preceding frame, required
///   for TwoDelta frames.
/// * `frame` - The stored frame to decode.
///
/// # Returns
///
/// Returns the expanded assignment vector.
fn decode_stored_frame_to_assignment(
    previous_assignment: &mut Option<Vec<u16>>,
    frame: &StoredBenFrame,
) -> io::Result<Vec<u16>> {
    match frame {
        StoredBenFrame::Ben(frame) => decode_ben_frame_to_assignment(frame),
        StoredBenFrame::TwoDelta { frame, .. } => decode_twodelta_frame_to_assignment(
            previous_assignment.take().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "TwoDelta frame encountered before an initial BEN frame",
                )
            })?,
            frame,
        ),
    }
}

impl<R: Read> Iterator for BenDecoder<R> {
    type Item = io::Result<MkvRecord>;

    /// Decode and return the next assignment from the BEN stream.
    fn next(&mut self) -> Option<io::Result<MkvRecord>> {
        let frame = match self.pop_frame_from_reader() {
            Some(Ok(frame)) => frame,
            Some(Err(e)) => return Some(Err(e)),
            None => return None,
        };
        let assignment =
            match decode_stored_frame_to_assignment(&mut self.previous_assignment, &frame) {
                Ok(assgn) => assgn,
                Err(e) => return Some(Err(e)),
            };
        let count = frame.count();
        self.previous_assignment = Some(assignment.clone());
        self.sample_count += count as usize;
        if !self.silent {
            progress!("Decoding sample: {}\r", self.sample_count);
        }
        Some(Ok((assignment, count)))
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
        match self.inner.variant {
            BenVariant::Standard | BenVariant::MkvChain => match self.inner.pop_frame_from_reader()
            {
                Some(Ok(StoredBenFrame::Ben(frame))) => Some(Ok(frame)),
                Some(Ok(StoredBenFrame::TwoDelta { .. })) => Some(Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unexpected TwoDelta frame in non-TwoDelta BEN stream",
                ))),
                Some(Err(err)) => Some(Err(err)),
                None => None,
            },
            BenVariant::TwoDelta => match self.inner.next() {
                Some(Ok((assignment, count))) => {
                    let encoded = encode_ben_vec_from_assign(&assignment);
                    let raw_data = encoded.as_slice()[6..].to_vec();
                    Some(Ok(BenFrame {
                        max_val_bits: encoded.max_val_bits,
                        max_len_bits: encoded.max_len_bits,
                        count,
                        n_bytes: encoded.n_bytes,
                        raw_data,
                    }))
                }
                Some(Err(err)) => Some(Err(err)),
                None => None,
            },
        }
    }
}

/// Iterator over decoded assignments in an XBEN stream.
pub struct XBenDecoder<R: Read> {
    xz: BufReader<XzDecoder<R>>,
    /// Variant encoded in the XBEN banner.
    pub variant: BenVariant,
    overflow: Vec<u8>,
    buf: Box<[u8]>,
    previous_assignment: Option<Vec<u16>>,
    chunk_queue: std::collections::VecDeque<(XBenTwoDeltaFrame, u16)>,
}

impl<R: Read> XBenDecoder<R> {
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
            variant,
            overflow: Vec::with_capacity(1 << 20),
            buf: vec![0u8; 1 << 20].into_boxed_slice(),
            previous_assignment: None,
            chunk_queue: std::collections::VecDeque::new(),
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
    pub fn new(reader: R) -> io::Result<Self> {
        let xz = XzDecoder::new(reader);
        let mut xz = BufReader::with_capacity(1 << 20, xz);

        let mut first = [0u8; BANNER_LEN];
        xz.read_exact(&mut first)?;
        let variant = variant_from_banner(&first).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid .xben header (expecting STANDARD/MKVCHAIN/TWODELTA BEN FILE)",
            )
        })?;

        Ok(Self::from_decompressed_stream(xz, variant))
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
            _ => Some(Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid TwoDelta XBEN frame tag",
            ))),
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
        let header_len = 5;
        let pairs_len = n_frames * 4;
        let counts_len = n_frames * 2;
        let run_counts_len = n_frames * 4;
        let fixed_len = header_len + pairs_len + counts_len + run_counts_len;

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
            total_runs += rc;
        }

        let run_data_len = total_runs * 2;
        let total_len = fixed_len + run_data_len;
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
            match self.variant {
                BenVariant::Standard | BenVariant::MkvChain => {
                    if let Some((frame_bytes, consumed, count)) =
                        self.pop_frame_from_overflow(&self.overflow)
                    {
                        let res = match decode_xben_frame_to_assignment(frame_bytes, self.variant) {
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
                        let assignment = match frame {
                            XBenTwoDeltaFrame::Full { runs } => Ok(rle_to_vec(runs)),
                            XBenTwoDeltaFrame::Delta { pair, run_lengths } => {
                                match self.previous_assignment.take() {
                                    Some(prev) => {
                                        apply_twodelta_runs_to_assignment(prev, pair, &run_lengths)
                                    }
                                    None => Err(io::Error::new(
                                        io::ErrorKind::InvalidData,
                                        "TwoDelta XBEN frame encountered before an initial BEN frame",
                                    )),
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
                                            None => Err(io::Error::new(
                                                io::ErrorKind::InvalidData,
                                                "TwoDelta XBEN frame encountered before an initial BEN frame",
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
        if self.inner.variant == BenVariant::TwoDelta {
            return self.inner.next().map(|result| {
                result.and_then(|(assignment, count)| {
                    Ok((encode_ben32_assignments(&assignment)?.into_u8_vec()?, count))
                })
            });
        }

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
