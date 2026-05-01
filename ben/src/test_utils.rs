//! Test helpers shared across unit and integration tests.
//!
//! This module is always-compiled (not `#[cfg(test)]`) so integration tests
//! in `ben/tests/` — which are separate crates — can reuse the same helpers
//! as unit tests inside `ben/src/.../tests.rs`. It is `#[doc(hidden)]` and
//! is not part of the stable public API.

use std::io::{Cursor, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;

use crate::codec::encode::encode_jsonl_to_ben;
use crate::io::bundle::format::AssignmentFormat;
use crate::io::bundle::BendlWriter;
use crate::BenVariant;

/// Return a unique temp path of the form `binary-ensemble-{name}-{nonce}` in
/// the system temp directory. The nonce is the current monotonic-ish time in
/// nanoseconds, sufficient to avoid collisions between parallel test runs.
pub fn unique_path(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("binary-ensemble-{name}-{nonce}"))
}

/// Build a JSONL byte buffer from a sequence of assignment vectors,
/// numbering samples from 1.
pub fn jsonl_from_assignments(assignments: &[Vec<u16>]) -> Vec<u8> {
    let mut buf = Vec::new();
    for (i, a) in assignments.iter().enumerate() {
        writeln!(&mut buf, "{}", json!({"assignment": a, "sample": i + 1})).unwrap();
    }
    buf
}

/// Expand an RLE sequence `(value, length)` into a flat assignment vector,
/// truncating at `cap`.
pub fn expand_rle(rle: &[(u16, u16)], cap: usize) -> Vec<u16> {
    let mut v = Vec::with_capacity(cap);
    for &(val, len) in rle {
        let take = (len as usize).min(cap.saturating_sub(v.len()));
        v.extend(std::iter::repeat_n(val, take));
        if v.len() >= cap {
            break;
        }
    }
    v
}

/// Encode the given JSONL bytes as a BEN byte vector, including the 17-byte
/// banner. Panics on encoder error; intended only for fixture construction.
pub fn sample_ben_bytes(jsonl: &[u8], variant: BenVariant) -> Vec<u8> {
    let mut out = Vec::new();
    encode_jsonl_to_ben(jsonl, &mut out, variant).unwrap();
    out
}

/// Build a minimal finalized `.bendl` byte vector containing the given
/// pre-encoded assignment stream bytes. Panics on writer error; intended
/// only for fixture construction.
pub fn sample_bendl_bytes(stream: &[u8], format: AssignmentFormat) -> Vec<u8> {
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut writer = BendlWriter::new(Cursor::new(&mut buf), format).unwrap();
        writer.write_stream_bytes(stream, 1).unwrap();
        writer.finish().unwrap();
    }
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_path_includes_name_and_is_unique() {
        let a = unique_path("hello");
        let b = unique_path("hello");
        assert!(a.file_name().unwrap().to_string_lossy().contains("hello"));
        assert_ne!(a, b);
    }

    #[test]
    fn jsonl_from_assignments_emits_one_line_per_sample() {
        let out = jsonl_from_assignments(&[vec![1, 2, 3], vec![2, 1, 3]]);
        let s = std::str::from_utf8(&out).unwrap();
        assert_eq!(s.lines().count(), 2);
        assert!(s.contains("\"sample\":1"));
        assert!(s.contains("\"sample\":2"));
        assert!(s.contains("[1,2,3]"));
    }

    #[test]
    fn expand_rle_truncates_at_cap() {
        let v = expand_rle(&[(1, 5), (2, 5)], 7);
        assert_eq!(v, vec![1, 1, 1, 1, 1, 2, 2]);
    }

    #[test]
    fn expand_rle_handles_zero_cap() {
        let v = expand_rle(&[(1, 5)], 0);
        assert!(v.is_empty());
    }

    #[test]
    fn sample_ben_bytes_round_trips_via_decode() {
        use crate::codec::decode::decode_ben_to_jsonl;
        let jsonl = jsonl_from_assignments(&[vec![1, 2, 3]]);
        let ben = sample_ben_bytes(&jsonl, BenVariant::Standard);
        let mut decoded = Vec::new();
        decode_ben_to_jsonl(ben.as_slice(), &mut decoded).unwrap();
        let s = String::from_utf8(decoded).unwrap();
        assert!(s.contains("[1,2,3]"));
    }

    #[test]
    fn sample_bendl_bytes_yields_complete_bundle() {
        use crate::io::bundle::BendlReader;
        use std::io::BufReader;

        let bytes = sample_bendl_bytes(b"STANDARD BEN FILE\x00fake", AssignmentFormat::Ben);
        let reader = BendlReader::open(BufReader::new(Cursor::new(bytes))).unwrap();
        assert!(reader.is_complete());
    }
}
