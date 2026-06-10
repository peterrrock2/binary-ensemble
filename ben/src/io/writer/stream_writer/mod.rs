//! Unified writer for the BEN-stack stream layer (layer 3 — see `docs/glossary.md`).
//!
//! Hides the wire-format choice (BEN bit-packed vs ben32 / XBEN columnar) and the transport choice
//! (plain vs xz-compressed) behind one type.

mod ben;
mod xben;

#[cfg(test)]
pub(super) mod test_helpers {
    pub(crate) use super::ben::twodelta_repeat_frame;
    pub(crate) use super::xben::twodelta_repeat_buffered_frame;
}

use std::io::{self, BufRead, Write};

use serde_json::Value;
use xz2::stream::Stream;
use xz2::write::XzEncoder;

use crate::codec::encode::xz::{build_mt_stream, resolve_threads};
use crate::format::banners::banner_for_variant;
use crate::BenVariant;

use super::options::XzEncodeOptions;
use super::utils::parse_json_assignment;

pub use crate::io::reader::BenWireFormat;

use ben::BenState;
use xben::XBenInner;

/// Writer for an encoded BEN-stack stream of samples (layer 3 — see `docs/glossary.md`).
///
/// Construct with [`BenStreamWriter::for_ben`] for plain BEN or [`BenStreamWriter::for_xben`] for
/// XBEN. `write_assignment` is available on both arms; `write_frame` is plain-BEN-only and
/// preserves one frame boundary per call. Calling `write_frame` on an XBEN writer returns
/// `InvalidInput`.
pub struct BenStreamWriter<W: Write> {
    /// Wrapped in `Option` so [`Self::finish_into_inner`] can `take()` it without partial-moving
    /// out of a `Drop` type. All other access sites unwrap with `.expect("inner present")` — only
    /// the consuming `finish_into_inner` ever leaves it `None`.
    inner: Option<BenStreamInner<W>>,
    state: WriterState,
    /// Tracks whether any sample-writing or direct-ingest operation has touched the writer.
    /// `ingest_ben_stream` requires this to be `false`.
    body_started: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WriterState {
    Open,
    BodyClosed,
    Complete,
    Failed,
}

enum BenStreamInner<W: Write> {
    Ben(BenState<W>),
    XBen(Box<XBenInner<W>>),
}

impl<W: Write> BenStreamWriter<W> {
    /// Open a plain-BEN writer. Emits the BEN banner immediately.
    ///
    /// On error, the underlying `writer` is dropped — no partial `BenStreamWriter` is returned. The
    /// caller treats the output as failed and discards.
    pub fn for_ben(mut writer: W, variant: BenVariant) -> io::Result<Self> {
        writer.write_all(banner_for_variant(variant))?;
        Ok(Self {
            inner: Some(BenStreamInner::Ben(BenState::new(writer, variant))),
            state: WriterState::Open,
            body_started: false,
        })
    }

    /// Open an XBEN writer. Builds the xz encoder from `options` and emits the BEN banner inside
    /// the compressed stream.
    pub fn for_xben(writer: W, variant: BenVariant, options: XzEncodeOptions) -> io::Result<Self> {
        let n_cpus = resolve_threads(options.n_threads);
        let level = options.compression_level.unwrap_or(9).min(9);
        let mt: Stream = build_mt_stream(n_cpus, level, options.block_size)?;
        let encoder = XzEncoder::new_stream(writer, mt);
        Self::for_xben_with_encoder(encoder, variant, Some(options.twodelta_chunk_size))
    }

    /// Open an XBEN writer around an already-built xz encoder. Used by codec plumbing that
    /// constructs encoders explicitly. The TwoDelta chunk size is passed independently because
    /// compression options have already been consumed building the encoder; `None` means default.
    pub(crate) fn for_xben_with_encoder(
        mut encoder: XzEncoder<W>,
        variant: BenVariant,
        twodelta_chunk_size: Option<usize>,
    ) -> io::Result<Self> {
        encoder.write_all(banner_for_variant(variant))?;
        let chunk_size = twodelta_chunk_size
            .unwrap_or(super::twodelta::DEFAULT_TWODELTA_CHUNK_SIZE)
            .max(1);
        Ok(Self {
            inner: Some(BenStreamInner::XBen(Box::new(XBenInner::new(
                encoder, variant, chunk_size,
            )))),
            state: WriterState::Open,
            body_started: false,
        })
    }

    /// The BEN variant of this stream.
    pub fn variant(&self) -> BenVariant {
        match self.inner.as_ref().expect("inner present") {
            BenStreamInner::Ben(b) => b.variant,
            BenStreamInner::XBen(x) => x.variant(),
        }
    }

    /// The wire format (BEN vs XBEN) of this stream.
    pub fn wire_format(&self) -> BenWireFormat {
        match self.inner.as_ref().expect("inner present") {
            BenStreamInner::Ben(_) => BenWireFormat::Ben,
            BenStreamInner::XBen(_) => BenWireFormat::XBen,
        }
    }

    /// Encode one assignment vector. Count-capable formats buffer adjacent-equal assignments into
    /// counted frames; XBEN-Standard writes each assignment immediately, and Standard BEN expands
    /// buffered counts into one-sample frames on flush.
    pub fn write_assignment(&mut self, assign_vec: Vec<u16>) -> io::Result<()> {
        match self.state {
            WriterState::Complete | WriterState::Failed | WriterState::BodyClosed => {
                return Err(invalid_input(
                    "writer is not in a state that accepts samples",
                ));
            }
            WriterState::Open => {}
        }

        self.body_started = true;
        let result = match self.inner.as_mut().expect("inner present") {
            BenStreamInner::Ben(b) => b.write_assignment(assign_vec),
            BenStreamInner::XBen(x) => x.write_assignment(assign_vec),
        };
        if result.is_err() {
            self.state = WriterState::Failed;
        }
        result
    }

    /// Plain-BEN only: encode one assignment vector with a caller-supplied count. MkvChain/TwoDelta
    /// emit one counted frame; Standard expands `count` into one-sample frames.
    ///
    /// Guard order: writer-state, then mode, then zero-count no-op, then the stateful flush/encode
    /// path.
    pub fn write_frame(&mut self, assignment: Vec<u16>, count: u16) -> io::Result<()> {
        match self.state {
            WriterState::Complete | WriterState::Failed | WriterState::BodyClosed => {
                return Err(invalid_input(
                    "writer is not in a state that accepts frames",
                ));
            }
            WriterState::Open => {}
        }
        let ben = match self.inner.as_mut().expect("inner present") {
            BenStreamInner::Ben(b) => b,
            BenStreamInner::XBen(_) => {
                return Err(invalid_input("write_frame is plain-BEN-only"));
            }
        };
        if count == 0 {
            return Ok(());
        }

        self.body_started = true;
        let result = ben.write_frame(assignment, count);
        if result.is_err() {
            self.state = WriterState::Failed;
        }
        result
    }

    /// Encode one JSON assignment record.
    pub fn write_json_value(&mut self, data: Value) -> io::Result<()> {
        match self.state {
            WriterState::Complete | WriterState::Failed | WriterState::BodyClosed => {
                return Err(invalid_input(
                    "writer is not in a state that accepts samples",
                ));
            }
            WriterState::Open => {}
        }
        // JSON parse is preflight: failure does not poison.
        let new_assign = parse_json_assignment(data)?;
        // From here on, we are in the stateful encode path.
        self.body_started = true;
        let result = match self.inner.as_mut().expect("inner present") {
            BenStreamInner::Ben(b) => b.write_assignment(new_assign),
            BenStreamInner::XBen(x) => x.write_assignment(new_assign),
        };
        if result.is_err() {
            self.state = WriterState::Failed;
        }
        result
    }

    /// Crate-private XBEN-only direct ingest. Fresh-writer-only and terminal for sample writes: on
    /// success the writer transitions to `BodyClosed` and only `finish()` remains valid.
    pub(crate) fn ingest_ben_stream(&mut self, reader: impl BufRead) -> io::Result<()> {
        match self.state {
            WriterState::Complete | WriterState::Failed | WriterState::BodyClosed => {
                return Err(invalid_input(
                    "writer is not in a state that accepts ingest",
                ));
            }
            WriterState::Open => {}
        }
        let xben = match self.inner.as_mut().expect("inner present") {
            BenStreamInner::Ben(_) => {
                return Err(invalid_input("ingest_ben_stream requires XBEN mode"));
            }
            BenStreamInner::XBen(x) => x,
        };
        if self.body_started {
            return Err(invalid_input(
                "ingest_ben_stream requires a fresh writer with no prior sample writes",
            ));
        }

        self.body_started = true;
        let result = xben.ingest_ben_stream(reader);
        match result {
            Ok(()) => {
                self.state = WriterState::BodyClosed;
                Ok(())
            }
            Err(e) => {
                self.state = WriterState::Failed;
                Err(e)
            }
        }
    }

    /// Flush buffered BEN/XBEN state and finalize the underlying compressed stream when present.
    /// Valid from `Open` and `BodyClosed`. Repeated `finish()` after success returns `Ok(())`. Once
    /// finalization enters the stateful path, any encode/writer/encoder error transitions the
    /// writer to `Failed`; subsequent calls return `InvalidInput`.
    pub fn finish(&mut self) -> io::Result<()> {
        match self.state {
            WriterState::Complete => return Ok(()),
            WriterState::Failed => {
                return Err(invalid_input("writer was poisoned by a prior error"));
            }
            WriterState::Open | WriterState::BodyClosed => {}
        }

        let result: io::Result<()> = match self.inner.as_mut().expect("inner present") {
            BenStreamInner::Ben(b) => {
                if self.state == WriterState::Open {
                    b.flush_pending_frame()
                } else {
                    Ok(())
                }
            }
            BenStreamInner::XBen(x) => {
                let flush_res = if self.state == WriterState::Open {
                    x.flush()
                } else {
                    Ok(())
                };
                match flush_res {
                    Ok(()) => x.encoder.try_finish(),
                    Err(e) => Err(e),
                }
            }
        };

        match result {
            Ok(()) => {
                self.state = WriterState::Complete;
                Ok(())
            }
            Err(e) => {
                self.state = WriterState::Failed;
                Err(e)
            }
        }
    }

    /// Consume the writer, flush any buffered state, finalize the underlying compressed stream when
    /// present (XBEN), and return the underlying `W`.
    ///
    /// Unlike `std::io::BufWriter::into_inner`, this method's name is intentionally
    /// `finish_into_inner` because errors from the BEN flush or the consuming `XzEncoder::finish()`
    /// can still lose access to the inner writer. Returns `InvalidInput` if the writer is in
    /// `Failed`. Accepted from `Open`, `BodyClosed`, and `Complete`; the `Complete` path simply
    /// extracts the inner writer after prior finalization.
    pub fn finish_into_inner(mut self) -> io::Result<W> {
        let state = self.state;
        match state {
            WriterState::Failed => return Err(invalid_input("writer was poisoned")),
            WriterState::Open | WriterState::BodyClosed | WriterState::Complete => {}
        }
        let inner = self.inner.take().expect("inner present");
        match inner {
            BenStreamInner::Ben(mut b) => {
                if state == WriterState::Open {
                    b.flush_pending_frame()?;
                }
                Ok(b.writer)
            }
            BenStreamInner::XBen(boxed) => {
                let mut x = *boxed;
                if state == WriterState::Open {
                    x.flush()?;
                }
                x.encoder.finish()
            }
        }
    }
}

impl<W: Write> Drop for BenStreamWriter<W> {
    fn drop(&mut self) {
        if self.inner.is_some() && matches!(self.state, WriterState::Open | WriterState::BodyClosed)
        {
            // Best-effort safety net only: Drop cannot propagate errors, so a failed final flush
            // here means the output is incomplete. Callers that care must call `finish()`
            // explicitly; the warn makes a forgotten finish diagnosable instead of silent.
            if let Err(e) = self.finish() {
                tracing::warn!(
                    "BenStreamWriter dropped without an explicit finish and the final flush \
                     failed; output is incomplete: {e}"
                );
            }
        }
    }
}

fn invalid_input(msg: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, msg)
}
