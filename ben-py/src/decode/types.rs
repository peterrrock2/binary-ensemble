use binary_ensemble::io::bundle::format::AssignmentFormat;
use binary_ensemble::io::bundle::BendlReader;
use binary_ensemble::io::reader::{BenWireFormat, MkvRecord, Selection};
use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use std::fs::File;
use std::io::{self, BufReader};

pub(super) type DynIter = Box<dyn Iterator<Item = io::Result<MkvRecord>> + Send>;

#[derive(Clone, Copy)]
pub(super) enum DecoderMode {
    Ben,
    XBen,
}

impl DecoderMode {
    pub(super) fn parse(mode: &str) -> PyResult<Self> {
        match mode {
            "ben" => Ok(Self::Ben),
            "xben" => Ok(Self::XBen),
            _ => Err(PyException::new_err(
                "Unknown mode. Supported modes are 'ben' and 'xben'.",
            )),
        }
    }

    pub(super) fn as_str(&self) -> &'static str {
        match self {
            Self::Ben => "ben",
            Self::XBen => "xben",
        }
    }

    pub(super) fn wire_format(&self) -> BenWireFormat {
        match self {
            Self::Ben => BenWireFormat::Ben,
            Self::XBen => BenWireFormat::XBen,
        }
    }

    pub(super) fn from_assignment_format(fmt: AssignmentFormat) -> Self {
        match fmt {
            AssignmentFormat::Ben => Self::Ben,
            AssignmentFormat::Xben => Self::XBen,
        }
    }
}

/// Cached bundle state for a decoder opened on a `.bendl` file.
///
/// Holds a dedicated [`BendlReader`] so the decoder can satisfy TOC inspection and asset-read calls
/// without disturbing the iterator (which reads the stream region through a separate file handle).
pub(super) struct BundleState {
    pub reader: BendlReader<BufReader<File>>,
    pub stream_offset: u64,
    pub stream_len: u64,
}

/// What the decoder was actually opened on.
pub(super) enum DecoderBackend {
    Plain,
    Bundle(BundleState),
}

impl DecoderBackend {
    pub(super) fn is_bundle(&self) -> bool {
        matches!(self, DecoderBackend::Bundle(_))
    }
}

/// Stored form of the most recently installed subsampling selection.
///
/// The iterator is single-pass, so to support restarting iteration (e.g.
/// `for x in dec: ... ; for x in dec: ...`) the decoder remembers the active selection and rebuilds
/// a fresh frame decoder on every call to `__iter__`.
#[derive(Clone)]
pub(super) enum ActiveSelection {
    None,
    Indices(Vec<usize>),
    Range { start: usize, end: usize },
    Every { step: usize, offset: usize },
}

impl ActiveSelection {
    pub(super) fn to_selection(&self) -> Option<Selection> {
        match self {
            Self::None => None,
            Self::Indices(v) => Some(Selection::Indices(v.clone().into_iter().peekable())),
            Self::Range { start, end } => Some(Selection::Range {
                start: *start,
                end: *end,
            }),
            Self::Every { step, offset } => Some(Selection::Every {
                step: *step,
                offset: *offset,
            }),
        }
    }
}
