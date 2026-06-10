use crate::BenVariant;
use byteorder::{BigEndian, ReadBytesExt};
use std::io::{self, BufRead};

/// Decode a single ben32 frame into an assignment vector and repetition count.
///
/// This helper is crate-private because ben32 is an implementation detail of XBEN, but it underpins
/// both the stream decoders and the translation logic.
///
/// # Arguments
///
/// * `reader` - A reader positioned at the start of a single ben32 frame.
/// * `variant` - The BEN variant used to interpret the frame tail.
///
/// # Returns
///
/// Returns the expanded assignment vector together with its repetition count.
pub(crate) fn decode_ben32_line<R: BufRead>(
    mut reader: R,
    variant: BenVariant,
) -> io::Result<(Vec<u16>, u16)> {
    let mut buffer = [0u8; 4];
    let mut output_vec: Vec<u16> = Vec::new();

    loop {
        match reader.read_exact(&mut buffer) {
            Ok(()) => {
                let encoded = u32::from_be_bytes(buffer);
                if encoded == 0 {
                    break;
                }

                let value = (encoded >> 16) as u16;
                let count = (encoded & 0xFFFF) as u16;

                for _ in 0..count {
                    output_vec.push(value);
                }
            }
            Err(e) => {
                return Err(e);
            }
        }
    }

    let count = if variant == BenVariant::MkvChain {
        reader.read_u16::<BigEndian>()?
    } else {
        1
    };

    Ok((output_vec, count))
}
