use super::twodelta_encode::TwoDeltaEncodeFrame;
use super::BenDecode;
use byteorder::{BigEndian, ReadBytesExt};
use std::io::{self, Read};

/// A decoded TwoDelta delta frame, containing only what's needed to apply the delta.
///
/// Unlike `TwoDeltaEncodeFrame`, this type does not retain raw bytes or
/// bit-packing metadata. It delegates bit-unpacking of the run lengths to
/// `TwoDeltaEncodeFrame::from_parts` and then discards everything except
/// `pair`, `run_lengths`, and `count`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwoDeltaDecodeFrame {
    /// The ordered pair of district ids involved in the delta.
    pub pair: (u16, u16),
    /// The unpacked run-length vector over the positions occupied by the pair.
    pub run_lengths: Vec<u16>,
    /// The number of times this delta repeats.
    pub count: u16,
}

impl BenDecode for TwoDeltaDecodeFrame {
    /// Read the next TwoDelta delta frame from the stream.
    ///
    /// Reads pair, max_len_bits, n_bytes, payload, and count, then delegates
    /// bit-unpacking to `TwoDeltaEncodeFrame::from_parts`.
    /// Returns `Ok(None)` on a clean EOF at a frame boundary.
    fn from_reader(reader: &mut impl Read) -> io::Result<Option<Self>> {
        let pair_a = match reader.read_u16::<BigEndian>() {
            Ok(v) => v,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        };

        let pair_b = reader.read_u16::<BigEndian>()?;
        let max_len_bits = reader.read_u8()?;
        if max_len_bits == 0 || max_len_bits > 16 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid TwoDelta run-length bit width: {max_len_bits}"),
            ));
        }
        let n_bytes = reader.read_u32::<BigEndian>()?;

        let mut payload = vec![0u8; n_bytes as usize];
        reader.read_exact(&mut payload)?;

        let count = reader.read_u16::<BigEndian>()?;

        let encode_frame =
            TwoDeltaEncodeFrame::from_parts((pair_a, pair_b), max_len_bits, payload, count);

        Ok(Some(TwoDeltaDecodeFrame {
            pair: encode_frame.pair,
            run_lengths: encode_frame.run_length_vector,
            count,
        }))
    }
}
