use crate::BenVariant;
use byteorder::{BigEndian, ReadBytesExt};
use serde_json::json;
use std::io::{self, BufRead, Write};

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
        reader
            .read_u16::<BigEndian>()
            .expect("Error when reading sample.")
    } else {
        1
    };

    Ok((output_vec, count))
}

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
