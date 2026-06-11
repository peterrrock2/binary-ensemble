mod mkvchain;
mod standard;
mod twodelta;

use std::io;

#[test]
fn decode_error_io_passthrough() {
    let inner = io::Error::new(io::ErrorKind::BrokenPipe, "pipe broke");
    let decode_err = super::DecodeError::Io(inner);
    let io_err: io::Error = decode_err.into();
    assert_eq!(io_err.kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(io_err.to_string(), "pipe broke");
}

#[test]
fn decode_error_non_io_becomes_invalid_data() {
    let decode_err = super::DecodeError::TwoDeltaNoAnchorFrame;
    let io_err: io::Error = decode_err.into();
    assert_eq!(io_err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn decode_xben_to_ben_twodelta_roundtrip() {
    use crate::codec::decode::{decode_ben_to_jsonl, decode_xben_to_ben};
    use crate::codec::encode::encode_jsonl_to_xben;
    use crate::BenVariant;
    use serde_json::Value;
    use std::io::BufReader;

    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[2,1,2,2],"sample":2}
{"assignment":[2,2,2,2],"sample":3}
"#;
    let mut xben = Vec::new();
    encode_jsonl_to_xben(
        jsonl.as_bytes(),
        &mut xben,
        BenVariant::TwoDelta,
        Some(1),
        Some(1),
        None,
        None,
    )
    .unwrap();

    // Decode XBEN → BEN
    let mut ben = Vec::new();
    decode_xben_to_ben(BufReader::new(xben.as_slice()), &mut ben).unwrap();
    assert!(!ben.is_empty());

    // Decode BEN → JSONL and verify
    let mut jsonl_out = Vec::new();
    decode_ben_to_jsonl(ben.as_slice(), &mut jsonl_out).unwrap();
    let output_str = String::from_utf8(jsonl_out).unwrap();
    let lines: Vec<&str> = output_str.trim().split('\n').collect();
    assert_eq!(lines.len(), 3);

    let v1: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(v1["assignment"], serde_json::json!([1, 1, 2, 2]));
    let v2: Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(v2["assignment"], serde_json::json!([2, 1, 2, 2]));
    let v3: Value = serde_json::from_str(lines[2]).unwrap();
    assert_eq!(v3["assignment"], serde_json::json!([2, 2, 2, 2]));
}

#[test]
fn decode_xben_to_jsonl_twodelta() {
    use crate::codec::decode::decode_xben_to_jsonl;
    use crate::codec::encode::encode_jsonl_to_xben;
    use crate::BenVariant;
    use serde_json::Value;
    use std::io::BufReader;

    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[2,1,2,2],"sample":2}
"#;
    let mut xben = Vec::new();
    encode_jsonl_to_xben(
        jsonl.as_bytes(),
        &mut xben,
        BenVariant::TwoDelta,
        Some(1),
        Some(1),
        None,
        None,
    )
    .unwrap();

    let mut jsonl_out = Vec::new();
    decode_xben_to_jsonl(BufReader::new(xben.as_slice()), &mut jsonl_out).unwrap();
    let output_str = String::from_utf8(jsonl_out).unwrap();
    let lines: Vec<&str> = output_str.trim().split('\n').collect();
    assert_eq!(lines.len(), 2);

    let v1: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(v1["assignment"], serde_json::json!([1, 1, 2, 2]));
}

#[test]
fn decode_xben_to_jsonl_rejects_invalid_banner() {
    use crate::codec::decode::decode_xben_to_jsonl;
    use crate::codec::encode::xz_compress;
    use std::io::BufReader;

    // Create XZ-compressed data with a bad banner
    let mut bad_data = b"GARBAGE BANNER!!!".to_vec();
    bad_data.extend_from_slice(&[0u8; 20]);
    let mut xz = Vec::new();
    xz_compress(bad_data.as_slice(), &mut xz, Some(1), Some(1), None).unwrap();

    let mut output = Vec::new();
    let err = decode_xben_to_jsonl(BufReader::new(xz.as_slice()), &mut output).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn encode_ben_to_xben_roundtrip() {
    use crate::codec::decode::decode_xben_to_ben;
    use crate::codec::encode::{encode_ben_to_xben, encode_jsonl_to_ben};
    use crate::BenVariant;
    use std::io::BufReader;

    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[2,2,1,1],"sample":2}
"#;
    // JSONL → BEN
    let mut ben = Vec::new();
    encode_jsonl_to_ben(jsonl.as_bytes(), &mut ben, BenVariant::Standard).unwrap();

    // BEN → XBEN
    let mut xben = Vec::new();
    encode_ben_to_xben(ben.as_slice(), &mut xben, Some(1), Some(1), None, None).unwrap();

    // XBEN → BEN
    let mut ben2 = Vec::new();
    decode_xben_to_ben(BufReader::new(xben.as_slice()), &mut ben2).unwrap();

    assert_eq!(ben, ben2);
}

#[test]
fn encode_ben_to_xben_with_chunk_size() {
    use crate::codec::decode::{decode_ben_to_jsonl, decode_xben_to_ben};
    use crate::codec::encode::{encode_ben_to_xben, encode_jsonl_to_ben};
    use crate::BenVariant;
    use serde_json::Value;
    use std::io::BufReader;

    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[2,2,1,1],"sample":2}
"#;
    let mut ben = Vec::new();
    encode_jsonl_to_ben(jsonl.as_bytes(), &mut ben, BenVariant::Standard).unwrap();

    let mut xben = Vec::new();
    encode_ben_to_xben(ben.as_slice(), &mut xben, Some(1), Some(1), Some(1), None).unwrap();
    assert!(!xben.is_empty());

    // Verify content roundtrips correctly
    let mut ben2 = Vec::new();
    decode_xben_to_ben(BufReader::new(xben.as_slice()), &mut ben2).unwrap();
    let mut jsonl_out = Vec::new();
    decode_ben_to_jsonl(ben2.as_slice(), &mut jsonl_out).unwrap();
    let output_str = String::from_utf8(jsonl_out).unwrap();
    let lines: Vec<&str> = output_str.trim().split('\n').collect();
    assert_eq!(lines.len(), 2);
    let v1: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(v1["assignment"], serde_json::json!([1, 1, 2, 2]));
    let v2: Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(v2["assignment"], serde_json::json!([2, 2, 1, 1]));
}

#[test]
fn encode_ben_to_xben_mkvchain_roundtrip() {
    use crate::codec::decode::{decode_ben_to_jsonl, decode_xben_to_ben};
    use crate::codec::encode::{encode_ben_to_xben, encode_jsonl_to_ben};
    use crate::BenVariant;
    use serde_json::Value;
    use std::io::BufReader;

    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[1,1,2,2],"sample":2}
{"assignment":[2,2,1,1],"sample":3}
"#;
    let mut ben = Vec::new();
    encode_jsonl_to_ben(jsonl.as_bytes(), &mut ben, BenVariant::MkvChain).unwrap();

    let mut xben = Vec::new();
    encode_ben_to_xben(ben.as_slice(), &mut xben, Some(1), Some(1), None, None).unwrap();

    let mut ben2 = Vec::new();
    decode_xben_to_ben(BufReader::new(xben.as_slice()), &mut ben2).unwrap();
    let mut jsonl_out = Vec::new();
    decode_ben_to_jsonl(ben2.as_slice(), &mut jsonl_out).unwrap();
    let output_str = String::from_utf8(jsonl_out).unwrap();
    let lines: Vec<&str> = output_str.trim().split('\n').collect();
    assert_eq!(lines.len(), 3);
    let v1: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(v1["assignment"], serde_json::json!([1, 1, 2, 2]));
    let v2: Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(v2["assignment"], serde_json::json!([1, 1, 2, 2]));
    let v3: Value = serde_json::from_str(lines[2]).unwrap();
    assert_eq!(v3["assignment"], serde_json::json!([2, 2, 1, 1]));
}

#[test]
fn decode_twodelta_frame_rejects_zero_run_length() {
    use crate::codec::decode::decode_twodelta_frame;
    use crate::codec::BenEncodeFrame;

    // The delta paint loop assumes no zero-length runs exist (a zero would underflow its
    // per-run countdown and mispaint positions), so a frame carrying one is rejected up front.
    let frame = BenEncodeFrame::from_run_lengths((1, 2), vec![1, 0, 1], Some(1)).unwrap();
    let err = decode_twodelta_frame(vec![1, 2], &frame).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert!(err.to_string().contains("zero"));
}

#[test]
fn decode_error_remaining_variants() {
    // Test DecodeError variants we haven't covered
    let err = super::DecodeError::XBenUnknownFrameTag { tag: 0xFF };
    let io_err: io::Error = err.into();
    assert_eq!(io_err.kind(), io::ErrorKind::InvalidData);
    assert!(io_err.to_string().contains("0xff"));

    let err = super::DecodeError::XBenTruncated;
    let io_err: io::Error = err.into();
    assert_eq!(io_err.kind(), io::ErrorKind::InvalidData);

    let err = super::DecodeError::UnexpectedTwoDeltaFrame {
        variant: crate::BenVariant::Standard,
    };
    let io_err: io::Error = err.into();
    assert_eq!(io_err.kind(), io::ErrorKind::InvalidData);

    let err = super::DecodeError::TwoDeltaRunsExhausted { run_idx: 3, pos: 7 };
    let io_err: io::Error = err.into();
    assert_eq!(io_err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn decode_xben_to_ben_twodelta_with_repeated_assignments() {
    use crate::codec::decode::{decode_ben_to_jsonl, decode_xben_to_ben};
    use crate::codec::encode::encode_jsonl_to_xben;
    use crate::BenVariant;
    use serde_json::Value;
    use std::io::BufReader;

    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[1,1,2,2],"sample":2}
{"assignment":[1,1,2,2],"sample":3}
{"assignment":[2,1,2,2],"sample":4}
"#;
    let mut xben = Vec::new();
    encode_jsonl_to_xben(
        jsonl.as_bytes(),
        &mut xben,
        BenVariant::TwoDelta,
        Some(1),
        Some(1),
        None,
        None,
    )
    .unwrap();

    let mut ben = Vec::new();
    decode_xben_to_ben(BufReader::new(xben.as_slice()), &mut ben).unwrap();

    let mut jsonl_out = Vec::new();
    decode_ben_to_jsonl(ben.as_slice(), &mut jsonl_out).unwrap();
    let output_str = String::from_utf8(jsonl_out).unwrap();
    let lines: Vec<&str> = output_str.trim().split('\n').collect();
    assert_eq!(lines.len(), 4);
    let v1: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(v1["assignment"], serde_json::json!([1, 1, 2, 2]));
    let v4: Value = serde_json::from_str(lines[3]).unwrap();
    assert_eq!(v4["assignment"], serde_json::json!([2, 1, 2, 2]));
}

#[test]
fn xz_decompress_roundtrip() {
    use crate::codec::decode::xz_decompress;
    use crate::codec::encode::xz_compress;
    use std::io::BufReader;

    let original = b"hello world, this is a test of xz_decompress";
    let mut compressed = Vec::new();
    xz_compress(original.as_slice(), &mut compressed, Some(1), Some(1), None).unwrap();

    let mut decompressed = Vec::new();
    xz_decompress(BufReader::new(compressed.as_slice()), &mut decompressed).unwrap();
    assert_eq!(decompressed, original);
}

#[test]
fn xz_compress_direct_test() {
    use crate::codec::encode::xz_compress;

    let data = b"compress me please with xz";
    let mut out = Vec::new();
    xz_compress(data.as_slice(), &mut out, None, None, None).unwrap();
    assert!(!out.is_empty());

    let mut decompressed = Vec::new();
    crate::codec::decode::xz_decompress(std::io::BufReader::new(out.as_slice()), &mut decompressed)
        .unwrap();
    assert_eq!(decompressed, data);
}

#[test]
fn encode_ben_to_xben_rejects_invalid_banner() {
    use crate::codec::encode::encode_ben_to_xben;

    let garbage = b"GARBAGE BANNER!!!extra_padding";
    let mut out = Vec::new();
    let err =
        encode_ben_to_xben(garbage.as_slice(), &mut out, Some(1), Some(1), None, None).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn decode_xben_to_ben_rejects_invalid_banner() {
    use crate::codec::decode::decode_xben_to_ben;
    use crate::codec::encode::xz_compress;
    use std::io::BufReader;

    let mut bad_data = b"GARBAGE BANNER!!!".to_vec();
    bad_data.extend_from_slice(&[0u8; 20]);
    let mut xz = Vec::new();
    xz_compress(bad_data.as_slice(), &mut xz, Some(1), Some(1), None).unwrap();

    let mut output = Vec::new();
    let err = decode_xben_to_ben(BufReader::new(xz.as_slice()), &mut output).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}
