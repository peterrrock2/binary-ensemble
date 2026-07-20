//! Unified reader for the BEN-stack stream layer (layer 3, see `docs/glossary.md`).
//!
//! Hides the wire-format choice (BEN bit-packed vs ben32 columnar) and the transport choice (plain
//! vs xz-compressed) behind one type. The decode-side laziness invariant is preserved on both wire
//! formats: frame payload bytes stay opaque until [`crate::codec::BenDecodeFrame::expand`]
//! (frame-level decode), not to be confused with
//! [`crate::io::reader::DecodeFrame::expand_self_contained`] (subsample-level).

mod ben;
mod events;
mod frames;
mod xben;

use std::collections::VecDeque;
use std::io::{self, BufReader, Read, Write};

use serde_json::json;
use xz2::read::XzDecoder;

use super::errors::DecoderInitError;
use super::subsample::{MkvRecord, SubsampleFrameDecoder};
use crate::codec::decode::TwoDeltaMaskIndex;
use crate::format::banners::{variant_from_banner, BANNER_LEN};
use crate::progress::Spinner;
use crate::BenVariant;

pub use events::{TwoDeltaFrameEvent, TwoDeltaFrameEventReader};
pub use frames::BenStreamFrameReader;

/// Wire format of a BEN-stack stream.
///
/// The Rust representation of the BEN/XBEN stream choice. This is the seam the public reader API
/// uses to dispatch on wire format; the bundle layer owns its own conversion from
/// `AssignmentFormat`.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum BenWireFormat {
    /// Plain BEN bit-packed wire format over an unwrapped byte stream.
    Ben,
    /// BEN32 columnar / TwoDelta wire format over an xz-compressed byte stream.
    XBen,
}

/// Reader for an encoded BEN-stack stream of samples.
///
/// Construct with [`BenStreamReader::from_ben`] or [`BenStreamReader::from_xben`]. Both arms expose
/// the same downstream surface for assignment iteration, JSONL writing, sample counting, and
/// subsampling.
pub struct BenStreamReader<R: Read> {
    inner: BenStreamInner<R>,
    variant: BenVariant,
    silent: bool,
}

/// Wire-format split: the `Ben` arm carries inline state, the `XBen` arm is boxed so the enum's
/// static size stays close to the smaller plain-BEN footprint instead of being dictated by the
/// larger xz state.
pub(crate) enum BenStreamInner<R: Read> {
    Ben {
        reader: R,
        previous_assignment: Option<Vec<u16>>,
        twodelta_masks: Option<TwoDeltaMaskIndex>,
        sample_count: usize,
        spinner: Option<Spinner>,
    },
    XBen(Box<XBenInner<R>>),
}

/// Decompressed-stream state for the `XBen` arm.
pub(crate) struct XBenInner<R: Read> {
    pub(crate) xz: BufReader<XzDecoder<R>>,
    pub(crate) overflow: Vec<u8>,
    pub(crate) buf: Box<[u8]>,
    pub(crate) previous_assignment: Option<Vec<u16>>,
    pub(crate) chunk_queue: VecDeque<((u16, u16), Vec<u16>, u16)>,
}

pub(super) fn zero_count_frame_error(label: &'static str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("{label} frame count must be greater than zero"),
    )
}

impl<R: Read> BenStreamReader<R> {
    /// Open a plain BEN stream. The reader must begin with a 17-byte BEN banner.
    pub fn from_ben(mut reader: R) -> Result<Self, DecoderInitError> {
        let mut check_buffer = [0u8; BANNER_LEN];
        if let Err(e) = reader.read_exact(&mut check_buffer) {
            return Err(DecoderInitError::Io(e));
        }
        let variant = variant_from_banner(&check_buffer)
            .ok_or_else(|| DecoderInitError::InvalidFileFormat(check_buffer.to_vec()))?;
        Ok(Self {
            inner: BenStreamInner::Ben {
                reader,
                previous_assignment: None,
                twodelta_masks: None,
                sample_count: 0,
                spinner: None,
            },
            variant,
            silent: false,
        })
    }

    /// Open an XBEN stream. The reader must produce, after xz decompression, a 17-byte BEN banner
    /// followed by ben32 columnar frames.
    pub fn from_xben(reader: R) -> Result<Self, DecoderInitError> {
        let xz = XzDecoder::new(reader);
        let mut xz = BufReader::with_capacity(1 << 20, xz);

        let mut first = [0u8; BANNER_LEN];
        if let Err(e) = xz.read_exact(&mut first) {
            return Err(DecoderInitError::Io(e));
        }
        let variant = variant_from_banner(&first)
            .ok_or_else(|| DecoderInitError::InvalidFileFormat(first.to_vec()))?;

        Ok(Self::from_xben_decompressed(xz, variant))
    }

    /// Build from a decompressed XBEN stream already positioned past the 17-byte BEN banner.
    pub(crate) fn from_xben_decompressed(xz: BufReader<XzDecoder<R>>, variant: BenVariant) -> Self {
        Self {
            inner: BenStreamInner::XBen(Box::new(XBenInner {
                xz,
                overflow: Vec::with_capacity(1 << 20),
                buf: vec![0u8; 1 << 20].into_boxed_slice(),
                previous_assignment: None,
                chunk_queue: VecDeque::new(),
            })),
            variant,
            silent: false,
        }
    }

    /// Return the BEN variant detected from the stream banner.
    pub fn variant(&self) -> BenVariant {
        self.variant
    }

    /// Return the wire format (BEN vs XBEN) of this stream.
    pub fn wire_format(&self) -> BenWireFormat {
        match &self.inner {
            BenStreamInner::Ben { .. } => BenWireFormat::Ben,
            BenStreamInner::XBen(_) => BenWireFormat::XBen,
        }
    }

    /// Suppress progress output from this decoder's iteration paths.
    ///
    /// In the `Ben` arm, this clears any active spinner. In the `XBen` arm, `for_each_assignment`
    /// consults `silent` before creating its local spinner.
    pub fn silent(mut self, silent: bool) -> Self {
        self.silent = silent;
        if let BenStreamInner::Ben { spinner, .. } = &mut self.inner {
            if silent {
                *spinner = None;
            }
        }
        self
    }

    /// Whether this reader is in silent mode.
    pub(crate) fn is_silent(&self) -> bool {
        self.silent
    }

    /// Return a mutable reference to the inner stream state.
    pub(crate) fn inner_mut(&mut self) -> &mut BenStreamInner<R> {
        &mut self.inner
    }

    /// Consume this decoder and iterate over raw BEN/ben32 frames instead of materialized
    /// assignments.
    pub fn into_frames(self) -> BenStreamFrameReader<R> {
        BenStreamFrameReader::from_stream(self)
    }

    /// Count the number of samples remaining in the stream.
    ///
    /// Walks frame boundaries rather than expanding every assignment.
    pub fn count_samples(self) -> io::Result<usize> {
        let variant = self.variant;
        match self.inner {
            BenStreamInner::Ben { reader, .. } => ben::count_samples_ben(reader, variant),
            BenStreamInner::XBen(inner) => xben::count_samples_xben(*inner, variant),
        }
    }

    /// Decode assignments and pass each one to a callback by reference.
    ///
    /// Unlike [`Iterator`], this avoids cloning the assignment buffer on every frame. The callback
    /// receives a borrowed slice and its repetition count. Return `true` to continue or `false` to
    /// stop early.
    pub fn for_each_assignment<F>(&mut self, f: F) -> io::Result<()>
    where
        F: FnMut(&[u16], u16) -> io::Result<bool>,
    {
        let silent = self.silent;
        let variant = self.variant;
        match &mut self.inner {
            BenStreamInner::Ben {
                reader,
                previous_assignment,
                twodelta_masks,
                sample_count,
                spinner,
            } => ben::for_each_assignment_ben(
                reader,
                variant,
                previous_assignment,
                twodelta_masks,
                sample_count,
                spinner,
                silent,
                f,
            ),
            BenStreamInner::XBen(inner) => {
                xben::for_each_assignment_xben(inner, variant, silent, f)
            }
        }
    }

    /// Decode the remaining stream and write it as JSONL.
    ///
    /// Each decoded sample is written as a JSON object containing an `assignment` vector and a
    /// one-based `sample` number.
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

impl<R: Read> Iterator for BenStreamReader<R> {
    type Item = io::Result<MkvRecord>;

    fn next(&mut self) -> Option<Self::Item> {
        let silent = self.silent;
        let variant = self.variant;
        match &mut self.inner {
            BenStreamInner::Ben {
                reader,
                previous_assignment,
                twodelta_masks,
                sample_count,
                spinner,
            } => ben::next_record_ben(
                reader,
                variant,
                previous_assignment,
                twodelta_masks,
                sample_count,
                spinner,
                silent,
            ),
            BenStreamInner::XBen(inner) => xben::next_record_xben(inner, variant),
        }
    }
}

impl<R: Read + Send> BenStreamReader<R> {
    /// Convert this decoder into a subsampling iterator over explicit zero-based indices.
    pub fn into_subsample_by_indices<T>(
        self,
        indices: T,
    ) -> SubsampleFrameDecoder<BenStreamFrameReader<R>>
    where
        T: IntoIterator<Item = usize>,
    {
        SubsampleFrameDecoder::by_indices(self.into_frames(), indices)
    }

    /// Convert this decoder into a subsampling iterator over the half-open zero-based range
    /// `[start, end)`.
    pub fn into_subsample_by_range(
        self,
        start: usize,
        end: usize,
    ) -> SubsampleFrameDecoder<BenStreamFrameReader<R>> {
        SubsampleFrameDecoder::by_range(self.into_frames(), start, end)
    }

    /// Convert this decoder into a subsampling iterator that selects every `step` samples from the
    /// zero-based `offset`.
    pub fn into_subsample_every(
        self,
        step: usize,
        offset: usize,
    ) -> SubsampleFrameDecoder<BenStreamFrameReader<R>> {
        SubsampleFrameDecoder::every(self.into_frames(), step, offset)
    }
}
