use super::{compress_rle_to_ben_bytes, BenConstruct};

/// Canonical representation of a BEN frame.
///
/// The frame stores the semantic RLE runs together with the derived header
/// fields and the serialized frame bytes. `to_bytes()` returns the full BEN
/// frame, including the two one-byte bit-width fields and the four-byte payload
/// length.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MkvBenEncodeFrame {
    // The RLE runs that were encoded into this frame, stored here for reference
    pub runs: Vec<(u16, u16)>,
    // The number of bits used to encode the maximum label value in this frame.
    pub max_val_bit_count: u8,
    // The number of bits used to encode the maximum run length in this frame.
    pub max_len_bit_count: u8,
    // The number of bytes in the packed payload.
    pub n_bytes: u32,
    // The full serialized MKVBEN frame bytes, including the header and payload.
    pub raw_bytes: Vec<u8>,
    // The number of times that this frame was repeated
    pub count: u16,
}

impl MkvBenEncodeFrame {
    /// Borrow the serialized BEN frame bytes.
    pub fn as_slice(&self) -> &[u8] {
        &self.raw_bytes
    }

    /// Clone out the serialized BEN frame bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.raw_bytes.clone()
    }

    /// Consume the frame and return the serialized BEN bytes without cloning.
    pub fn into_bytes(self) -> Vec<u8> {
        self.raw_bytes
    }
}

impl AsRef<[u8]> for MkvBenEncodeFrame {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl std::ops::Deref for MkvBenEncodeFrame {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl PartialEq<Vec<u8>> for MkvBenEncodeFrame {
    fn eq(&self, other: &Vec<u8>) -> bool {
        self.raw_bytes == *other
    }
}

impl PartialEq<MkvBenEncodeFrame> for Vec<u8> {
    fn eq(&self, other: &MkvBenEncodeFrame) -> bool {
        *self == other.raw_bytes
    }
}

impl BenConstruct for MkvBenEncodeFrame {
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
            compress_rle_to_ben_bytes(max_val_bit_count, max_len_bit_count, n_bytes, &runs);

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
