/// Canonical representation of a BEN frame.
///
/// The frame stores the semantic RLE runs together with the derived header
/// fields and the serialized frame bytes. `to_bytes()` returns the full BEN
/// frame, including the two one-byte bit-width fields and the four-byte payload
/// length.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BenEncodeFrame {
    // The RLE runs that were encoded into this frame, stored here for reference
    pub runs: Vec<(u16, u16)>,
    // The number of bits used to encode the maximum label value in this frame.
    pub max_val_bit_count: u8,
    // The number of bits used to encode the maximum run length in this frame.
    pub max_len_bit_count: u8,
    // The number of bytes in the packed payload.
    pub n_bytes: u32,
    // The full serialized BEN frame bytes, including the header and payload.
    pub raw_bytes: Vec<u8>,
}

impl BenEncodeFrame {
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

impl AsRef<[u8]> for BenEncodeFrame {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl std::ops::Deref for BenEncodeFrame {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl PartialEq<Vec<u8>> for BenEncodeFrame {
    fn eq(&self, other: &Vec<u8>) -> bool {
        self.raw_bytes == *other
    }
}

impl PartialEq<BenEncodeFrame> for Vec<u8> {
    fn eq(&self, other: &BenEncodeFrame) -> bool {
        *self == other.raw_bytes
    }
}

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
    pub count: usize,
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

/// Canonical representation of a TwoDelta frame.
///
/// A TwoDelta frame stores the two assignment ids that may change relative to
/// the previous sample and then encodes the lengths of alternating runs over
/// just those two ids. The first run always corresponds to `pair.0`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwoDeltaFrame {
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
    // The full serialized TwoDelta frame bytes, including the header and payload.
    pub raw_bytes: Vec<u8>,
}

impl TwoDeltaFrame {
    /// Borrow just the packed payload bytes.
    pub fn payload(&self) -> &[u8] {
        &self.raw_bytes[9..]
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
}

impl AsRef<[u8]> for TwoDeltaFrame {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl std::ops::Deref for TwoDeltaFrame {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}
