/// Canonical representation of a TwoDelta frame.
///
/// A TwoDelta frame stores the two assignment ids that may change relative to
/// the previous sample and then encodes the lengths of alternating runs over
/// just those two ids. The first run always corresponds to `pair.0`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwoDeltaEncodeFrame {
    // The pair of assignment ids that are encoded in this frame, stored here for reference.
    // Canonically, `pair.0` is the id for the first run in the run-length vector and `pair.1`
    // is the id for the second run.
    pub pair: (u16, u16),
    // The number of bits used to encode the maximum run length in this frame.
    pub max_len_bit_count: u8,
    // The number of bytes in the packed payload.
    pub n_bytes: u32,
    // The run-length vector that was encoded into this frame, stored here for reference.
    pub run_length_vector: Vec<u16>,
    // The full serialized TwoDelta frame bytes, including the header, payload, and count.
    pub raw_bytes: Vec<u8>,
    // The number of times this frame is repeated. Mirrors the trailing u16 in `raw_bytes`.
    pub count: u16,
}

impl TwoDeltaEncodeFrame {
    /// Borrow just the packed payload bytes.
    pub fn payload(&self) -> &[u8] {
        &self.raw_bytes[9..9 + self.n_bytes as usize]
    }

    /// Borrow the serialized TwoDelta frame bytes.
    pub fn as_slice(&self) -> &[u8] {
        &self.raw_bytes
    }

    /// Clone out the serialized TwoDelta frame bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.raw_bytes.clone()
    }

    /// Consume the frame and return the serialized bytes without cloning.
    pub fn into_bytes(self) -> Vec<u8> {
        self.raw_bytes
    }

    /// Build a TwoDelta frame by packing a run-length vector into the binary format.
    ///
    /// Run lengths are packed at `max_len_bit_count` bits per value (the minimum
    /// bit width needed to represent the largest run length), MSB-first with no
    /// padding between values. If the total bit count is not a multiple of 8, the
    /// final byte is zero-padded on the right.
    ///
    /// The serialized layout is:
    /// ```text
    /// [pair.0: u16 BE][pair.1: u16 BE][max_len_bit_count: u8][n_bytes: u32 BE][payload...][count: u16 BE]
    /// ```
    /// where the payload is the bit-packed run lengths.
    ///
    /// # Arguments
    ///
    /// * `pair` - The ordered pair of assignment ids. `pair.0` corresponds to the first run.
    /// * `run_length_vector` - The lengths of alternating runs of `pair.0` and `pair.1`
    ///   over the positions occupied by the pair, in position order.
    ///
    /// # Returns
    ///
    /// A fully serialized `TwoDeltaEncodeFrame` with both the packed `raw_bytes` and the
    /// original `run_length_vector` stored on the struct.
    pub fn from_run_lengths(
        pair: (u16, u16),
        run_length_vector: Vec<u16>,
        count: Option<u16>,
    ) -> Self {
        let count = match count {
            Some(v) => v,
            None => 1,
        };

        let max_len = run_length_vector.iter().copied().max().unwrap_or(0);
        let max_len_bit_count = (16 - max_len.leading_zeros() as u8).max(1);

        let payload_bits = max_len_bit_count as u32 * run_length_vector.len() as u32;
        let n_bytes = payload_bits.div_ceil(8);

        // pair_bytes (4) + max_len_bit_count (1) + n_bytes (4) + payload (n_bytes)
        let mut raw_bytes = Vec::with_capacity((n_bytes + 9) as usize);
        raw_bytes.extend_from_slice(&pair.0.to_be_bytes());
        raw_bytes.extend_from_slice(&pair.1.to_be_bytes());
        raw_bytes.push(max_len_bit_count);
        raw_bytes.extend_from_slice(&n_bytes.to_be_bytes());

        let mut remainder: u32 = 0;
        let mut remainder_bits: u8 = 0;

        for &item in &run_length_vector {
            let mut packed = (remainder << max_len_bit_count) | item as u32;
            let mut bits_left = remainder_bits + max_len_bit_count;

            while bits_left >= 8 {
                bits_left -= 8;
                raw_bytes.push((packed >> bits_left) as u8);
                packed &= !((u32::MAX) << bits_left);
            }

            remainder = packed;
            remainder_bits = bits_left;
        }

        if remainder_bits > 0 {
            raw_bytes.push((remainder << (8 - remainder_bits)) as u8);
        }

        raw_bytes.extend(count.to_be_bytes());

        Self {
            pair,
            max_len_bit_count,
            n_bytes,
            run_length_vector,
            raw_bytes,
            count,
        }
    }

    /// Reconstruct a TwoDelta frame from already-parsed header fields and a raw payload.
    ///
    /// This is the inverse of `from_run_lengths`: it re-assembles the serialized bytes
    /// and decodes the bit-packed payload back into the run-length vector so that both
    /// representations are available on the resulting frame.
    ///
    /// The decoding reads `max_len_bit_count` bits at a time from the payload, MSB-first,
    /// and discards any trailing zero-valued items produced by right-padding in the final byte.
    ///
    /// # Arguments
    ///
    /// * `pair` - The ordered pair of assignment ids as read from the frame header.
    /// * `max_len_bit_count` - The bit width of each packed run length, as read from the
    ///   frame header.
    /// * `payload` - The raw packed payload bytes, not including the 9-byte header.
    /// * `count` - The repetition count for the frame, as read from the trailing `u16`
    ///   in the wire format.
    ///
    /// # Returns
    ///
    /// A `TwoDeltaEncodeFrame` with `raw_bytes` (header + payload + count), the decoded
    /// `run_length_vector`, and `count` populated.
    pub fn from_parts(
        pair: (u16, u16),
        max_len_bit_count: u8,
        payload: Vec<u8>,
        count: u16,
    ) -> Self {
        let n_bytes = payload.len() as u32;
        let mut raw_bytes = Vec::with_capacity(9 + payload.len() + 2);
        raw_bytes.extend_from_slice(&pair.0.to_be_bytes());
        raw_bytes.extend_from_slice(&pair.1.to_be_bytes());
        raw_bytes.push(max_len_bit_count);
        raw_bytes.extend_from_slice(&n_bytes.to_be_bytes());
        raw_bytes.extend_from_slice(&payload);
        raw_bytes.extend_from_slice(&count.to_be_bytes());

        let mut run_length_vector = Vec::new();
        let mut buffer: u32 = 0;
        let mut n_bits_in_buff: u16 = 0;

        for &byte in payload[..n_bytes as usize].iter() {
            buffer |= (byte as u32).to_be() >> n_bits_in_buff;
            n_bits_in_buff += 8;

            while n_bits_in_buff >= max_len_bit_count as u16 {
                let item = (buffer >> (32 - max_len_bit_count)) as u16;
                buffer <<= max_len_bit_count;
                n_bits_in_buff -= max_len_bit_count as u16;
                if item > 0 {
                    run_length_vector.push(item);
                }
            }
        }

        Self {
            pair,
            max_len_bit_count,
            n_bytes,
            run_length_vector,
            raw_bytes,
            count,
        }
    }
}

impl AsRef<[u8]> for TwoDeltaEncodeFrame {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl std::ops::Deref for TwoDeltaEncodeFrame {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}
