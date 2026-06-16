//! TwoDelta frame discriminator tags, shared by the reader and writer.
//!
//! These are wire-format constants: the reader and writer must agree on every value, so they live
//! in one place and both `io::reader::twodelta` and `io::writer::twodelta` re-export them.

/// XBEN columnar TwoDelta tag for a full (snapshot) column.
pub(crate) const XBEN_TWODELTA_FULL_TAG: u8 = 0;
/// XBEN columnar TwoDelta tag for a delta-encoded chunk.
pub(crate) const XBEN_TWODELTA_CHUNK_TAG: u8 = 2;

/// Per-frame discriminator prepended to every frame of a plain-BEN `TwoDelta` stream. This is a
/// distinct namespace from the XBEN columnar tags above: a BEN stream interleaves self-describing
/// snapshot and delta frames, so the wire format is `[tag u8][body]`.
pub(crate) const BEN_TWODELTA_SNAPSHOT_TAG: u8 = 0x00;
pub(crate) const BEN_TWODELTA_DELTA_TAG: u8 = 0x01;
