//! Frame-layer types — one sample's encoded bytes.
//!
//! See `docs/glossary.md` for the encoding-stack layering. This module owns layer 2 (frame). Each
//! direction is a single enum whose arms mirror [`crate::BenVariant`]:
//!
//! - [`BenEncodeFrame`] is built **from** RLE runs (or a pair + run-length vector for the
//!   `TwoDelta` arm) and carries the source representation alongside the serialized bytes.
//! - [`BenDecodeFrame`] is built **from** wire bytes and keeps the bit-packed payload opaque on
//!   `Standard`/`MkvChain` arms so frame-level subsampling stays cheap (no eager bit-unpacking).

mod decode;
mod encode;

#[cfg(test)]
mod tests;

pub use decode::BenDecodeFrame;
pub use encode::BenEncodeFrame;

/// Bit-pack an RLE run vector into a serialized BEN frame payload.
///
/// Output layout:
///
/// ```text
/// [max_val_bit_count: u8][max_len_bit_count: u8][n_bytes: u32 BE][packed payload...]
/// ```
pub(super) fn compress_rle_to_ben_bytes(
    max_val_bit_count: u8,
    max_len_bit_count: u8,
    n_bytes: u32,
    runs: &Vec<(u16, u16)>,
) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(6 + n_bytes as usize);
    bytes.push(max_val_bit_count);
    bytes.push(max_len_bit_count);
    bytes.extend_from_slice(&n_bytes.to_be_bytes());

    let mut remainder: u32 = 0;
    let mut remainder_bits: u8 = 0;

    for &(val, len) in runs {
        let mut packed = (remainder << max_val_bit_count) | (val as u32);
        let mut bits_left = remainder_bits + max_val_bit_count;

        while bits_left >= 8 {
            bits_left -= 8;
            bytes.push((packed >> bits_left) as u8);
            packed &= !((u32::MAX) << bits_left);
        }

        packed = (packed << max_len_bit_count) | (len as u32);
        bits_left += max_len_bit_count;

        while bits_left >= 8 {
            bits_left -= 8;
            bytes.push((packed >> bits_left) as u8);
            packed &= !((u32::MAX) << bits_left);
        }

        remainder = packed;
        remainder_bits = bits_left;
    }

    if remainder_bits > 0 {
        bytes.push((remainder << (8 - remainder_bits)) as u8);
    }

    bytes
}
