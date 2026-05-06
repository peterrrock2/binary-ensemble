use super::assignment_reader::AssignmentFrameReader;
use super::errors::DecoderInitError;
use super::xz_assignment_reader::decode_xben_frame_to_assignment;
use super::xz_assignment_reader::XZAssignmentReader;
use crate::codec::BenDecodeFrame;
use crate::BenVariant;
use std::fs::File;
use std::io::{self, BufReader, Read};
use std::iter::Peekable;
use std::path::{Path, PathBuf};

/// A decoded assignment together with the number of times it repeats.
pub type MkvRecord = (Vec<u16>, u16);
/// A raw ben32 frame together with the number of times it repeats.
pub type Ben32Frame = (Vec<u8>, u16);
/// A boxed iterator over generic BEN/XBEN frames used by subsampling helpers.
pub type FrameIter = Box<dyn Iterator<Item = io::Result<(DecodeFrame, u16)>> + Send>;

#[derive(Clone)]
/// A generalized frame type used by the subsampling machinery.
pub enum DecodeFrame {
    /// A raw BEN frame.
    Ben(BenDecodeFrame),
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
///
/// `AssignmentFrameReader` rewrites TwoDelta BEN frames into self-contained
/// Standard frames before they reach this path, so `Ben(...)` is always a
/// `Standard` or `MkvChain` arm and `expand(None)` is always sufficient here.
pub(super) fn decode_frame_to_assignment(frame: &DecodeFrame) -> io::Result<Vec<u16>> {
    match frame {
        DecodeFrame::Ben(f) => f.expand(None),
        DecodeFrame::XBen(bytes, variant) => decode_xben_frame_to_assignment(bytes, *variant),
    }
}

/// Iterator adaptor that decodes only selected samples from a frame stream.
pub struct SubsampleFrameDecoder<I>
where
    I: Iterator<Item = io::Result<(DecodeFrame, u16)>>,
{
    inner: I,
    selection: Selection,
    sample: usize,
}

impl<I> SubsampleFrameDecoder<I>
where
    I: Iterator<Item = io::Result<(DecodeFrame, u16)>>,
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
    I: Iterator<Item = io::Result<(DecodeFrame, u16)>>,
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
            if count == 0 {
                return Some(Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "frame count must be greater than zero",
                )));
            }

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
    build_frame_iter_from_reader(reader, mode)
}

/// Build a generic frame iterator from an already-opened reader.
///
/// This is the reader-driven variant of [`build_frame_iter`], useful when
/// the caller needs to iterate frames over a sub-region of a file (e.g.
/// the assignment stream embedded in a `.bendl` bundle, wrapped in a
/// [`std::io::Read::take`] guard) without re-opening the file from offset
/// zero.
///
/// # Arguments
///
/// * `reader` - Any owned reader positioned at the start of a `.ben` or
///   `.xben` byte stream.
/// * `mode` - Either `"ben"` or `"xben"`.
///
/// # Returns
///
/// Returns a boxed iterator over generic frames and their repetition counts.
pub fn build_frame_iter_from_reader<R: Read + Send + 'static>(
    reader: R,
    mode: &str,
) -> io::Result<FrameIter> {
    match mode {
        "ben" => {
            let frames = AssignmentFrameReader::new(reader)?;
            let mapped = frames.map(|res| res.map(|(f, cnt)| (DecodeFrame::Ben(f), cnt)));
            Ok(Box::new(mapped))
        }
        "xben" => {
            let x = XZAssignmentReader::new(reader)?;
            let variant = x.variant();
            let frames = x.into_frames();
            let mapped = frames
                .map(move |res| res.map(|(bytes, cnt)| (DecodeFrame::XBen(bytes, variant), cnt)));
            Ok(Box::new(mapped))
        }
        _ => Err(io::Error::from(DecoderInitError::UnknownMode {
            mode: mode.to_string(),
        })),
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
    count_samples_from_frame_iter(iter)
}

/// Count the number of samples reachable through a pre-built frame iterator.
///
/// Mirror of [`count_samples_from_file`] that operates on an existing
/// [`FrameIter`], so callers that already have one (e.g. constructed via
/// [`build_frame_iter_from_reader`] over a bundle's stream region) can
/// reuse the walking logic without re-opening any files.
pub fn count_samples_from_frame_iter(iter: FrameIter) -> io::Result<usize> {
    let mut total = 0usize;
    for item in iter {
        let (_frame, cnt) = item?;
        total += cnt as usize;
    }
    Ok(total)
}
