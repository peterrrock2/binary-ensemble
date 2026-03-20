#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BenDecodeFrame {
    // The number of bits used to encode the maximum label value in this frame.
    pub max_val_bit_count: u8,
    // The number of bits used to encode the maximum run length in this frame.
    pub max_len_bit_count: u8,
    // The number of bytes in the packed payload.
    pub n_bytes: u32,
    // The full serialized BEN frame bytes, including the header and payload.
    pub raw_bytes: Vec<u8>,
    // The number of times this frame was repeated
    pub count: u16,
}

impl BenDecodeFrame {
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

impl AsRef<[u8]> for BenDecodeFrame {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl std::ops::Deref for BenDecodeFrame {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl PartialEq<Vec<u8>> for BenDecodeFrame {
    fn eq(&self, other: &Vec<u8>) -> bool {
        self.raw_bytes == *other
    }
}

impl PartialEq<BenDecodeFrame> for Vec<u8> {
    fn eq(&self, other: &BenDecodeFrame) -> bool {
        *self == other.raw_bytes
    }
}
