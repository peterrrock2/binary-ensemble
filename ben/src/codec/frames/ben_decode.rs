use super::BenDecode;
use byteorder::{BigEndian, ReadBytesExt};
use std::io;

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

impl BenDecode for BenDecodeFrame {
    /// Read the next Standard BEN frame from the stream.
    ///
    /// Standard BEN frames have no trailing count; `count` is always set to 1.
    /// Returns `Ok(None)` on a clean EOF at a frame boundary.
    fn from_reader(reader: &mut impl io::Read) -> io::Result<Option<Self>> {
        let max_val_bit_count = match reader.read_u8() {
            Ok(v) => v,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        };

        let max_len_bit_count = reader.read_u8()?;
        let n_bytes = reader.read_u32::<BigEndian>()?;

        let mut raw_bytes = vec![0u8; n_bytes as usize];
        reader.read_exact(&mut raw_bytes)?;

        Ok(Some(BenDecodeFrame {
            max_val_bit_count,
            max_len_bit_count,
            n_bytes,
            raw_bytes,
        }))
    }
}
