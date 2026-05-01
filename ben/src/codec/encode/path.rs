//! Path-based convenience wrappers around the streaming encoders.
//!
//! Each wrapper opens a buffered reader on the input and a buffered writer on
//! the output, then delegates to the corresponding streaming function. The
//! wrappers exist so that CLI dispatch and library consumers do not have to
//! repeat the `BufReader`/`BufWriter`/`File` plumbing at every callsite.

use std::fs::File;
use std::io::{BufReader, BufWriter, Result};
use std::path::Path;

use crate::BenVariant;

use super::{encode_ben_to_xben, encode_jsonl_to_ben, encode_jsonl_to_xben, xz_compress};

/// Encode a JSONL file at `input` into a BEN file at `output`.
pub fn encode_jsonl_to_ben_path(input: &Path, output: &Path, variant: BenVariant) -> Result<()> {
    let reader = BufReader::new(File::open(input)?);
    let writer = BufWriter::new(File::create(output)?);
    encode_jsonl_to_ben(reader, writer, variant)
}

/// Encode a JSONL file at `input` into an XBEN file at `output`.
pub fn encode_jsonl_to_xben_path(
    input: &Path,
    output: &Path,
    variant: BenVariant,
    n_threads: Option<u32>,
    compression_level: Option<u32>,
    chunk_size: Option<usize>,
) -> Result<()> {
    let reader = BufReader::new(File::open(input)?);
    let writer = BufWriter::new(File::create(output)?);
    encode_jsonl_to_xben(reader, writer, variant, n_threads, compression_level, chunk_size)
}

/// Encode a BEN file at `input` into an XBEN file at `output`.
pub fn encode_ben_to_xben_path(
    input: &Path,
    output: &Path,
    n_threads: Option<u32>,
    compression_level: Option<u32>,
    chunk_size: Option<usize>,
) -> Result<()> {
    let reader = BufReader::new(File::open(input)?);
    let writer = BufWriter::new(File::create(output)?);
    encode_ben_to_xben(reader, writer, n_threads, compression_level, chunk_size)
}

/// Compress an arbitrary file at `input` into an `.xz` file at `output`.
pub fn xz_compress_path(
    input: &Path,
    output: &Path,
    n_threads: Option<u32>,
    compression_level: Option<u32>,
) -> Result<()> {
    let reader = BufReader::new(File::open(input)?);
    let writer = BufWriter::new(File::create(output)?);
    xz_compress(reader, writer, n_threads, compression_level)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{jsonl_from_assignments, unique_path};

    #[test]
    fn jsonl_to_ben_path_round_trips() {
        use crate::codec::decode::path::decode_ben_to_jsonl_path;

        let jsonl_in = unique_path("path-encode-jsonl.jsonl");
        let ben_out = unique_path("path-encode-jsonl.ben");
        let jsonl_back = unique_path("path-encode-jsonl-back.jsonl");

        std::fs::write(
            &jsonl_in,
            jsonl_from_assignments(&[vec![1, 2, 3], vec![2, 1, 3]]),
        )
        .unwrap();

        encode_jsonl_to_ben_path(&jsonl_in, &ben_out, BenVariant::Standard).unwrap();
        decode_ben_to_jsonl_path(&ben_out, &jsonl_back).unwrap();

        let s = std::fs::read_to_string(&jsonl_back).unwrap();
        assert!(s.contains("[1,2,3]"));
        assert!(s.contains("[2,1,3]"));

        for p in [&jsonl_in, &ben_out, &jsonl_back] {
            let _ = std::fs::remove_file(p);
        }
    }

    #[test]
    fn jsonl_to_xben_path_round_trips() {
        use crate::codec::decode::path::decode_xben_to_jsonl_path;

        let jsonl_in = unique_path("path-encode-xben.jsonl");
        let xben_out = unique_path("path-encode-xben.xben");
        let jsonl_back = unique_path("path-encode-xben-back.jsonl");

        std::fs::write(
            &jsonl_in,
            jsonl_from_assignments(&[vec![1, 2, 3], vec![2, 1, 3]]),
        )
        .unwrap();

        encode_jsonl_to_xben_path(
            &jsonl_in,
            &xben_out,
            BenVariant::Standard,
            Some(1),
            Some(1),
            None,
        )
        .unwrap();
        decode_xben_to_jsonl_path(&xben_out, &jsonl_back).unwrap();

        let s = std::fs::read_to_string(&jsonl_back).unwrap();
        assert!(s.contains("[1,2,3]"));

        for p in [&jsonl_in, &xben_out, &jsonl_back] {
            let _ = std::fs::remove_file(p);
        }
    }

    #[test]
    fn ben_to_xben_path_round_trips() {
        use crate::codec::decode::path::decode_xben_to_ben_path;

        let jsonl_in = unique_path("path-bxb.jsonl");
        let ben = unique_path("path-bxb.ben");
        let xben = unique_path("path-bxb.xben");
        let ben_back = unique_path("path-bxb-back.ben");

        std::fs::write(
            &jsonl_in,
            jsonl_from_assignments(&[vec![1, 2, 3]]),
        )
        .unwrap();
        encode_jsonl_to_ben_path(&jsonl_in, &ben, BenVariant::Standard).unwrap();
        encode_ben_to_xben_path(&ben, &xben, Some(1), Some(1), None).unwrap();
        decode_xben_to_ben_path(&xben, &ben_back).unwrap();

        // Round trip: ben_back should be byte-equivalent to ben (same banner, same content).
        assert_eq!(std::fs::read(&ben).unwrap(), std::fs::read(&ben_back).unwrap());

        for p in [&jsonl_in, &ben, &xben, &ben_back] {
            let _ = std::fs::remove_file(p);
        }
    }

    #[test]
    fn xz_compress_path_round_trips() {
        use crate::codec::decode::path::xz_decompress_path;

        let plain = unique_path("path-xz.txt");
        let xz_out = unique_path("path-xz.txt.xz");
        let plain_back = unique_path("path-xz-back.txt");

        std::fs::write(&plain, b"hello world\n").unwrap();
        xz_compress_path(&plain, &xz_out, Some(1), Some(1)).unwrap();
        xz_decompress_path(&xz_out, &plain_back).unwrap();

        assert_eq!(std::fs::read(&plain_back).unwrap(), b"hello world\n");

        for p in [&plain, &xz_out, &plain_back] {
            let _ = std::fs::remove_file(p);
        }
    }

    #[test]
    fn encode_path_propagates_open_error() {
        let missing = unique_path("nonexistent.jsonl");
        let out = unique_path("encode-fail.ben");
        let err = encode_jsonl_to_ben_path(&missing, &out, BenVariant::Standard).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
        // Note: out is created before the read fails, so its absence is not asserted.
        let _ = std::fs::remove_file(&out);
    }
}
