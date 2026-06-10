use crate::io::reader::BenStreamReader;
use std::io::{self, BufRead, Read, Write};

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
///
/// # Errors
///
/// Surfaces an error (rather than a truncated result) if the decompressed stream ends partway
/// through a frame, declares a zero repetition count, or carries an unknown banner.
pub fn decode_xben_to_jsonl<R: BufRead, W: Write>(reader: R, writer: W) -> io::Result<()> {
    let mut xben_decoder = BenStreamReader::from_xben(reader).map_err(io::Error::from)?;
    xben_decoder.write_all_jsonl(writer)
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
        // propagates through the write_all_jsonl call. No custom Write impl needed.
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
