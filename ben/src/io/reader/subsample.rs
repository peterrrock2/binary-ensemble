use super::stream_reader::{BenStreamFrameReader, BenWireFormat};
use crate::codec::decode::decode_ben32_line;
use crate::codec::BenDecodeFrame;
use crate::BenVariant;
use std::fs::File;
use std::io::{self, BufReader, Cursor, Read};
use std::iter::Peekable;
use std::path::{Path, PathBuf};

/// A decoded assignment together with the number of times it repeats.
pub type MkvRecord = (Vec<u16>, u16);
/// A raw ben32 frame together with the number of times it repeats.
pub type Ben32Frame = (Vec<u8>, u16);
/// A boxed iterator over generic BEN/XBEN frames used by subsampling helpers.
pub type FrameIter = Box<dyn Iterator<Item = io::Result<(DecodeFrame, u16)>> + Send>;

#[derive(Clone, Debug)]
/// A generalized frame type used by the subsampling machinery.
pub enum DecodeFrame {
    /// A raw BEN frame.
    Ben(BenDecodeFrame),
    /// A raw ben32 frame from an XBEN stream together with its variant.
    XBen(Vec<u8>, BenVariant),
}

impl DecodeFrame {
    /// Expand a self-contained subsample frame into an assignment vector.
    ///
    /// Distinct from [`BenDecodeFrame::expand`] (which takes a previous assignment for delta
    /// variants); the frame readers guarantee frames reaching the subsample path are
    /// self-contained, so no `prev` is needed: plain-BEN TwoDelta is materialized and re-encoded as
    /// `Standard`, and XBEN TwoDelta is materialized and re-encoded as ben32.
    pub fn expand_self_contained(&self) -> io::Result<Vec<u16>> {
        match self {
            DecodeFrame::Ben(f) => f.expand(None),
            DecodeFrame::XBen(bytes, variant) => {
                let (assignment, _) = decode_ben32_line(Cursor::new(bytes), *variant)?;
                Ok(assignment)
            }
        }
    }
}

/// A selection strategy for extracting only part of a frame stream.
pub enum Selection {
    /// Select explicit zero-based indices.
    Indices(Peekable<std::vec::IntoIter<usize>>),
    /// Select every `step` samples starting at the zero-based `offset`.
    Every { step: usize, offset: usize },
    /// Select the half-open zero-based range `[start, end)`.
    Range { start: usize, end: usize },
}

/// Iterator adaptor that decodes only selected samples from a frame stream.
pub struct SubsampleFrameDecoder<I>
where
    I: Iterator<Item = io::Result<(DecodeFrame, u16)>>,
{
    inner: I,
    selection: Selection,
    next_index: usize,
}

impl<I> SubsampleFrameDecoder<I>
where
    I: Iterator<Item = io::Result<(DecodeFrame, u16)>>,
{
    /// Create a subsampling iterator from a lower-level frame iterator.
    pub fn new(inner: I, selection: Selection) -> Self {
        Self {
            inner,
            selection,
            next_index: 0,
        }
    }

    /// Select a set of zero-based sample indices.
    ///
    /// Indices are sorted and deduplicated before iteration begins.
    pub fn by_indices<T>(inner: I, indices: T) -> Self
    where
        T: IntoIterator<Item = usize>,
    {
        let mut v: Vec<usize> = indices.into_iter().collect();
        v.sort_unstable();
        v.dedup();
        Self::new(inner, Selection::Indices(v.into_iter().peekable()))
    }

    /// Select the half-open zero-based range `[start, end)`.
    pub fn by_range(inner: I, start: usize, end: usize) -> Self {
        assert!(end >= start, "range end must be >= start");
        Self::new(inner, Selection::Range { start, end })
    }

    /// Select every `step` samples beginning from the zero-based `offset`.
    pub fn every(inner: I, step: usize, offset: usize) -> Self {
        assert!(step >= 1, "step must be >= 1");
        Self::new(inner, Selection::Every { step, offset })
    }

    /// Count how many selected samples fall within a half-open sample interval.
    fn count_selected_in(&mut self, lo: usize, hi: usize) -> u16 {
        match &mut self.selection {
            Selection::Indices(iter) => {
                let mut taken = 0u16;
                while let Some(&next) = iter.peek() {
                    if next < lo {
                        iter.next();
                        continue;
                    }
                    if next >= hi {
                        break;
                    }
                    iter.next();
                    taken = taken.saturating_add(1);
                }
                taken
            }
            Selection::Every { step, offset } => {
                let start = lo.max(*offset);
                let remainder = (start - *offset) % *step;
                let first = start + ((*step - remainder) % *step);
                if first >= hi {
                    0
                } else {
                    (1 + (hi - 1 - first) / *step) as u16
                }
            }
            Selection::Range { start, end } => {
                let a = lo.max(*start);
                let b = hi.min(*end);
                if a >= b {
                    0
                } else {
                    (b - a) as u16
                }
            }
        }
    }
}

impl<I> Iterator for SubsampleFrameDecoder<I>
where
    I: Iterator<Item = io::Result<(DecodeFrame, u16)>>,
{
    type Item = io::Result<MkvRecord>;

    /// Return the next decoded sample selected by the subsampling rule.
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Selection::Range { end, .. } = self.selection {
                if self.next_index >= end {
                    return None;
                }
            }
            if let Selection::Indices(ref mut it) = self.selection {
                it.peek()?;
            }

            let (frame, count) = match self.inner.next()? {
                Ok(x) => x,
                Err(e) => return Some(Err(e)),
            };
            if count == 0 {
                return Some(Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "frame count must be greater than zero",
                )));
            }

            let lo = self.next_index;
            let hi = self.next_index + count as usize;
            let selected = self.count_selected_in(lo, hi);

            self.next_index = hi;

            if selected > 0 {
                match frame.expand_self_contained() {
                    Ok(assignment) => return Some(Ok((assignment, selected))),
                    Err(e) => return Some(Err(e)),
                }
            }
        }
    }
}

/// Build a generic frame iterator from a BEN or XBEN file path.
///
/// Frame iteration is useful for subsampling and counting because it avoids decoding every sample
/// into a full assignment vector.
pub fn build_frame_iter(file_path: &PathBuf, format: BenWireFormat) -> io::Result<FrameIter> {
    let file = File::options().read(true).open(file_path)?;
    let reader = BufReader::new(file);
    build_frame_iter_from_reader(reader, format)
}

/// Build a generic frame iterator from an already-opened reader.
///
/// This is the reader-driven variant of [`build_frame_iter`], useful when the caller needs to
/// iterate frames over a sub-region of a file (e.g. the assignment stream embedded in a `.bendl`
/// bundle, wrapped in a bounded-length guard) without re-opening the file from offset zero.
pub fn build_frame_iter_from_reader<R: Read + Send + 'static>(
    reader: R,
    format: BenWireFormat,
) -> io::Result<FrameIter> {
    match format {
        BenWireFormat::Ben => {
            let frames = BenStreamFrameReader::from_ben(reader).map_err(io::Error::from)?;
            Ok(Box::new(frames))
        }
        BenWireFormat::XBen => {
            let frames = BenStreamFrameReader::from_xben(reader).map_err(io::Error::from)?;
            Ok(Box::new(frames))
        }
    }
}

/// Count the number of samples in a BEN or XBEN file on disk.
///
/// The file is walked frame-by-frame, so this is linear in file size but avoids materializing full
/// assignment vectors.
pub fn count_samples_from_file(path: &Path, format: BenWireFormat) -> io::Result<usize> {
    let iter = build_frame_iter(&path.to_path_buf(), format)?;
    count_samples_from_frame_iter(iter)
}

/// Count the number of samples reachable through a pre-built frame iterator.
///
/// Mirror of [`count_samples_from_file`] that operates on an existing [`FrameIter`], so callers
/// that already have one (e.g. constructed via [`build_frame_iter_from_reader`] over a bundle's
/// stream region) can reuse the walking logic without re-opening any files.
pub fn count_samples_from_frame_iter(iter: FrameIter) -> io::Result<usize> {
    let mut total = 0usize;
    for item in iter {
        let (_frame, cnt) = item?;
        total += cnt as usize;
    }
    Ok(total)
}
