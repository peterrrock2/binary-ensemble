use crate::codec::encode::traits::{FromAssign, FromRLE};
use crate::codec::frames::{BenEncodeFrame, MkvBenEncodeFrame};
use crate::util::rle::assign_to_rle;
use serde_json::Value;
use std::io::{Error, ErrorKind, Result};

/// Encode a JSON assignment record into the ben32 frame representation used by
/// XBEN streams.
///
/// Note: This is a helper function that is only used in the testing suite.
///
/// # Arguments
///
/// * `data` - A JSON object containing an `assignment` array.
///
/// # Returns
///
/// Returns the encoded ben32 frame bytes terminated by the four-byte `0`
/// sentinel.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn encode_ben32_line(data: Value) -> Result<Vec<u8>> {
    let json_value_assign_vec = match data["assignment"].as_array() {
        Some(vec) => vec,
        None => {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "'assignment' field either missing or is not an array of integers",
            ))
        }
    };

    let possible_assign_vec = json_value_assign_vec
        .iter()
        .map(|assignment| {
            let assign_u64 = assignment.as_u64().ok_or_else(|| {
                Error::new(
                    ErrorKind::InvalidData,
                    format!(
                        "The value '{}' could not be unwrapped as an unsigned 64 bit integer.",
                        assignment
                    ),
                )
            })?;
            u16::try_from(assign_u64).map_err(|_| {
                Error::new(
                    ErrorKind::InvalidData,
                    format!("The value '{}' is too large to fit in a u16.", assign_u64),
                )
            })
        })
        .collect::<Result<Vec<u16>>>();

    match possible_assign_vec {
        Ok(vec) => encode_ben32_assignments(vec),
        Err(e) => Err(e),
    }
}

/// Encode an assignment vector into a Ben32 vector
///
/// # Arguments
///
/// * `assign_vec` - The full assignment vector to encode
///
/// # Returns
///
/// Returns the encoded BEN32 frame byte vector.
pub(crate) fn encode_ben32_assignments(assign_vec: impl AsRef<[u16]>) -> Result<Vec<u8>> {
    let assign_vec = assign_vec.as_ref();
    let mut prev_assign: u16 = 0;
    let mut count: u16 = 0;
    let mut first = true;

    let mut ret = Vec::new();

    for &assign in assign_vec {
        if first {
            prev_assign = assign;
            count = 1;
            first = false;
            continue;
        }
        if assign == prev_assign {
            count += 1;
        } else {
            let encoded = (prev_assign as u32) << 16 | count as u32;
            ret.extend(&encoded.to_be_bytes());
            prev_assign = assign;
            count = 1;
        }
    }

    if count > 0 {
        let encoded = (prev_assign as u32) << 16 | count as u32;
        ret.extend(&encoded.to_be_bytes());
    }

    ret.extend([0, 0, 0, 0]);
    Ok(ret)
}

/// Compresses a Run-length encoded vector into a BEN-bytes vector.
fn compress_rle_to_bytes(
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

impl FromRLE for BenEncodeFrame {
    /// Build a frame from an RLE run vector.
    fn from_rle(runs: Vec<(u16, u16)>, _count: Option<u16>) -> Self {
        let (max_val, max_len) = runs
            .iter()
            .fold((0u16, 0u16), |(max_val, max_len), &(val, len)| {
                (max_val.max(val), max_len.max(len))
            });
        let max_val_bit_count = (16 - max_val.leading_zeros() as u8).max(1);
        let max_len_bit_count = (16 - max_len.leading_zeros() as u8).max(1);
        let assign_bits = (max_val_bit_count + max_len_bit_count) as u32;
        let payload_bits = assign_bits * runs.len() as u32;
        let n_bytes = payload_bits.div_ceil(8);
        let raw_bytes = compress_rle_to_bytes(max_val_bit_count, max_len_bit_count, n_bytes, &runs);

        Self {
            runs,
            max_val_bit_count,
            max_len_bit_count,
            n_bytes,
            raw_bytes,
        }
    }
}

impl FromAssign for BenEncodeFrame {
    /// Build a frame from a full assignment vector.
    fn from_assignment(assignments: impl AsRef<[u16]>, _count: Option<u16>) -> Self {
        Self::from_rle(assign_to_rle(assignments), _count)
    }
}

impl FromRLE for MkvBenEncodeFrame {
    /// Build a frame from an RLE run vector.
    fn from_rle(runs: Vec<(u16, u16)>, count: Option<u16>) -> Self {
        let count = match count {
            Some(v) => v,
            None => 1,
        };

        let (max_val, max_len) = runs
            .iter()
            .fold((0u16, 0u16), |(max_val, max_len), &(val, len)| {
                (max_val.max(val), max_len.max(len))
            });
        let max_val_bit_count = (16 - max_val.leading_zeros() as u8).max(1);
        let max_len_bit_count = (16 - max_len.leading_zeros() as u8).max(1);
        let assign_bits = (max_val_bit_count + max_len_bit_count) as u32;
        let payload_bits = assign_bits * runs.len() as u32;
        let n_bytes = payload_bits.div_ceil(8);
        let mut raw_bytes =
            compress_rle_to_bytes(max_val_bit_count, max_len_bit_count, n_bytes, &runs);

        raw_bytes.extend(count.to_be_bytes());

        Self {
            runs,
            max_val_bit_count,
            max_len_bit_count,
            n_bytes,
            raw_bytes,
            count,
        }
    }
}

impl FromAssign for MkvBenEncodeFrame {
    /// Build a frame from a full assignment vector.
    fn from_assignment(assignments: impl AsRef<[u16]>, count: Option<u16>) -> Self {
        Self::from_rle(assign_to_rle(assignments), count)
    }
}
