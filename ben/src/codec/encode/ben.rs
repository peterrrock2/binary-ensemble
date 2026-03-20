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
