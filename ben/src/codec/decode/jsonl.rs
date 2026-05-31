use crate::codec::decode::jsonl_decode_ben32;
use crate::format::banners::{variant_from_banner, BANNER_LEN};
use crate::format::FormatError;
use crate::io::reader::BenStreamReader;
use crate::progress::Spinner;
use crate::BenVariant;
use serde_json::json;
use std::io::{self, BufRead, BufReader, Read, Write};
use xz2::read::XzDecoder;

/// Decode a BEN stream into JSONL assignment records.
///
/// Each decoded sample is written as a JSON object containing an `assignment` vector and a 1-based
/// `sample` index.
///
/// # Arguments
///
/// * `reader` - The input BEN stream, including the 17-byte BEN banner.
/// * `writer` - The destination that will receive one JSON object per decoded sample.
///
/// # Returns
///
/// Returns `Ok(())` after the stream has been fully decoded and written.
pub fn decode_ben_to_jsonl<R: Read, W: Write>(reader: R, writer: W) -> io::Result<()> {
    let mut ben_decoder = BenStreamReader::from_ben(reader)?;
    ben_decoder.write_all_jsonl(writer)
}

/// Decode an XBEN stream directly into JSONL assignment records.
///
/// # Arguments
///
/// * `reader` - The compressed XBEN input stream.
/// * `writer` - The destination that will receive one JSON object per decoded sample.
///
/// # Returns
///
/// Returns `Ok(())` after the XBEN stream has been fully decoded into JSONL.
pub fn decode_xben_to_jsonl<R: BufRead, W: Write>(reader: R, mut writer: W) -> io::Result<()> {
    let mut decoder = XzDecoder::new(reader);

    let mut first_buffer = [0u8; BANNER_LEN];

    decoder.read_exact(&mut first_buffer)?;

    let variant = match variant_from_banner(&first_buffer) {
        Some(BenVariant::Standard) => BenVariant::Standard,
        Some(BenVariant::MkvChain) => BenVariant::MkvChain,
        Some(BenVariant::TwoDelta) => {
            let mut xben = BenStreamReader::from_xben_decompressed(
                BufReader::new(decoder),
                BenVariant::TwoDelta,
            );
            let mut sample_number = 1usize;
            let spinner = Spinner::new("Decoding sample");
            for record in &mut xben {
                let (assignment, count) = record?;
                for _ in 0..count {
                    spinner.set_count(sample_number as u64);
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
    let spinner = Spinner::new("Decoding sample");
    loop {
        let count = decoder.read(&mut buffer)?;
        if count == 0 {
            break;
        }

        overflow.extend(&buffer[..count]);

        let mut last_valid_assignment = 0;

        // TwoDelta was dispatched before this loop and returned early.
        if variant == BenVariant::Standard {
            for i in (3..overflow.len()).step_by(4) {
                if overflow[i - 3..=i] == [0, 0, 0, 0] {
                    last_valid_assignment = i + 1;
                    line_count += 1;
                    spinner.set_count(line_count as u64);
                }
            }
        } else {
            for i in (last_valid_assignment + 3..overflow.len().saturating_sub(2)).step_by(2) {
                if overflow[i - 3..=i] == [0, 0, 0, 0] {
                    last_valid_assignment = i + 3;
                    let lines = &overflow[i + 1..i + 3];
                    let n_lines = u16::from_be_bytes([lines[0], lines[1]]);
                    line_count += n_lines as usize;
                    spinner.set_count(line_count as u64);
                }
            }
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::encode::encode_jsonl_to_xben;
    use crate::BenVariant;
    use std::io::{self, BufReader};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn decode_xben_to_jsonl_writer_error_propagates() {
        // Build a valid Standard XBEN stream.
        let jsonl = b"{\"assignment\":[1,2,3],\"sample\":1}\n";
        let mut xben = Vec::new();
        encode_jsonl_to_xben(
            jsonl.as_slice(),
            &mut xben,
            BenVariant::Standard,
            Some(1),
            Some(1),
            None,
            None,
        )
        .unwrap();

        // Use a read-only File as the writer — writing to it fails with a permission error, which
        // propagates through the jsonl_decode_ben32 call at line 128 of this file. No custom Write
        // impl needed.
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("xben-ro-{nonce}.tmp"));
        std::fs::write(&path, b"").unwrap();
        let ro_file = std::fs::File::open(&path).unwrap(); // read-only
                                                           // Writing to a read-only file fails —
                                                           // the exact error kind varies by OS.
        let err = decode_xben_to_jsonl(BufReader::new(xben.as_slice()), ro_file).unwrap_err();
        assert!(err.kind() != io::ErrorKind::UnexpectedEof);
        let _ = std::fs::remove_file(path);
    }
}
