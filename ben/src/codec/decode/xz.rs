use crate::codec::decode::jsonl_decode_ben32;
use crate::codec::translate::ben32_to_ben_lines;
use crate::io::reader::XBenDecoder;
use crate::io::writer::BenEncoder;
use crate::{progress, BenVariant};
use serde_json::json;
use std::io::{self, BufRead, BufReader, Error, Read, Write};
use xz2::read::XzDecoder;

/// Decode an XBEN stream into an equivalent BEN stream.
///
/// The output begins with the normal BEN banner followed by uncompressed BEN
/// frames.
///
/// # Arguments
///
/// * `reader` - The compressed XBEN input stream.
/// * `writer` - The destination for the uncompressed BEN stream.
///
/// # Returns
///
/// Returns `Ok(())` after the full XBEN stream has been decoded into BEN.
pub fn decode_xben_to_ben<R: BufRead, W: Write>(reader: R, mut writer: W) -> io::Result<()> {
    let mut decoder = XzDecoder::new(reader);

    let mut first_buffer = [0u8; 17];

    if let Err(e) = decoder.read_exact(&mut first_buffer) {
        return Err(e);
    }

    let variant = match &first_buffer {
        b"STANDARD BEN FILE" => {
            writer.write_all(b"STANDARD BEN FILE")?;
            BenVariant::Standard
        }
        b"MKVCHAIN BEN FILE" => {
            writer.write_all(b"MKVCHAIN BEN FILE")?;
            BenVariant::MkvChain
        }
        b"TWODELTA BEN FILE" => {
            let mut xben = XBenDecoder::from_decompressed_stream(BufReader::new(decoder), BenVariant::TwoDelta);
            let mut ben = BenEncoder::new(writer, BenVariant::TwoDelta);
            for record in &mut xben {
                let (assignment, count) = record?;
                ben.write_assignment(assignment.clone())?;
                for _ in 1..count {
                    ben.write_assignment(assignment.clone())?;
                }
            }
            return Ok(());
        }
        _ => {
            return Err(Error::new(
                io::ErrorKind::InvalidData,
                "Invalid file format",
            ));
        }
    };

    let mut buffer = [0u8; 1048576];
    let mut overflow: Vec<u8> = Vec::new();

    let mut line_count: usize = 0;
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
                for i in (3..overflow.len() - 2).step_by(2) {
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

        ben32_to_ben_lines(&overflow[0..last_valid_assignment], &mut writer, variant)?;
        overflow = overflow[last_valid_assignment..].to_vec();
    }
    tracing::trace!("");
    tracing::trace!("Done!");
    Ok(())
}

/// Decompress a general XZ byte stream without applying any BEN-specific logic.
///
/// # Arguments
///
/// * `reader` - The compressed XZ stream.
/// * `writer` - The destination for the decompressed bytes.
///
/// # Returns
///
/// Returns `Ok(())` once the compressed stream has been fully expanded.
pub fn xz_decompress<R: BufRead, W: Write>(reader: R, mut writer: W) -> io::Result<()> {
    let mut decoder = XzDecoder::new(reader);
    let mut buffer = [0u8; 4096];

    while let Ok(count) = decoder.read(&mut buffer) {
        if count == 0 {
            break;
        }
        writer.write_all(&buffer[..count])?;
    }

    Ok(())
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

    let mut first_buffer = [0u8; 17];

    if let Err(e) = decoder.read_exact(&mut first_buffer) {
        return Err(e);
    }

    let variant = match &first_buffer {
        b"STANDARD BEN FILE" => BenVariant::Standard,
        b"MKVCHAIN BEN FILE" => BenVariant::MkvChain,
        b"TWODELTA BEN FILE" => {
            let mut xben = XBenDecoder::from_decompressed_stream(BufReader::new(decoder), BenVariant::TwoDelta);
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
        _ => {
            return Err(Error::new(
                io::ErrorKind::InvalidData,
                "Invalid file format",
            ));
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
