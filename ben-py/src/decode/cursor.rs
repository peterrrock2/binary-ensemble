//! Shared iteration and subsampling core for the stream and bundle decoders.
//!
//! [`SampleCursor`] owns everything needed to walk an assignment stream and to apply a subsample
//! selection, independent of whether the bytes come from a plain `.ben`/`.xben` file or from a
//! `.bendl` file's embedded stream region. Both `PyBenDecoder` and `PyBendlDecoder` embed one and
//! forward their iteration / `len` / `subsample_*` methods to it, so the single-pass restart logic,
//! the `MkvRecord` run expansion, and the subsample bounds checks cannot drift between the two.

use super::helpers::{build_frames_for_subsample, build_iter, scan_samples};
use super::types::{ActiveSelection, DecoderMode, DynIter, StreamSource};
use binary_ensemble::io::reader::{Selection, SubsampleFrameDecoder};
use pyo3::exceptions::{PyException, PyUserWarning};
use pyo3::prelude::*;
use pyo3::types::PyDict;

/// Iteration state shared by the stream and bundle decoders.
pub(super) struct SampleCursor {
    source: StreamSource,
    mode: DecoderMode,
    /// Lazily-constructed frame iterator. Construction is deferred so opening a bundle with an
    /// empty or truncated stream still succeeds — only methods that actually walk the stream
    /// need a live iterator.
    iter: Option<DynIter>,
    current_assignment: Option<Vec<u16>>,
    remaining_count: u16,
    base_len: Option<usize>,
    len_hint: Option<usize>,
    active_selection: ActiveSelection,
}

impl SampleCursor {
    pub(super) fn new(source: StreamSource, mode: DecoderMode) -> Self {
        Self {
            source,
            mode,
            iter: None,
            current_assignment: None,
            remaining_count: 0,
            base_len: None,
            len_hint: None,
            active_selection: ActiveSelection::None,
        }
    }

    pub(super) fn mode(&self) -> DecoderMode {
        self.mode
    }

    /// Eagerly construct the iterator now, surfacing a malformed-banner error at open time. Used by
    /// the plain-stream decoder, which can only learn the variant by opening the reader.
    pub(super) fn prime_iter(&mut self) -> PyResult<()> {
        self.iter = Some(build_iter(&self.source, self.mode)?);
        Ok(())
    }

    /// Reset and rebuild the iterator from the start, reapplying any active subsample selection.
    pub(super) fn restart(&mut self) -> PyResult<()> {
        self.current_assignment = None;
        self.remaining_count = 0;

        let new_iter: DynIter = match self.active_selection.clone() {
            ActiveSelection::None => build_iter(&self.source, self.mode)?,
            sel => {
                let frames = build_frames_for_subsample(&self.source, self.mode)?;
                let ben_sel = sel
                    .to_selection()
                    .expect("active subsample selection must be convertible");
                Box::new(SubsampleFrameDecoder::new(frames, ben_sel))
            }
        };
        self.iter = Some(new_iter);
        Ok(())
    }

    pub(super) fn next(&mut self) -> PyResult<Option<Vec<u16>>> {
        if self.remaining_count > 0 {
            self.remaining_count -= 1;
            let a = self.current_assignment.as_ref().unwrap().clone();
            return Ok(Some(a));
        }
        // Build the iterator on first use (e.g. when iteration begins without an explicit
        // `__iter__` call). For bundle sources with empty/truncated streams this is where a
        // BEN-banner-required error surfaces, instead of at decoder construction.
        if self.iter.is_none() {
            self.iter = Some(build_iter(&self.source, self.mode)?);
        }
        let next = self
            .iter
            .as_mut()
            .expect("iter populated by the lazy-init branch above")
            .next();
        match next {
            Some(Ok((assignment, count))) => {
                if count == 0 {
                    return Err(PyException::new_err(
                        "Decoder yielded a zero-count record; data may be corrupted.",
                    ));
                }
                self.current_assignment = Some(assignment.clone());
                self.remaining_count = count - 1;
                Ok(Some(assignment))
            }
            Some(Err(e)) => Err(PyException::new_err(format!(
                "Error decoding next item: {e}"
            ))),
            None => Ok(None),
        }
    }

    /// Report the number of samples `len(dec)` should return: the filtered count when a subsample
    /// selection is active, otherwise the base count.
    pub(super) fn len(&mut self, py: Python<'_>) -> PyResult<usize> {
        if let Some(len_hint) = self.len_hint {
            return Ok(len_hint);
        }
        let base_len = self.ensure_base_len(py)?;
        self.len_hint = Some(base_len);
        Ok(base_len)
    }

    /// Always report the base (unfiltered) sample count, even after `subsample_*` has been applied.
    /// Deliberately does not touch `len_hint`, which tracks the filtered count for `__len__`.
    pub(super) fn count_samples(&mut self, py: Python<'_>) -> PyResult<usize> {
        self.ensure_base_len(py)
    }

    pub(super) fn subsample_indices(
        &mut self,
        mut indices: Vec<usize>,
        py: Python<'_>,
    ) -> PyResult<()> {
        if !indices.iter().is_sorted() {
            // We need to sort and deduplicate the indices. This is necessary so we can efficiently
            // iterate over the underlying data. Unstable sort is fine because we do not care about
            // the order of equal elements.
            let warnings = py.import("warnings")?;
            let kwargs = PyDict::new(py);
            warnings.call_method(
                "warn",
                (
                    "Indices must be sorted and unique; sorting and deduplicating.",
                    py.get_type::<PyUserWarning>(),
                ),
                Some(&kwargs),
            )?;
        }
        indices.sort_unstable();
        indices.dedup();

        if indices.is_empty() {
            return Err(PyException::new_err("indices must not be empty"));
        }
        let base_len = self.ensure_base_len(py)?;
        if indices[0] == 0 {
            return Err(PyException::new_err("indices must be 1-based"));
        }
        if indices.last().unwrap() > &base_len {
            return Err(PyException::new_err(format!(
                "indices must be <= number of samples in base data ({base_len})"
            )));
        }
        let len_hint = indices.len();
        self.active_selection = ActiveSelection::Indices(indices.clone());
        let sel = Selection::Indices(indices.into_iter().peekable());
        self.reset_with_selection(sel, len_hint)
    }

    pub(super) fn subsample_range(
        &mut self,
        start: usize,
        end: usize,
        py: Python<'_>,
    ) -> PyResult<()> {
        if start == 0 || end < start {
            return Err(PyException::new_err(
                "range must be 1-based and end >= start",
            ));
        }
        let base_len = self.ensure_base_len(py)?;
        if end > base_len {
            return Err(PyException::new_err(format!(
                "end must be <= number of samples in base data ({base_len})"
            )));
        }
        self.active_selection = ActiveSelection::Range { start, end };
        let sel = Selection::Range { start, end };
        let len_hint = end - start + 1;
        self.reset_with_selection(sel, len_hint)
    }

    pub(super) fn subsample_every(
        &mut self,
        step: usize,
        offset: usize,
        py: Python<'_>,
    ) -> PyResult<()> {
        if step == 0 || offset == 0 {
            return Err(PyException::new_err("step and offset must be >= 1"));
        }
        let base_len = self.ensure_base_len(py)?;
        if offset > base_len {
            return Err(PyException::new_err(format!(
                "offset must be <= number of samples in base data ({base_len})"
            )));
        }
        self.active_selection = ActiveSelection::Every { step, offset };
        let sel = Selection::Every { step, offset };
        let len_hint = (base_len + step - 1 - (offset - 1)) / step;
        self.reset_with_selection(sel, len_hint)
    }

    fn reset_with_selection(&mut self, selection: Selection, len_hint: usize) -> PyResult<()> {
        let frames = build_frames_for_subsample(&self.source, self.mode)?;
        let frame_decoder = SubsampleFrameDecoder::new(frames, selection);
        self.iter = Some(Box::new(frame_decoder));
        self.current_assignment = None;
        self.remaining_count = 0;
        self.len_hint = Some(len_hint);
        Ok(())
    }

    fn ensure_base_len(&mut self, py: Python<'_>) -> PyResult<usize> {
        if let Some(base_len) = self.base_len {
            return Ok(base_len);
        }
        let base_len = match &self.source {
            StreamSource::Bundle { empty: true, .. } => 0,
            StreamSource::Bundle {
                header_sample_count: Some(n),
                ..
            } if *n >= 0 => *n as usize,
            _ => scan_samples(&self.source, self.mode, py)?,
        };
        self.base_len = Some(base_len);
        Ok(base_len)
    }
}
