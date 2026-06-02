pub(crate) const XBEN_TWODELTA_FULL_TAG: u8 = 0;
pub(crate) const XBEN_TWODELTA_CHUNK_TAG: u8 = 2;

// Per-frame discriminator prepended to every frame of a plain-BEN `TwoDelta` stream (writer copy in
// `io::writer::twodelta`). Distinct from the XBEN columnar tags above; the two copies must agree.
pub(crate) const BEN_TWODELTA_SNAPSHOT_TAG: u8 = 0x00;
pub(crate) const BEN_TWODELTA_DELTA_TAG: u8 = 0x01;
