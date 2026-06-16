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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_equals_default() {
        let a = XzEncodeOptions::new();
        let b = XzEncodeOptions::default();
        assert_eq!(a.n_threads, b.n_threads);
        assert_eq!(a.compression_level, b.compression_level);
        assert_eq!(a.block_size, b.block_size);
        assert_eq!(a.twodelta_chunk_size, b.twodelta_chunk_size);
    }

    #[test]
    fn defaults_are_none_and_default_chunk_size() {
        let o = XzEncodeOptions::default();
        assert_eq!(o.n_threads, None);
        assert_eq!(o.compression_level, None);
        assert_eq!(o.block_size, None);
        assert_eq!(o.twodelta_chunk_size, DEFAULT_TWODELTA_CHUNK_SIZE);
    }

    #[test]
    fn with_n_threads_clamps_zero_to_one() {
        // The clamp is part of the contract: the underlying xz mt encoder requires ≥1.
        assert_eq!(XzEncodeOptions::new().with_n_threads(0).n_threads, Some(1));
        assert_eq!(XzEncodeOptions::new().with_n_threads(8).n_threads, Some(8));
    }

    #[test]
    fn with_compression_level_clamps_to_nine() {
        assert_eq!(
            XzEncodeOptions::new()
                .with_compression_level(99)
                .compression_level,
            Some(9)
        );
        // Level 0 (store-mode) is a legitimate setting and must be preserved as-is.
        assert_eq!(
            XzEncodeOptions::new()
                .with_compression_level(0)
                .compression_level,
            Some(0)
        );
        assert_eq!(
            XzEncodeOptions::new()
                .with_compression_level(6)
                .compression_level,
            Some(6)
        );
    }

    #[test]
    fn with_block_size_round_trips_any_value() {
        let o = XzEncodeOptions::new().with_block_size(64 * 1024 * 1024);
        assert_eq!(o.block_size, Some(64 * 1024 * 1024));
    }

    #[test]
    fn with_twodelta_chunk_size_clamps_zero_to_one() {
        assert_eq!(
            XzEncodeOptions::new()
                .with_twodelta_chunk_size(0)
                .twodelta_chunk_size,
            1
        );
        assert_eq!(
            XzEncodeOptions::new()
                .with_twodelta_chunk_size(7)
                .twodelta_chunk_size,
            7
        );
    }

    #[test]
    fn chained_builder_composes_all_fields() {
        let o = XzEncodeOptions::new()
            .with_n_threads(4)
            .with_compression_level(3)
            .with_block_size(1024)
            .with_twodelta_chunk_size(128);
        assert_eq!(o.n_threads, Some(4));
        assert_eq!(o.compression_level, Some(3));
        assert_eq!(o.block_size, Some(1024));
        assert_eq!(o.twodelta_chunk_size, 128);
    }
}
