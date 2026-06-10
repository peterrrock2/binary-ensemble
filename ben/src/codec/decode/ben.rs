use std::io::{self, Read};

/// Upper bound on `n_bytes` accepted by [`decode_ben_line`] and by the frame readers in
/// [`crate::codec::BenDecodeFrame`]. A frame larger than this is rejected without allocating, so
/// malformed or adversarial input cannot OOM the process during fuzzing or stream decoding. The
/// cap is well above any legitimate BEN frame: at 64 MiB of packed RLE data it would hold tens of
/// millions of run pairs.
pub(crate) const MAX_FRAME_PAYLOAD_BYTES: u32 = 1 << 26;

/// Upper bound on the *speculative* run-pair reservation made before decoding a frame payload.
/// The header-derived pair count is attacker-controlled: a minimum-width frame at the payload cap
/// implies ~268 million pairs (≈1 GiB) before a single payload byte has been read. Legitimate
/// frames rarely exceed a few hundred thousand runs, and `Vec` growth covers any that do, so the
/// reservation is clamped and a hostile header costs kilobytes instead of a gigabyte.
const MAX_RLE_PREALLOC_PAIRS: usize = 1 << 16;

/// Decode a single BEN frame payload into run-length encoded assignments.
///
/// This function expects only the packed payload bytes for one BEN frame, not the leading per-frame
/// BEN header.
///
/// # Arguments
///
/// * `reader` - A reader positioned at the packed payload bytes for a single BEN frame.
/// * `max_val_bits` - The number of bits used to encode each label value.
/// * `max_len_bits` - The number of bits used to encode each run length.
/// * `n_bytes` - The number of payload bytes to read from `reader`.
///
/// # Returns
///
/// Returns the decoded run-length encoded assignment vector as `(value, count)` pairs.
///
/// # Errors
///
/// Returns [`io::ErrorKind::InvalidData`] for the following corrupt-frame conditions, which the
/// library writers never produce:
///
/// - `max_val_bits` or `max_len_bits` outside `1..=16`.
/// - `n_bytes` larger than [`MAX_FRAME_PAYLOAD_BYTES`].
/// - A decoded pair with a zero-length run before the trailing padding region.
/// - `n_bytes` not equal to `ceil(real_pairs * (mvb + mlb) / 8)` after decoding (the encoder uses
///   `div_ceil` to compute `n_bytes`, so any other value indicates a malformed or maliciously
///   crafted frame).
/// - The sum of the run lengths exceeding [`super::MAX_ASSIGNMENT_LEN`], so a small frame cannot
///   demand a multi-gigabyte expansion when the runs are later materialized.
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

    if n_bytes > MAX_FRAME_PAYLOAD_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "BEN frame payload of {n_bytes} bytes exceeds {MAX_FRAME_PAYLOAD_BYTES}; \
                 refusing to allocate"
            ),
        ));
    }

    let mut assign_bits: Vec<u8> = vec![0; n_bytes as usize];
    reader.read_exact(&mut assign_bits)?;

    // Bit-width invariants the encoder maintains. Width values themselves are bounded by 16 each
    // (checked above), so the sum fits in u32 trivially and the per-pair extraction below stays
    // within the 32-bit shift register.
    let bit_width = u64::from(max_val_bits) + u64::from(max_len_bits);
    let total_bits = u64::from(n_bytes) * 8;
    let n_assignments_upper_bound = (total_bits / bit_width) as usize;
    let mut output_rle: Vec<(u16, u16)> =
        Vec::with_capacity(n_assignments_upper_bound.min(MAX_RLE_PREALLOC_PAIRS));

    let mut buffer: u32 = 0;
    let mut n_bits_in_buff: u16 = 0;

    let mut val = 0;
    let mut val_set = false;
    let mut len = 0;
    let mut len_set = false;

    // Tracks zero-length pairs seen since the last real (len > 0) pair. The encoder never emits
    // zero-length runs, so any zero-length pair in the decoded stream is either trailing padding
    // (for narrow bit widths, where padding bits may form a complete pair) or a corrupt-frame
    // signal. We accumulate them until either (a) the frame ends — accepted as padding — or
    // (b) a real pair follows — rejected as interior corruption.
    let mut pending_zero_pairs: usize = 0;

    for &byte in &assign_bits {
        // Place the incoming byte at the top of the 32-bit shift register, below any bits already
        // buffered. The explicit shift is endian-independent; bit extraction below always reads
        // from the register's high end.
        buffer |= ((byte as u32) << 24) >> n_bits_in_buff;
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
            if len == 0 {
                pending_zero_pairs += 1;
            } else {
                if pending_zero_pairs > 0 {
                    return Err(interior_zero_length_run_error());
                }
                output_rle.push((val, len));
            }
            val_set = false;
            len_set = false;
        }

        // The while condition guarantees enough bits for a complete (val, len) pair. len_set is
        // always false on entry (reset by the outer for body above), so we extract len
        // unconditionally.
        while n_bits_in_buff >= max_val_bits as u16 + max_len_bits as u16 {
            if !val_set {
                val = (buffer >> (32 - max_val_bits)) as u16;
                buffer <<= max_val_bits;
                n_bits_in_buff -= max_val_bits as u16;
            }

            len = (buffer >> (32 - max_len_bits)) as u16;
            buffer <<= max_len_bits;
            n_bits_in_buff -= max_len_bits as u16;

            if len == 0 {
                pending_zero_pairs += 1;
            } else {
                if pending_zero_pairs > 0 {
                    return Err(interior_zero_length_run_error());
                }
                output_rle.push((val, len));
            }
            val_set = false;
        }
    }

    // n_bytes consistency: the encoder writes `n_bytes = ceil(real_pairs * bit_width / 8)`. Any
    // other relationship between n_bytes and the number of real pairs we recovered is a
    // corrupt-frame signal (n_bytes overstated → extra "phantom" capacity the encoder wouldn't
    // allocate; n_bytes understated → real pairs would have been truncated).
    let real_pairs = output_rle.len() as u64;
    let expected_bytes = (real_pairs * bit_width).div_ceil(8);
    if u64::from(n_bytes) != expected_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "inconsistent BEN frame size: n_bytes={n_bytes} but {real_pairs} pair(s) at \
                 {bit_width} bit(s)/pair require {expected_bytes} byte(s)"
            ),
        ));
    }

    // Expansion sanity bound: callers materialize the runs into a full assignment vector, so the
    // sum of the run lengths is the allocation a frame can demand. Reject absurd sums here, before
    // any caller pays for the expansion.
    let expanded_len: u64 = output_rle.iter().map(|&(_, len)| u64::from(len)).sum();
    if expanded_len > super::MAX_ASSIGNMENT_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "BEN frame expands to {expanded_len} elements, which exceeds the \
                 {} sanity bound",
                super::MAX_ASSIGNMENT_LEN
            ),
        ));
    }

    Ok(output_rle)
}

fn interior_zero_length_run_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "BEN frame contains an interior zero-length run; the encoder never emits zero-length runs",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn decode_ben_line_rejects_zero_length_run_when_trailing_real_pair_present() {
        // Hand-built frame: mvb=4, mlb=4 (bit_width=8 = one full byte per pair). First byte
        // 0x10 = (val=1, len=0) — zero-length, should not exist. Second byte 0x23 = (val=2,
        // len=3). The trailing real pair makes the leading zero-length pair "interior", which is
        // rejected.
        let err =
            decode_ben_line(Cursor::new(&[0x10u8, 0x23u8]), 4, 4, 2).expect_err("must reject");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("interior zero-length"));
    }

    #[test]
    fn decode_ben_line_rejects_inconsistent_n_bytes() {
        // Plan's headline case: mvb=8, mlb=8 → 16 bits/pair = 2 bytes/pair. n_bytes=3 should
        // decode 1 pair but leaves a full byte of "padding" — the encoder uses div_ceil(2*16/8)=2,
        // never 3. The post-decode consistency check rejects this.
        let err = decode_ben_line(Cursor::new(&[0x01u8, 0x03u8, 0xff]), 8, 8, 3)
            .expect_err("must reject");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("inconsistent"));
    }

    #[test]
    fn decode_ben_line_grows_past_the_clamped_preallocation() {
        use crate::codec::BenEncodeFrame;
        use crate::BenVariant;
        // 100,000 pairs sit above MAX_RLE_PREALLOC_PAIRS, so the output vector must grow past its
        // clamped initial reservation without losing or reordering pairs.
        let runs = vec![(1u16, 1u16); 100_000];
        let frame = BenEncodeFrame::from_rle(runs.clone(), BenVariant::Standard, None);
        let decoded = decode_ben_line(
            Cursor::new(frame.payload()),
            frame.max_val_bit_count().unwrap(),
            frame.max_len_bit_count(),
            frame.n_bytes(),
        )
        .unwrap();
        assert_eq!(decoded, runs);
    }

    #[test]
    fn decode_ben_line_rejects_oversized_expansion() {
        use crate::codec::BenEncodeFrame;
        use crate::BenVariant;
        // 2049 runs of 65,535 elements expand past the 2^27 sanity bound; each run is
        // individually legal, so only the bound on the sum catches this.
        let frame =
            BenEncodeFrame::from_rle(vec![(1u16, u16::MAX); 2049], BenVariant::Standard, None);
        let err = decode_ben_line(
            Cursor::new(frame.payload()),
            frame.max_val_bit_count().unwrap(),
            frame.max_len_bit_count(),
            frame.n_bytes(),
        )
        .expect_err("must reject");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("sanity bound"));
    }

    #[test]
    fn decode_ben_line_rejects_oversized_n_bytes_without_allocating() {
        // n_bytes way above the sanity cap must error before allocating. We don't supply any
        // bytes here because the cap check fires first; read_exact would otherwise try to fill
        // ~4GiB.
        let err = decode_ben_line(Cursor::new(&[][..]), 8, 8, u32::MAX).expect_err("must reject");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("exceeds"));
    }

    #[test]
    fn decode_ben_line_accepts_narrow_bit_width_with_trailing_zero_padding() {
        // mvb=1, mlb=1, n_bytes=1, single real pair (1, 1) at the high bits. The remaining 6 bits
        // are zero, which the decoder reads as three trailing (0, 0) "phantom" pairs. These are
        // padding artifacts of the byte-aligned wire format and must be accepted.
        let result = decode_ben_line(Cursor::new(&[0b11_00_00_00u8]), 1, 1, 1).unwrap();
        assert_eq!(result, vec![(1u16, 1u16)]);
    }

    #[test]
    #[allow(clippy::unusual_byte_groupings)]
    fn decode_ben_line_accepts_non_byte_aligned_frame() {
        // mvb=2, mlb=3 (bit_width=5), n_bytes=2 (16 bits = 3 real pairs + 1 padding bit). Encoder
        // produces this layout for RLE [(1,4),(2,1),(3,3)]; the consistency check must accept it.
        let result =
            decode_ben_line(Cursor::new(&[0b01100_100u8, 0b01_11011_0u8]), 2, 3, 2).unwrap();
        assert_eq!(result, vec![(1u16, 4u16), (2, 1), (3, 3)]);
    }

    #[test]
    fn decode_ben_line_partial_bits_skip_val_len_check() {
        // max_val_bits=8, max_len_bits=8 → each run requires 2 bytes. After byte 1: val_set=true,
        // len_set=false → `if val_set && len_set` is false (the `}` closing that block is the
        // false-path counter in LLVM coverage). After byte 2: both set → run (1, 3) is pushed.
        let result = decode_ben_line(Cursor::new(&[0x01u8, 0x03u8]), 8, 8, 2).unwrap();
        assert_eq!(result, vec![(1u16, 3u16)]);
    }
}
