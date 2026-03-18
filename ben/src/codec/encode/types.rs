use std::io;

/// Typed identifier storage used by experimental delta encoders.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IdVec {
    U8(Vec<u8>),
    U16(Vec<u16>),
}

/// A single typed identifier item.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Eq, Hash)]
pub enum IdItem {
    U8(u8),
    U16(u16),
}

impl IdVec {
    /// Borrow the inner `u8` bytes.
    pub fn as_slice(&self) -> &[u8] {
        self.as_u8_slice().expect("expected U8-encoded payload")
    }

    /// Borrow the inner `u8` bytes, returning an error on variant mismatch.
    pub fn as_u8_slice(&self) -> io::Result<&[u8]> {
        match self {
            IdVec::U8(v) => Ok(v.as_slice()),
            IdVec::U16(_) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "expected U8-encoded payload",
            )),
        }
    }

    /// Consume into raw `u8` bytes.
    pub fn into_u8_vec(self) -> io::Result<Vec<u8>> {
        match self {
            IdVec::U8(v) => Ok(v),
            IdVec::U16(_) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "expected U8-encoded payload",
            )),
        }
    }

    /// Return the logical element count.
    pub fn len(&self) -> usize {
        match self {
            IdVec::U8(v) => v.len(),
            IdVec::U16(v) => v.len(),
        }
    }

    /// Return whether the container is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Iterate over items while preserving the original scalar type.
    pub fn iter(&self) -> impl Iterator<Item = IdItem> + '_ {
        match self {
            IdVec::U8(v) => {
                Box::new(v.iter().copied().map(IdItem::U8)) as Box<dyn Iterator<Item = IdItem>>
            }
            IdVec::U16(v) => Box::new(v.iter().copied().map(IdItem::U16)),
        }
    }

    /// Return the item at index `i`, if any.
    pub fn get(&self, i: usize) -> Option<IdItem> {
        match self {
            IdVec::U8(v) => v.get(i).copied().map(IdItem::U8),
            IdVec::U16(v) => v.get(i).copied().map(IdItem::U16),
        }
    }
}

impl<'a> IntoIterator for &'a IdVec {
    type Item = IdItem;
    type IntoIter = Box<dyn Iterator<Item = IdItem> + 'a>;

    fn into_iter(self) -> Self::IntoIter {
        Box::new(self.iter())
    }
}

impl AsRef<[u8]> for IdVec {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl std::ops::Deref for IdVec {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl PartialEq<Vec<u8>> for IdVec {
    fn eq(&self, other: &Vec<u8>) -> bool {
        matches!(self, IdVec::U8(v) if v == other)
    }
}

impl PartialEq<IdVec> for Vec<u8> {
    fn eq(&self, other: &IdVec) -> bool {
        matches!(other, IdVec::U8(v) if self == v)
    }
}

/// Pack a slice of items into a byte vector using a fixed bit width per item.
///
/// # Arguments
///
/// * `items` - The values to pack.
/// * `item_bits` - The number of bits used to encode each item.
///
/// # Returns
///
/// Returns the payload length in bytes and the packed byte vector.
fn pack_fixed_width_items(items: &[u16], item_bits: u8) -> (u32, Vec<u8>) {
    let payload_bits = item_bits as u32 * items.len() as u32;
    let n_bytes = payload_bits.div_ceil(8);
    let mut bytes = Vec::with_capacity(n_bytes as usize);

    let mut remainder: u32 = 0;
    let mut remainder_bits: u8 = 0;

    for &item in items {
        let mut packed = (remainder << item_bits) | item as u32;
        let mut bits_left = remainder_bits + item_bits;

        while bits_left >= 8 {
            bits_left -= 8;
            bytes.push((packed >> bits_left) as u8);
            packed &= !((u32::MAX) << bits_left);
        }

        remainder = packed;
        remainder_bits = bits_left;
    }

    if remainder_bits > 0 {
        bytes.push((remainder << (8 - remainder_bits)) as u8);
    }

    (n_bytes, bytes)
}

/// Canonical representation of a BEN frame.
///
/// The frame stores the semantic RLE runs together with the derived header
/// fields and the serialized frame bytes. `to_bytes()` returns the full BEN
/// frame, including the two one-byte bit-width fields and the four-byte payload
/// length.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BenFrame {
    pub runs: Vec<(u16, u16)>,
    pub max_val_bits: u8,
    pub max_len_bits: u8,
    pub n_bytes: u32,
    bytes: Vec<u8>,
}

impl BenFrame {
    /// Build a frame from an RLE run vector.
    pub fn from_rle(runs: Vec<(u16, u16)>) -> Self {
        let (max_val, max_len) = runs
            .iter()
            .fold((0u16, 0u16), |(max_val, max_len), &(val, len)| {
                (max_val.max(val), max_len.max(len))
            });
        let max_val_bits = (16 - max_val.leading_zeros() as u8).max(1);
        let max_len_bits = (16 - max_len.leading_zeros() as u8).max(1);
        let assign_bits = (max_val_bits + max_len_bits) as u32;
        let payload_bits = assign_bits * runs.len() as u32;
        let n_bytes = payload_bits.div_ceil(8);

        let mut bytes = Vec::with_capacity(6 + n_bytes as usize);
        bytes.push(max_val_bits);
        bytes.push(max_len_bits);
        bytes.extend_from_slice(&n_bytes.to_be_bytes());

        let mut remainder: u32 = 0;
        let mut remainder_bits: u8 = 0;

        for &(val, len) in &runs {
            let mut packed = (remainder << max_val_bits) | (val as u32);
            let mut bits_left = remainder_bits + max_val_bits;

            while bits_left >= 8 {
                bits_left -= 8;
                bytes.push((packed >> bits_left) as u8);
                packed &= !((u32::MAX) << bits_left);
            }

            packed = (packed << max_len_bits) | (len as u32);
            bits_left += max_len_bits;

            while bits_left >= 8 {
                bits_left -= 8;
                bytes.push((packed >> bits_left) as u8);
                packed &= !((u32::MAX) << bits_left);
            }

            remainder = packed;
            remainder_bits = bits_left;
        }

        if remainder_bits > 0 {
            bytes.push((remainder << (8 - remainder_bits)) as u8);
        }

        Self {
            runs,
            max_val_bits,
            max_len_bits,
            n_bytes,
            bytes,
        }
    }

    /// Build a frame from a full assignment vector.
    pub fn from_assignment(assignments: impl AsRef<[u16]>) -> Self {
        Self::from_rle(crate::util::rle::assign_to_rle(assignments))
    }

    /// Borrow the serialized BEN frame bytes.
    pub fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    /// Clone out the serialized BEN frame bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.bytes.clone()
    }

    /// Consume the frame and return the serialized BEN bytes without cloning.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

impl AsRef<[u8]> for BenFrame {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl std::ops::Deref for BenFrame {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl PartialEq<Vec<u8>> for BenFrame {
    fn eq(&self, other: &Vec<u8>) -> bool {
        self.bytes == *other
    }
}

impl PartialEq<BenFrame> for Vec<u8> {
    fn eq(&self, other: &BenFrame) -> bool {
        *self == other.bytes
    }
}

/// Canonical representation of a TwoDelta frame.
///
/// A TwoDelta frame stores the two assignment ids that may change relative to
/// the previous sample and then encodes the lengths of alternating runs over
/// just those two ids. The first run always corresponds to `pair.0`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwoDeltaFrame {
    pub pair: (u16, u16),
    pub max_len_bits: u8,
    pub n_bytes: u32,
    bytes: Vec<u8>,
}

impl TwoDeltaFrame {
    /// Build a TwoDelta frame from a pair ordering and run lengths.
    pub fn from_run_lengths(pair: (u16, u16), run_lengths: Vec<u16>) -> Self {
        let max_len = run_lengths.iter().copied().max().unwrap_or(0);
        let max_len_bits = (16 - max_len.leading_zeros() as u8).max(1);
        let (n_bytes, payload_bytes) = pack_fixed_width_items(&run_lengths, max_len_bits);

        let mut bytes = Vec::with_capacity(9 + payload_bytes.len());
        bytes.extend_from_slice(&pair.0.to_be_bytes());
        bytes.extend_from_slice(&pair.1.to_be_bytes());
        bytes.push(max_len_bits);
        bytes.extend_from_slice(&n_bytes.to_be_bytes());
        bytes.extend_from_slice(&payload_bytes);

        Self {
            pair,
            max_len_bits,
            n_bytes,
            bytes,
        }
    }

    /// Rebuild a TwoDelta frame from already-parsed header fields and payload bytes.
    pub fn from_parts(pair: (u16, u16), max_len_bits: u8, payload: Vec<u8>) -> Self {
        let n_bytes = payload.len() as u32;
        let mut bytes = Vec::with_capacity(9 + payload.len());
        bytes.extend_from_slice(&pair.0.to_be_bytes());
        bytes.extend_from_slice(&pair.1.to_be_bytes());
        bytes.push(max_len_bits);
        bytes.extend_from_slice(&n_bytes.to_be_bytes());
        bytes.extend_from_slice(&payload);

        Self {
            pair,
            max_len_bits,
            n_bytes,
            bytes,
        }
    }

    /// Borrow just the packed payload bytes.
    pub fn payload(&self) -> &[u8] {
        &self.bytes[9..]
    }

    /// Borrow the serialized TwoDelta frame bytes.
    pub fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    /// Clone out the serialized TwoDelta frame bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.bytes.clone()
    }

    /// Consume the frame and return the serialized bytes without cloning.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
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
