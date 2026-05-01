use super::helpers::{
    build_bundle_iter, build_frames_for_subsample, build_plain_iter, detect_is_bundle,
    scan_bundle_samples, warn_xben_startup,
};
use super::types::{ActiveSelection, BundleState, DecoderBackend, DecoderMode, DynIter};
use binary_ensemble::io::bundle::format::{
    ASSET_FLAG_CHECKSUM, ASSET_FLAG_JSON, ASSET_FLAG_XZ, ASSET_TYPE_GRAPH, ASSET_TYPE_METADATA,
    ASSET_TYPE_RELABEL_MAP,
};
use binary_ensemble::io::bundle::BendlReader;
use binary_ensemble::io::reader::{count_samples_from_file, Selection, SubsampleFrameDecoder};
use pyo3::exceptions::{PyException, PyIOError, PyKeyError, PyUserWarning};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Write};
use std::path::PathBuf;

#[pyclass(module = "binary_ensemble", unsendable)]
pub struct PyBenDecoder {
    path: PathBuf,
    mode: DecoderMode,
    backend: DecoderBackend,
    iter: DynIter,
    current_assignment: Option<Vec<u16>>,
    remaining_count: u16,
    base_len: Option<usize>,
    len_hint: Option<usize>,
    active_selection: ActiveSelection,
}

#[pymethods]
impl PyBenDecoder {
    /// Open a decoder on a `.ben`, `.xben`, or `.bendl` file.
    ///
    /// The file's leading bytes are sniffed to decide whether it is a
    /// bundle. When the file is a `.bendl`, the bundle's header decides
    /// the BEN/XBEN format and the `mode` argument is ignored; when the
    /// file is a plain stream, `mode` selects between the BEN and XBEN
    /// readers and defaults to `"ben"`.
    ///
    /// # Arguments
    ///
    /// * `file_path` - Path to the input file.
    /// * `mode` - Either `"ben"` or `"xben"`. Only consulted for plain
    ///   streams; bundles use `assignment_format` from the header.
    #[new]
    #[pyo3(signature = (file_path, mode = "ben"))]
    #[pyo3(text_signature = "(file_path, mode='ben')")]
    fn new(py: Python<'_>, file_path: PathBuf, mode: &str) -> PyResult<Self> {
        // Validate the mode string up front so "Unknown mode" is reported
        // regardless of whether the file exists or turns out to be a bundle.
        let parsed_mode = DecoderMode::parse(mode)?;
        let is_bundle = detect_is_bundle(&file_path).map_err(|e| {
            PyIOError::new_err(format!("Failed to open {}: {e}", file_path.display()))
        })?;

        if is_bundle {
            let file = File::open(&file_path).map_err(|e| {
                PyIOError::new_err(format!("Failed to open {}: {e}", file_path.display()))
            })?;
            let mut reader = BendlReader::open(BufReader::new(file)).map_err(|e| {
                PyException::new_err(format!(
                    "Failed to parse bundle header in {}: {e}",
                    file_path.display()
                ))
            })?;
            let fmt = reader.assignment_format().ok_or_else(|| {
                PyException::new_err(
                    "Bundle header has an unrecognized assignment_format field.",
                )
            })?;
            let derived_mode = DecoderMode::from_assignment_format(fmt);
            let (stream_offset, stream_len) =
                reader.assignment_stream_range().map_err(|e| {
                    PyException::new_err(format!(
                        "Failed to determine stream region in {}: {e}",
                        file_path.display()
                    ))
                })?;
            let state = BundleState {
                reader,
                stream_offset,
                stream_len,
            };

            // Emit the XBEN startup warning once, up front.
            if matches!(derived_mode, DecoderMode::XBen) {
                warn_xben_startup(py)?;
            }

            let iter = build_bundle_iter(&file_path, &state, derived_mode)?;
            Ok(Self {
                path: file_path,
                mode: derived_mode,
                backend: DecoderBackend::Bundle(state),
                iter,
                current_assignment: None,
                remaining_count: 0,
                base_len: None,
                len_hint: None,
                active_selection: ActiveSelection::None,
            })
        } else {
            if matches!(parsed_mode, DecoderMode::XBen) {
                warn_xben_startup(py)?;
            }
            let iter = build_plain_iter(&file_path, parsed_mode)?;
            Ok(Self {
                path: file_path,
                mode: parsed_mode,
                backend: DecoderBackend::Plain,
                iter,
                current_assignment: None,
                remaining_count: 0,
                base_len: None,
                len_hint: None,
                active_selection: ActiveSelection::None,
            })
        }
    }

    /// Return `self` as an iterator, rebuilding the underlying frame
    /// walker so iteration can be restarted.
    ///
    /// Calling `iter(dec)` (or using `for x in dec: …`) more than once
    /// is supported: each call reopens the stream region from the start
    /// and, if a subsample selection is active, reapplies it.
    fn __iter__(mut slf: PyRefMut<Self>) -> PyResult<Py<Self>> {
        slf.current_assignment = None;
        slf.remaining_count = 0;

        let path = slf.path.clone();
        let mode = slf.mode;
        let selection = slf.active_selection.clone();

        let new_iter: DynIter = match selection {
            ActiveSelection::None => match &slf.backend {
                DecoderBackend::Plain => build_plain_iter(&path, mode)?,
                DecoderBackend::Bundle(state) => build_bundle_iter(&path, state, mode)?,
            },
            sel => {
                let frames = build_frames_for_subsample(&path, mode, &slf.backend)?;
                let ben_sel = sel
                    .to_selection()
                    .expect("active subsample selection must be convertible");
                Box::new(SubsampleFrameDecoder::new(frames, ben_sel))
            }
        };

        slf.iter = new_iter;
        Ok(slf.into())
    }

    fn __next__(mut slf: PyRefMut<Self>) -> PyResult<Option<Vec<u16>>> {
        if slf.remaining_count > 0 {
            slf.remaining_count -= 1;
            let a = slf.current_assignment.as_ref().unwrap().clone();
            return Ok(Some(a));
        }
        match slf.iter.next() {
            Some(Ok((assignment, count))) => {
                if count == 0 {
                    return Err(PyException::new_err(
                        "Decoder yielded a zero-count record; data may be corrupted.",
                    ));
                }
                slf.current_assignment = Some(assignment.clone());
                slf.remaining_count = count - 1;
                Ok(Some(assignment))
            }
            Some(Err(e)) => Err(PyException::new_err(format!(
                "Error decoding next item: {e}"
            ))),
            None => Ok(None),
        }
    }

    // Because we want progress bars!!!
    fn __len__(mut slf: PyRefMut<Self>, py: Python<'_>) -> PyResult<usize> {
        if let Some(len_hint) = slf.len_hint {
            return Ok(len_hint);
        }

        let base_len = ensure_base_len(&mut slf, py)?;
        slf.len_hint = Some(base_len);
        Ok(base_len)
    }

    #[pyo3(text_signature = "(self)")]
    fn count_samples(mut slf: PyRefMut<Self>, py: Python<'_>) -> PyResult<usize> {
        // Always reports the total number of samples in the source file,
        // even after `subsample_*` has been applied. We deliberately do
        // not touch `len_hint` here: when a subsample selection is
        // active, `len_hint` tracks the filtered count that `__len__`
        // should return, and clobbering it would break `len(dec)` after
        // a `count_samples()` call.
        ensure_base_len(&mut slf, py)
    }

    #[pyo3(text_signature = "(self, indices, /)")]
    fn subsample_indices<'py>(
        mut slf: PyRefMut<'py, Self>,
        mut indices: Vec<usize>,
        py: Python<'_>,
    ) -> PyResult<Py<Self>> {
        if !indices.iter().is_sorted() {
            // We need to sort and deduplicate the indices
            // This is a bit annoying, but it is necessary to ensure that we can
            // efficiently iterate over the underlying data.
            // We use unstable sort because we don't care about the order of equal elements
            // and it is faster than stable sort.
            let warnings = py.import("warnings")?;
            let kwargs = PyDict::new(py);
            // kwargs.set_item("stacklevel", 2)?;

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
        let base_len = ensure_base_len(&mut slf, py)?;
        if indices[0] <= 0 {
            return Err(PyException::new_err("indices must be 1-based"));
        }
        if indices.last().unwrap() > &base_len {
            return Err(PyException::new_err(format!(
                "indices must be <= number of samples in base data ({})",
                base_len
            )));
        }
        let len_hint = indices.len();

        slf.active_selection = ActiveSelection::Indices(indices.clone());
        let sel = Selection::Indices(indices.into_iter().peekable());
        reset_with_selection(&mut slf, sel, len_hint)?;
        Ok(slf.into())
    }

    #[pyo3(text_signature = "(self, start, end, /)")]
    fn subsample_range<'py>(
        mut slf: PyRefMut<'py, Self>,
        start: usize,
        end: usize,
        py: Python<'_>,
    ) -> PyResult<Py<Self>> {
        if start == 0 || end < start {
            return Err(PyException::new_err(
                "range must be 1-based and end >= start",
            ));
        }
        let base_len = ensure_base_len(&mut slf, py)?;
        if end > base_len {
            return Err(PyException::new_err(format!(
                "end must be <= number of samples in base data ({})",
                base_len
            )));
        }

        slf.active_selection = ActiveSelection::Range { start, end };
        let sel = Selection::Range { start, end };
        let len_hint = end - start + 1;
        reset_with_selection(&mut slf, sel, len_hint)?;
        Ok(slf.into())
    }

    #[pyo3(signature = (step, offset=1))]
    fn subsample_every<'py>(
        mut slf: PyRefMut<'py, Self>,
        step: usize,
        offset: usize,
        py: Python<'_>,
    ) -> PyResult<Py<Self>> {
        if step == 0 || offset == 0 {
            return Err(PyException::new_err("step and offset must be >= 1"));
        }
        let base_len = ensure_base_len(&mut slf, py)?;
        if offset > base_len {
            return Err(PyException::new_err(format!(
                "offset must be <= number of samples in base data ({})",
                base_len
            )));
        }
        slf.active_selection = ActiveSelection::Every { step, offset };
        let sel = Selection::Every { step, offset };
        let len_hint = (base_len + step - 1 - (offset - 1)) / step;
        reset_with_selection(&mut slf, sel, len_hint)?;
        Ok(slf.into())
    }

    // ---------------------------------------------------------------------
    // Bundle-inspection surface.
    //
    // These methods only make sense when the decoder was opened on a
    // `.bendl` file; on a plain `.ben`/`.xben` stream they raise a clear
    // error pointing the user at the right tool.
    // ---------------------------------------------------------------------

    /// Whether this decoder is backed by a `.bendl` bundle (`True`) or a
    /// plain `.ben`/`.xben` stream (`False`).
    #[pyo3(text_signature = "(self)")]
    fn is_bundle(&self) -> bool {
        self.backend.is_bundle()
    }

    /// Return the container format of the underlying assignment stream
    /// as `"ben"` or `"xben"`.
    #[pyo3(text_signature = "(self)")]
    fn assignment_format(&self) -> &'static str {
        self.mode.as_str()
    }

    /// Return the bundle's format version as a `(major, minor)` tuple.
    /// Errors on plain streams.
    #[pyo3(text_signature = "(self)")]
    fn version(&self) -> PyResult<(u16, u16)> {
        let state = self.require_bundle("version()")?;
        let h = state.reader.header();
        Ok((h.major_version, h.minor_version))
    }

    /// Whether the bundle was successfully finalized. Errors on plain
    /// streams.
    #[pyo3(text_signature = "(self)")]
    fn is_complete(&self) -> PyResult<bool> {
        let state = self.require_bundle("is_complete()")?;
        Ok(state.reader.is_complete())
    }

    /// Names of every entry in the bundle's directory, in directory
    /// order. Errors on plain streams.
    #[pyo3(text_signature = "(self)")]
    fn asset_names(&self) -> PyResult<Vec<String>> {
        let state = self.require_bundle("asset_names()")?;
        Ok(state
            .reader
            .assets()
            .iter()
            .map(|e| e.name.clone())
            .collect())
    }

    /// Return the full bundle directory as a list of dicts with keys
    /// `name`, `type`, `offset`, `len`, and `flags` (a list of string
    /// tags). Errors on plain streams.
    #[pyo3(text_signature = "(self)")]
    fn list_assets<'py>(&self, py: Python<'py>) -> PyResult<Vec<Bound<'py, PyDict>>> {
        let state = self.require_bundle("list_assets()")?;
        let entries = state.reader.assets();
        let mut out = Vec::with_capacity(entries.len());
        for entry in entries {
            let d = PyDict::new(py);
            d.set_item("name", &entry.name)?;
            d.set_item("type", entry.asset_type)?;
            d.set_item("offset", entry.payload_offset)?;
            d.set_item("len", entry.payload_len)?;
            let mut flags: Vec<&str> = Vec::new();
            if entry.asset_flags & ASSET_FLAG_JSON != 0 {
                flags.push("json");
            }
            if entry.asset_flags & ASSET_FLAG_XZ != 0 {
                flags.push("xz");
            }
            if entry.asset_flags & ASSET_FLAG_CHECKSUM != 0 {
                flags.push("checksum");
            }
            d.set_item("flags", flags)?;
            out.push(d);
        }
        Ok(out)
    }

    /// Read the (decoded) bytes of a named asset as a Python `bytes`
    /// object. Errors on plain streams.
    #[pyo3(text_signature = "(self, name, /)")]
    fn read_asset_bytes(&mut self, name: &str) -> PyResult<Vec<u8>> {
        let state = self.require_bundle_mut("read_asset_bytes()")?;
        let entry = state
            .reader
            .find_asset_by_name(name)
            .cloned()
            .ok_or_else(|| PyKeyError::new_err(format!("no asset named {name:?} in bundle")))?;
        state
            .reader
            .asset_bytes(&entry)
            .map_err(|e| PyIOError::new_err(format!("Failed to read asset {name:?}: {e}")))
    }

    /// Parse a JSON asset into a Python object (dict, list, …). Errors
    /// on plain streams and when the asset does not exist or is not
    /// valid UTF-8 / JSON.
    #[pyo3(text_signature = "(self, name, /)")]
    fn read_json_asset<'py>(&mut self, py: Python<'py>, name: &str) -> PyResult<Py<PyAny>> {
        let bytes = self.read_asset_bytes(name)?;
        let json_mod = py.import("json")?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|e| PyException::new_err(format!("asset {name:?} is not valid UTF-8: {e}")))?;
        let parsed = json_mod.call_method1("loads", (text,))?;
        Ok(parsed.into())
    }

    /// Read the bundle's `graph.json` asset as a parsed JSON object.
    /// Returns `None` if the bundle does not carry a graph asset. Errors
    /// on plain streams.
    #[pyo3(text_signature = "(self)")]
    fn read_graph<'py>(&mut self, py: Python<'py>) -> PyResult<Option<Py<PyAny>>> {
        {
            let state = self.require_bundle_mut("read_graph()")?;
            if state.reader.find_asset_by_type(ASSET_TYPE_GRAPH).is_none() {
                return Ok(None);
            }
        }
        Ok(Some(self.read_json_asset(py, "graph.json")?))
    }

    /// Read the bundle's `metadata.json` asset as a parsed JSON object,
    /// or `None` if absent. Errors on plain streams.
    #[pyo3(text_signature = "(self)")]
    fn read_metadata<'py>(&mut self, py: Python<'py>) -> PyResult<Option<Py<PyAny>>> {
        {
            let state = self.require_bundle_mut("read_metadata()")?;
            if state
                .reader
                .find_asset_by_type(ASSET_TYPE_METADATA)
                .is_none()
            {
                return Ok(None);
            }
        }
        Ok(Some(self.read_json_asset(py, "metadata.json")?))
    }

    /// Read the bundle's `relabel_map.json` asset as a parsed JSON
    /// object, or `None` if absent. Errors on plain streams.
    #[pyo3(text_signature = "(self)")]
    fn read_relabel_map<'py>(&mut self, py: Python<'py>) -> PyResult<Option<Py<PyAny>>> {
        {
            let state = self.require_bundle_mut("read_relabel_map()")?;
            if state
                .reader
                .find_asset_by_type(ASSET_TYPE_RELABEL_MAP)
                .is_none()
            {
                return Ok(None);
            }
        }
        Ok(Some(self.read_json_asset(py, "relabel_map.json")?))
    }

    /// Copy the embedded assignment stream region verbatim to
    /// `out_path`. The resulting file can be opened directly with
    /// `PyBenDecoder(out_path, mode=dec.assignment_format())`.
    /// Errors on plain streams.
    #[pyo3(signature = (out_path, overwrite=false))]
    #[pyo3(text_signature = "(self, out_path, overwrite=False)")]
    fn extract_stream(&mut self, out_path: PathBuf, overwrite: bool) -> PyResult<()> {
        let state = self.require_bundle_mut("extract_stream()")?;
        if out_path.exists() && !overwrite {
            return Err(PyIOError::new_err(format!(
                "Output file {} already exists (use overwrite=True to replace).",
                out_path.display()
            )));
        }
        let out = if overwrite {
            OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&out_path)
        } else {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&out_path)
        }
        .map_err(|e| PyIOError::new_err(format!("Failed to create {}: {e}", out_path.display())))?;
        let mut out = BufWriter::new(out);

        let mut stream = state
            .reader
            .assignment_stream_reader()
            .map_err(|e| PyException::new_err(format!("Failed to open stream region: {e}")))?;
        io::copy(&mut stream, &mut out)
            .map_err(|e| PyIOError::new_err(format!("Failed to copy stream bytes: {e}")))?;
        out.flush()
            .map_err(|e| PyIOError::new_err(format!("Failed to flush output: {e}")))?;
        Ok(())
    }
}

impl PyBenDecoder {
    /// Borrow the bundle state or raise a clear Python error explaining
    /// that the decoder was opened on a plain stream.
    fn require_bundle(&self, op: &str) -> PyResult<&BundleState> {
        match &self.backend {
            DecoderBackend::Bundle(state) => Ok(state),
            DecoderBackend::Plain => Err(PyException::new_err(format!(
                "{op} is only available on .bendl bundles; this decoder was opened \
                 on a plain .{} file. Wrap the stream in a .bendl bundle (e.g. \
                 via PyBenEncoder with ben_file_only=False) to get bundle features.",
                self.mode.as_str()
            ))),
        }
    }

    fn require_bundle_mut(&mut self, op: &str) -> PyResult<&mut BundleState> {
        match &mut self.backend {
            DecoderBackend::Bundle(state) => Ok(state),
            DecoderBackend::Plain => Err(PyException::new_err(format!(
                "{op} is only available on .bendl bundles; this decoder was opened \
                 on a plain .{} file. Wrap the stream in a .bendl bundle (e.g. \
                 via PyBenEncoder with ben_file_only=False) to get bundle features.",
                self.mode.as_str()
            ))),
        }
    }
}

fn reset_with_selection(
    decoder: &mut PyBenDecoder,
    selection: Selection,
    len_hint: usize,
) -> PyResult<()> {
    let frames = build_frames_for_subsample(&decoder.path, decoder.mode, &decoder.backend)?;
    let frame_decoder = SubsampleFrameDecoder::new(frames, selection);
    decoder.iter = Box::new(frame_decoder);
    decoder.current_assignment = None;
    decoder.remaining_count = 0;
    decoder.len_hint = Some(len_hint);
    Ok(())
}

fn ensure_base_len(decoder: &mut PyBenDecoder, py: Python<'_>) -> PyResult<usize> {
    if let Some(base_len) = decoder.base_len {
        return Ok(base_len);
    }

    let base_len = match &decoder.backend {
        DecoderBackend::Plain => {
            let path = decoder.path.clone();
            let mode = decoder.mode.as_str().to_string();
            py.detach(|| count_samples_from_file(&path, &mode))
                .map_err(|e| {
                    PyException::new_err(format!(
                        "Failed to count samples in {}: {e}",
                        path.display()
                    ))
                })?
        }
        DecoderBackend::Bundle(state) => {
            // Prefer the authoritative sample_count carried in the
            // bundle header, which is set for finalized bundles and is
            // O(1). Fall back to scanning the stream region when the
            // header has no count (unfinalized append target, or a
            // header byte we cannot interpret).
            if let Some(n) = state.reader.sample_count() {
                if n >= 0 {
                    n as usize
                } else {
                    scan_bundle_samples(&decoder.path, state, decoder.mode)?
                }
            } else {
                scan_bundle_samples(&decoder.path, state, decoder.mode)?
            }
        }
    };
    decoder.base_len = Some(base_len);
    Ok(base_len)
}
