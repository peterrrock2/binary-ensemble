use crate::BenVariant;
use byteorder::{BigEndian, ReadBytesExt};
use serde_json::json;
use std::io::{self, BufRead, Write};

/// Decode a single ben32 frame into an assignment vector and repetition count.
///
/// This helper is crate-private because ben32 is an implementation detail of
/// XBEN, but it underpins both the stream decoders and the translation logic.
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

/// Decode a ben32 stream into JSONL assignment records.
///
/// # Arguments
///
/// * `reader` - The ben32 input stream.
/// * `writer` - The destination for the JSONL output.
/// * `starting_sample` - The 0-based sample offset that should be added to the
///   emitted sample numbers.
/// * `variant` - The BEN variant used to interpret repetition counts.
///
/// # Returns
///
/// Returns `Ok(())` after the ben32 stream has been fully decoded.
pub(crate) fn jsonl_decode_ben32<R: BufRead, W: Write>(
    mut reader: R,
    mut writer: W,
    starting_sample: usize,
    variant: BenVariant,
) -> io::Result<()> {
    let mut sample_number = 1;
    loop {
        let result = decode_ben32_line(&mut reader, variant);
        if let Err(e) = result {
            if e.kind() == io::ErrorKind::UnexpectedEof {
                return Ok(());
            }
            return Err(e);
        }

        let (output_vec, count) = result.unwrap();

        for _ in 0..count {
            let line = json!({
                "assignment": output_vec,
                "sample": sample_number + starting_sample,
            })
            .to_string()
                + "\n";

            writer.write_all(line.as_bytes())?;
            sample_number += 1;
        }
    }
}
