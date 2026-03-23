use super::twodelta::XBEN_TWODELTA_FULL_TAG;

use crate::util::rle::assign_to_rle;
use serde_json::Value;
use std::io::{self, Result};

/// Extract and validate the `assignment` array from a JSON object.
///
/// # Arguments
///
/// * `data` - A JSON value expected to contain an `assignment` array of integers.
///
/// # Returns
///
/// Returns a `Vec<u16>` of assignment values, or an error if the field is
/// missing, not an array, or contains values that do not fit in a `u16`.
pub(super) fn parse_json_assignment(data: Value) -> Result<Vec<u16>> {
    let assign_vec = data["assignment"].as_array().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "'assignment' field either missing or is not an array of integers",
        )
    })?;

    assign_vec
        .iter()
        .map(|x| {
            let u = x.as_u64().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "The value '{}' could not be unwrapped as an unsigned 64 bit integer.",
                        x
                    ),
                )
            })?;

            u16::try_from(u).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("The value '{}' is too large to fit in a u16.", u),
                )
            })
        })
        .collect()
}

/// Encode an assignment vector as a full XBEN two-delta frame.
///
/// The frame begins with a full-frame tag byte followed by RLE-encoded
/// assignment runs in big-endian format.
///
/// # Arguments
///
/// * `assignments` - The full assignment vector to encode.
///
/// # Returns
///
/// Returns the encoded frame as a byte vector.
pub(super) fn encode_xben_twodelta_full_frame(assignments: &[u16]) -> Vec<u8> {
    let runs = assign_to_rle(assignments);
    let mut bytes = Vec::with_capacity(1 + 4 + runs.len() * 4);
    bytes.push(XBEN_TWODELTA_FULL_TAG);
    bytes.extend_from_slice(&(runs.len() as u32).to_be_bytes());
    for (value, len) in runs {
        bytes.extend_from_slice(&value.to_be_bytes());
        bytes.extend_from_slice(&len.to_be_bytes());
    }
    bytes
}
