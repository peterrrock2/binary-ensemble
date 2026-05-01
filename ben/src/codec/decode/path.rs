//! Path-based convenience wrappers around the streaming decoders.
//!
//! Each wrapper opens a buffered reader on the input and a buffered writer on
//! the output, then delegates to the corresponding streaming function. The
//! wrappers exist so that CLI dispatch and library consumers do not have to
//! repeat the `BufReader`/`BufWriter`/`File` plumbing at every callsite.

use std::fs::File;
use std::io::{BufReader, BufWriter, Result};
use std::path::Path;

use super::{decode_ben_to_jsonl, decode_xben_to_ben, decode_xben_to_jsonl, xz_decompress};

/// Decode a BEN file at `input` into a JSONL file at `output`.
pub fn decode_ben_to_jsonl_path(input: &Path, output: &Path) -> Result<()> {
    let reader = BufReader::new(File::open(input)?);
    let writer = BufWriter::new(File::create(output)?);
    decode_ben_to_jsonl(reader, writer)
}

/// Decode an XBEN file at `input` into a JSONL file at `output`.
pub fn decode_xben_to_jsonl_path(input: &Path, output: &Path) -> Result<()> {
    let reader = BufReader::new(File::open(input)?);
    let writer = BufWriter::new(File::create(output)?);
    decode_xben_to_jsonl(reader, writer)
}

/// Decode an XBEN file at `input` into a BEN file at `output`.
pub fn decode_xben_to_ben_path(input: &Path, output: &Path) -> Result<()> {
    let reader = BufReader::new(File::open(input)?);
    let writer = BufWriter::new(File::create(output)?);
    decode_xben_to_ben(reader, writer)
}

/// Decompress an `.xz` file at `input` into a plain file at `output`.
pub fn xz_decompress_path(input: &Path, output: &Path) -> Result<()> {
    let reader = BufReader::new(File::open(input)?);
    let writer = BufWriter::new(File::create(output)?);
    xz_decompress(reader, writer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::unique_path;

    #[test]
    fn decode_path_propagates_open_error() {
        let missing = unique_path("nonexistent.ben");
        let out = unique_path("decode-fail.jsonl");
        let err = decode_ben_to_jsonl_path(&missing, &out).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
        let _ = std::fs::remove_file(&out);
    }

    // The happy-path round-trip tests for these decoders live alongside the
    // matching encoders in `super::super::encode::path::tests`.
}
