use crate::util::rle::assign_to_rle;
use serde_json::Value;
use std::io;

pub(crate) fn encode_ben32_line(data: Value) -> io::Result<Vec<u8>> {
    let assign_vec = data["assignment"].as_array().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "'assignment' field either missing or is not an array of integers",
        )
    })?;
    let mut prev_assign: u16 = 0;
    let mut count: u16 = 0;
    let mut first = true;

    let mut ret = Vec::new();

    for assignment in assign_vec {
        let assign_u64 = assignment.as_u64().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "The value '{}' could not be unwrapped as an unsigned 64 bit integer.",
                    assignment
                ),
            )
        })?;
        let assign = u16::try_from(assign_u64).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("The value '{}' is too large to fit in a u16.", assign_u64),
            )
        })?;
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

pub fn encode_ben_vec_from_assign(assign_vec: Vec<u16>) -> Vec<u8> {
    let rle_vec: Vec<(u16, u16)> = assign_to_rle(assign_vec);
    encode_ben_vec_from_rle(rle_vec)
}

pub fn encode_ben_vec_from_rle(rle_vec: Vec<(u16, u16)>) -> Vec<u8> {
    let mut output_vec: Vec<u8> = Vec::new();

    let max_val: u16 = rle_vec.iter().max_by_key(|x| x.0).unwrap().0;
    let max_len: u16 = rle_vec.iter().max_by_key(|x| x.1).unwrap().1;
    let max_val_bits: u8 = (16 - max_val.leading_zeros() as u8).max(1);
    let max_len_bits: u8 = 16 - max_len.leading_zeros() as u8;
    let assign_bits: u32 = (max_val_bits + max_len_bits) as u32;
    let n_bytes: u32 = if (assign_bits * rle_vec.len() as u32).is_multiple_of(8) {
        (assign_bits * rle_vec.len() as u32) / 8
    } else {
        (assign_bits * rle_vec.len() as u32) / 8 + 1
    };

    output_vec.push(max_val_bits);
    output_vec.push(max_len_bits);
    output_vec.extend(n_bytes.to_be_bytes().as_slice());

    let mut remainder: u32 = 0;
    let mut remainder_bits: u8 = 0;

    for (val, len) in rle_vec {
        let mut new_val: u32 = (remainder << max_val_bits) | (val as u32);

        let mut buff: u8;

        let mut n_bits_left: u8 = remainder_bits + max_val_bits;

        while n_bits_left >= 8 {
            n_bits_left -= 8;
            buff = (new_val >> n_bits_left) as u8;
            output_vec.push(buff);
            new_val &= !((0xFFFFFFFF as u32) << n_bits_left);
        }

        new_val = (new_val << max_len_bits) | (len as u32);
        n_bits_left += max_len_bits;

        while n_bits_left >= 8 {
            n_bits_left -= 8;
            buff = (new_val >> n_bits_left) as u8;
            output_vec.push(buff);
            new_val &= !((0xFFFFFFFF as u32) << n_bits_left);
        }

        remainder_bits = n_bits_left;
        remainder = new_val;
    }

    if remainder_bits > 0 {
        let buff = (remainder << (8 - remainder_bits)) as u8;
        output_vec.push(buff);
    }

    output_vec
}
