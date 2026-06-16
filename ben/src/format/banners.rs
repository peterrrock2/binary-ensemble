//! Banner constants and helpers for BEN and XBEN streams.

use crate::BenVariant;

/// Fixed byte length of every BEN/XBEN banner.
pub const BANNER_LEN: usize = 17;
/// Banner for standard BEN/XBEN streams.
pub const STANDARD_BEN_BANNER: &[u8; BANNER_LEN] = b"STANDARD BEN FILE";
/// Banner for MKVChain BEN/XBEN streams.
pub const MKVCHAIN_BEN_BANNER: &[u8; BANNER_LEN] = b"MKVCHAIN BEN FILE";
/// Banner for TwoDelta BEN/XBEN streams.
pub const TWODELTA_BEN_BANNER: &[u8; BANNER_LEN] = b"TWODELTA BEN FILE";

/// Return the banner used by a BEN variant.
pub fn banner_for_variant(variant: BenVariant) -> &'static [u8; BANNER_LEN] {
    match variant {
        BenVariant::Standard => STANDARD_BEN_BANNER,
        BenVariant::MkvChain => MKVCHAIN_BEN_BANNER,
        BenVariant::TwoDelta => TWODELTA_BEN_BANNER,
    }
}

/// Parse a BEN/XBEN banner into its variant.
pub fn variant_from_banner(banner: &[u8; BANNER_LEN]) -> Option<BenVariant> {
    match banner {
        STANDARD_BEN_BANNER => Some(BenVariant::Standard),
        MKVCHAIN_BEN_BANNER => Some(BenVariant::MkvChain),
        TWODELTA_BEN_BANNER => Some(BenVariant::TwoDelta),
        _ => None,
    }
}

/// Return whether the given bytes begin with a known BEN/XBEN banner.
pub fn has_known_banner_prefix(bytes: &[u8]) -> bool {
    bytes.starts_with(STANDARD_BEN_BANNER)
        || bytes.starts_with(MKVCHAIN_BEN_BANNER)
        || bytes.starts_with(TWODELTA_BEN_BANNER)
}
