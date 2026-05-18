//! Encode-side configuration knobs for the unified BEN stream writer.
//!
//! Mirrors the discipline of `RelabelOptions`: a `#[non_exhaustive]` struct with private fields and
//! value-taking builder setters, so adding a knob later is non-breaking. `None` semantically means
//! "use the codec/lzma default" and is distinct from any specific user-provided value; callers who
//! want defaults simply do not call the setter.

use super::twodelta::DEFAULT_TWODELTA_CHUNK_SIZE;

/// Encode-side knobs for `BenStreamWriter::for_xben`.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct XzEncodeOptions {
    pub(crate) n_threads: Option<u32>,
    pub(crate) compression_level: Option<u32>,
    pub(crate) block_size: Option<u64>,
    pub(crate) twodelta_chunk_size: usize,
}

impl XzEncodeOptions {
    /// Build the default options. Matches today's `None`/`DEFAULT_TWODELTA_CHUNK_SIZE`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the XZ encoder thread count. `0` normalizes to `1`.
    pub fn with_n_threads(mut self, n: u32) -> Self {
        self.n_threads = Some(n.max(1));
        self
    }

    /// Set the XZ compression level. Clamped to `0..=9`.
    pub fn with_compression_level(mut self, level: u32) -> Self {
        self.compression_level = Some(level.min(9));
        self
    }

    /// Set the XZ per-block size in bytes.
    pub fn with_block_size(mut self, size: u64) -> Self {
        self.block_size = Some(size);
        self
    }

    /// Set the TwoDelta columnar chunk size. `0` normalizes to `1`. Ignored for Standard and
    /// MkvChain XBEN streams.
    pub fn with_twodelta_chunk_size(mut self, size: usize) -> Self {
        self.twodelta_chunk_size = size.max(1);
        self
    }
}

impl Default for XzEncodeOptions {
    fn default() -> Self {
        Self {
            n_threads: None,
            compression_level: None,
            block_size: None,
            twodelta_chunk_size: DEFAULT_TWODELTA_CHUNK_SIZE,
        }
    }
}
