use super::BenDecode;
use byteorder::{BigEndian, ReadBytesExt};
use std::io::{self, Read};

/// A decoded MkvChain BEN frame, including its repetition count.
///
/// Symmetric to `MkvBenEncodeFrame` but stores only the decoded payload bytes
/// and header fields rather than the original RLE runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MkvBenDecodeFrame {
    /// The number of bits used to encode the maximum label value in this frame.
    pub max_val_bit_count: u8,
    /// The number of bits used to encode the maximum run length in this frame.
    pub max_len_bit_count: u8,
    /// The number of bytes in the packed payload.
    pub n_bytes: u32,
    /// The packed payload bytes (not including the 6-byte header or count).
    pub raw_bytes: Vec<u8>,
    /// The number of times this assignment repeats.
    pub count: u16,
}

impl MkvBenDecodeFrame {
    /// Borrow the packed payload bytes.
    pub fn as_slice(&self) -> &[u8] {
        &self.raw_bytes
    }

    /// Clone out the packed payload bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.raw_bytes.clone()
    }

    /// Consume the frame and return the packed payload bytes without cloning.
    pub fn into_bytes(self) -> Vec<u8> {
        self.raw_bytes
    }
}

impl BenDecode for MkvBenDecodeFrame {
    /// Read the next MkvChain BEN frame from the stream.
    ///
    /// MkvChain frames carry a trailing `u16` repetition count.
    /// Returns `Ok(None)` on a clean EOF at a frame boundary.
    fn from_reader(reader: &mut impl Read) -> io::Result<Option<Self>> {
        let max_val_bit_count = match reader.read_u8() {
            Ok(v) => v,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        };

        let max_len_bit_count = reader.read_u8()?;
        let n_bytes = reader.read_u32::<BigEndian>()?;

        let mut raw_bytes = vec![0u8; n_bytes as usize];
        reader.read_exact(&mut raw_bytes)?;

        let count = reader.read_u16::<BigEndian>()?;

        Ok(Some(MkvBenDecodeFrame {
            max_val_bit_count,
            max_len_bit_count,
            n_bytes,
            raw_bytes,
            count,
        }))
    }
}

impl AsRef<[u8]> for MkvBenDecodeFrame {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl std::ops::Deref for MkvBenDecodeFrame {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl PartialEq<Vec<u8>> for MkvBenDecodeFrame {
    fn eq(&self, other: &Vec<u8>) -> bool {
        self.raw_bytes == *other
    }
}

impl PartialEq<MkvBenDecodeFrame> for Vec<u8> {
    fn eq(&self, other: &MkvBenDecodeFrame) -> bool {
        *self == other.raw_bytes
    }
}
