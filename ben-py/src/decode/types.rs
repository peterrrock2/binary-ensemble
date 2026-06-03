use binary_ensemble::io::bundle::format::AssignmentFormat;
use binary_ensemble::io::reader::{BenWireFormat, MkvRecord, Selection};
use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use std::io;
use std::path::PathBuf;

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

/// Where the iterable assignment stream lives.
///
/// A plain `.ben`/`.xben` file is read from the start; a `.bendl` bundle is read through a second
/// file handle bounded to the embedded stream region. Carrying the region offsets (rather than a
/// live [`binary_ensemble::io::bundle::BendlReader`]) keeps the iteration core free of the bundle
/// inspection surface, so [`super::cursor::SampleCursor`] is shared verbatim between the stream and
/// bundle decoders.
#[derive(Clone)]
pub(super) enum StreamSource {
    Plain {
        path: PathBuf,
    },
    Bundle {
        path: PathBuf,
        stream_offset: u64,
        stream_len: u64,
        /// Authoritative sample count from a finalized bundle header, or `None` when the bundle is
        /// unfinalized (forcing a stream scan).
        header_sample_count: Option<i64>,
        /// `true` for a finalized bundle whose stream region is empty (an assets-only bundle with no
        /// BEN banner). Iteration over such a source yields nothing instead of failing on the
        /// missing banner.
        empty: bool,
    },
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
