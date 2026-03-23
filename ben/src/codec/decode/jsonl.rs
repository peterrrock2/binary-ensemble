use crate::io::reader::{AssignmentReader, XZAssignmentReader};
use crate::{progress, BenVariant};
use crate::codec::decode::jsonl_decode_ben32;
use crate::format::banners::{variant_from_banner, BANNER_LEN};
use crate::format::FormatError;
use serde_json::json;
use std::io::{self, BufRead, BufReader, Read, Write};
use xz2::read::XzDecoder;

/// Decode a BEN stream into JSONL assignment records.
///
/// Each decoded sample is written as a JSON object containing an `assignment`
/// vector and a 1-based `sample` index.
///
/// # Arguments
///
/// * `reader` - The input BEN stream, including the 17-byte BEN banner.
/// * `writer` - The destination that will receive one JSON object per decoded
///   sample.
///
/// # Returns
///
/// Returns `Ok(())` after the stream has been fully decoded and written.
pub fn decode_ben_to_jsonl<R: Read, W: Write>(reader: R, writer: W) -> io::Result<()> {
    let mut ben_decoder = AssignmentReader::new(reader)?;
    ben_decoder.write_all_jsonl(writer)
}

/// Decode an XBEN stream directly into JSONL assignment records.
///
/// # Arguments
///
/// * `reader` - The compressed XBEN input stream.
/// * `writer` - The destination that will receive one JSON object per decoded
///   sample.
///
/// # Returns
///
/// Returns `Ok(())` after the XBEN stream has been fully decoded into JSONL.
pub fn decode_xben_to_jsonl<R: BufRead, W: Write>(reader: R, mut writer: W) -> io::Result<()> {
    let mut decoder = XzDecoder::new(reader);

    let mut first_buffer = [0u8; BANNER_LEN];

    if let Err(e) = decoder.read_exact(&mut first_buffer) {
        return Err(e);
    }

    let variant = match variant_from_banner(&first_buffer) {
        Some(BenVariant::Standard) => BenVariant::Standard,
        Some(BenVariant::MkvChain) => BenVariant::MkvChain,
        Some(BenVariant::TwoDelta) => {
            let mut xben = XZAssignmentReader::from_decompressed_stream(
                BufReader::new(decoder),
                BenVariant::TwoDelta,
            );
            let mut sample_number = 1usize;
            for record in &mut xben {
                let (assignment, count) = record?;
                for _ in 0..count {
                    progress!("Decoding sample: {}\r", sample_number);
                    let line = json!({
                        "assignment": assignment,
                        "sample": sample_number,
                    })
                    .to_string()
                        + "\n";
                    writer.write_all(line.as_bytes())?;
                    sample_number += 1;
                }
            }
            tracing::trace!("");
            tracing::trace!("Done!");
            return Ok(());
        }
        None => {
            return Err(io::Error::from(FormatError::UnknownBanner {
                actual: first_buffer.to_vec(),
            }));
        }
    };

    let mut buffer = [0u8; 1 << 20];
    let mut overflow: Vec<u8> = Vec::new();

    let mut line_count: usize = 0;
    let mut starting_sample: usize = 0;
    while let Ok(count) = decoder.read(&mut buffer) {
        if count == 0 {
            break;
        }

        overflow.extend(&buffer[..count]);

        let mut last_valid_assignment = 0;

        match variant {
            BenVariant::Standard => {
                for i in (3..overflow.len()).step_by(4) {
                    if overflow[i - 3..=i] == [0, 0, 0, 0] {
                        last_valid_assignment = i + 1;
                        line_count += 1;
                        progress!("Decoding sample: {}\r", line_count);
                    }
                }
            }
            BenVariant::MkvChain => {
                for i in (last_valid_assignment + 3..overflow.len().saturating_sub(2)).step_by(2) {
                    if overflow[i - 3..=i] == [0, 0, 0, 0] {
                        last_valid_assignment = i + 3;
                        let lines = &overflow[i + 1..i + 3];
                        let n_lines = u16::from_be_bytes([lines[0], lines[1]]);
                        line_count += n_lines as usize;
                        progress!("Decoding sample: {}\r", line_count);
                    }
                }
            }
            BenVariant::TwoDelta => unreachable!("handled before ben32 decoding"),
        }

        if last_valid_assignment == 0 {
            continue;
        }

        jsonl_decode_ben32(
            &overflow[0..last_valid_assignment],
            &mut writer,
            starting_sample,
            variant,
        )?;
        overflow.drain(..last_valid_assignment);
        starting_sample = line_count;
    }
    tracing::trace!("");
    tracing::trace!("Done!");
    Ok(())
}
