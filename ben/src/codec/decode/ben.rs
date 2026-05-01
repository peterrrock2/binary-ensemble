use std::io::{self, Read};

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
    if max_val_bits == 0 || max_val_bits > 16 || max_len_bits == 0 || max_len_bits > 16 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "invalid BEN bit width(s): max_val_bits={max_val_bits}, max_len_bits={max_len_bits}"
            ),
        ));
    }

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

        // The while condition guarantees enough bits for a complete (val, len) pair.
        // len_set is always false on entry (reset by the outer for body above),
        // so we extract len unconditionally.
        while n_bits_in_buff >= max_val_bits as u16 + max_len_bits as u16 {
            if !val_set {
                val = (buffer >> (32 - max_val_bits)) as u16;
                buffer <<= max_val_bits;
                n_bits_in_buff -= max_val_bits as u16;
            }

            len = (buffer >> (32 - max_len_bits)) as u16;
            buffer <<= max_len_bits;
            n_bits_in_buff -= max_len_bits as u16;

            if len > 0 {
                output_rle.push((val, len));
            }
            val_set = false;
        }
    }

    Ok(output_rle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn decode_ben_line_skips_zero_length_run() {
        // max_val_bits=1, max_len_bits=1, 1 byte payload = 0x80.
        // Bit layout: [val=1][len=0] → run with len=0 is not pushed.
        let result = decode_ben_line(Cursor::new(&[0x80u8]), 1, 1, 1).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn decode_ben_line_partial_bits_skip_val_len_check() {
        // max_val_bits=8, max_len_bits=8 → each run requires 2 bytes.
        // After byte 1: val_set=true, len_set=false → `if val_set && len_set`
        // is false (the `}` closing that block is the false-path counter in
        // LLVM coverage).
        // After byte 2: both set → run (1, 3) is pushed.
        let result = decode_ben_line(Cursor::new(&[0x01u8, 0x03u8]), 8, 8, 2).unwrap();
        assert_eq!(result, vec![(1u16, 3u16)]);
    }
}
