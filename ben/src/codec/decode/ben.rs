use crate::io::reader::BenDecoder;
use std::io::{self, Read, Write};

/// Decode a single BEN frame payload into run-length encoded assignments.
///
/// This function expects only the packed payload bytes for one BEN frame, not
/// the leading per-frame BEN header.
///
/// # Arguments
///
/// * `reader` - A reader positioned at the packed payload bytes for a single
///   BEN frame.
/// * `max_val_bits` - The number of bits used to encode each label value.
/// * `max_len_bits` - The number of bits used to encode each run length.
/// * `n_bytes` - The number of payload bytes to read from `reader`.
///
/// # Returns
///
/// Returns the decoded run-length encoded assignment vector as `(value, count)`
/// pairs.
pub fn decode_ben_line<R: Read>(
    mut reader: R,
    max_val_bits: u8,
    max_len_bits: u8,
    n_bytes: u32,
) -> io::Result<Vec<(u16, u16)>> {
    let mut assign_bits: Vec<u8> = vec![0; n_bytes as usize];
    reader.read_exact(&mut assign_bits)?;

    let n_assignments: usize =
        (n_bytes as f64 / ((max_val_bits + max_len_bits) as f64 / 8.0)) as usize;
    let mut output_rle: Vec<(u16, u16)> = Vec::with_capacity(n_assignments);

    let mut buffer: u32 = 0;
    let mut n_bits_in_buff: u16 = 0;

    let mut val = 0;
    let mut val_set = false;
    let mut len = 0;
    let mut len_set = false;

    for &byte in &assign_bits {
        buffer |= (byte as u32).to_be() >> n_bits_in_buff;
        n_bits_in_buff += 8;

        if n_bits_in_buff >= max_val_bits as u16 && !val_set {
            val = (buffer >> (32 - max_val_bits)) as u16;

            buffer <<= max_val_bits;
            n_bits_in_buff -= max_val_bits as u16;
            val_set = true;
        }

        if n_bits_in_buff >= max_len_bits as u16 && val_set && !len_set {
            len = (buffer >> (32 - max_len_bits)) as u16;
            buffer <<= max_len_bits;
            n_bits_in_buff -= max_len_bits as u16;
            len_set = true;
        }

        if val_set && len_set {
            if len > 0 {
                output_rle.push((val, len));
            }
            val_set = false;
            len_set = false;
        }

        while n_bits_in_buff >= max_val_bits as u16 + max_len_bits as u16 {
            if n_bits_in_buff >= max_val_bits as u16 && !val_set {
                val = (buffer >> (32 - max_val_bits)) as u16;
                buffer <<= max_val_bits;
                n_bits_in_buff -= max_val_bits as u16;
                val_set = true;
            }

            if n_bits_in_buff >= max_len_bits as u16 && val_set && !len_set {
                len = (buffer >> (32 - max_len_bits)) as u16;
                buffer <<= max_len_bits;
                n_bits_in_buff -= max_len_bits as u16;
                len_set = true;
            }

            if val_set && len_set {
                if len > 0 {
                    output_rle.push((val, len));
                }
                val_set = false;
                len_set = false;
            }
        }
    }

    Ok(output_rle)
}

/// Decode a BEN stream into JSONL assignment records.
///
/// Each decoded sample is written as a JSON object containing an `assignment`
/// vector and a 1-based `sample` index.
///
/// # Arguments
///
/// * `reader` - The input BEN stream, including the 17-byte BEN banner.
/// * `writer` - The destination that will receive one JSON object per decoded
///   sample.
///
/// # Returns
///
/// Returns `Ok(())` after the stream has been fully decoded and written.
pub fn decode_ben_to_jsonl<R: Read, W: Write>(reader: R, writer: W) -> io::Result<()> {
    let mut ben_decoder = BenDecoder::new(reader)?;
    ben_decoder.write_all_jsonl(writer)
}
