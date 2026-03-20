mod ben_decode;
mod ben_encode;
mod mkv_encode;
mod twodelta;

pub use ben_decode::BenDecodeFrame;
pub use ben_encode::BenEncodeFrame;
pub use mkv_encode::MkvBenEncodeFrame;
pub use twodelta::TwoDeltaEncodeFrame;

use crate::util::rle::assign_to_rle;

pub trait BenConstruct {
    fn from_rle(runs: Vec<(u16, u16)>, count: Option<u16>) -> Self;

    fn from_assignment(assignments: impl AsRef<[u16]>, count: Option<u16>) -> Self
    where
        Self: Sized,
    {
        Self::from_rle(assign_to_rle(assignments), count)
    }
}

/// Compresses a run-length encoded vector into BEN payload bytes.
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
