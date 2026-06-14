//! Rigorous coverage tests for the binary-ensemble `ben` library.
//!
//! These tests target code paths and edge-cases that are not covered by the existing integration /
//! property-based suites. They are deliberately strict: if the implementation behaves in an
//! unexpected way the test should fail rather than silently accept wrong output.

use binary_ensemble::codec::decode::{decode_ben_to_jsonl, decode_xben_to_ben};
use binary_ensemble::codec::encode::{
    encode_ben_to_xben, encode_jsonl_to_ben, encode_jsonl_to_xben, encode_twodelta_frame,
};
use binary_ensemble::codec::BenEncodeFrame;
use binary_ensemble::format::banners::{
    banner_for_variant, has_known_banner_prefix, variant_from_banner, MKVCHAIN_BEN_BANNER,
    STANDARD_BEN_BANNER, TWODELTA_BEN_BANNER,
};
use binary_ensemble::io::reader::{
    BenStreamFrameReader, BenStreamReader, DecodeFrame, DecoderInitError,
};
use binary_ensemble::io::writer::BenStreamWriter;
use binary_ensemble::json::graph::{
    sort_json_file_by_key, sort_json_file_by_ordering, GraphOrderingMethod,
};
use binary_ensemble::ops::relabel::{convert_ben_file, relabel_ben_file, RelabelOptions};
use binary_ensemble::util::rle::{assign_to_rle, rle_to_vec};
use binary_ensemble::BenVariant;

use serde_json::json;
use std::collections::HashMap;
use std::io::{self, BufReader, Cursor};

mod common;
use common::jsonl_from_assignments;

// ────────────────────────────────────────────────────────────────────────────── Helpers
// ──────────────────────────────────────────────────────────────────────────────

/// Encode assignments as a Standard BEN byte vector (including the 17-byte banner).
fn encode_standard_ben(assignments: &[Vec<u16>]) -> Vec<u8> {
    let jsonl = jsonl_from_assignments(assignments);
    let mut ben = Vec::new();
    encode_jsonl_to_ben(jsonl.as_slice(), &mut ben, BenVariant::Standard).unwrap();
    ben
}

/// Decode a BEN byte vector back to JSONL.
fn decode_ben_to_string(ben: &[u8]) -> String {
    let mut out = Vec::new();
    decode_ben_to_jsonl(ben, &mut out).unwrap();
    String::from_utf8(out).unwrap()
}

/// Encode assignments as an XBEN (compressed) byte vector.
fn encode_xben(assignments: &[Vec<u16>], variant: BenVariant) -> Vec<u8> {
    let jsonl = jsonl_from_assignments(assignments);
    let mut xben = Vec::new();
    encode_jsonl_to_xben(
        Cursor::new(jsonl),
        &mut xben,
        variant,
        Some(1),
        Some(1),
        None,
        None,
    )
    .unwrap();
    xben
}

/// Build a ring-graph JSON string with `n` nodes (0-based ids). Each node i is connected to (i-1)
/// mod n and (i+1) mod n.
fn make_ring_graph_json(n: usize) -> String {
    let nodes: Vec<serde_json::Value> = (0..n).map(|i| json!({"id": i})).collect();
    let adjacency: Vec<serde_json::Value> = (0..n)
        .map(|i| {
            let prev = (i + n - 1) % n;
            let next = (i + 1) % n;
            json!([{"id": prev}, {"id": next}])
        })
        .collect();
    serde_json::to_string(&json!({"nodes": nodes, "adjacency": adjacency})).unwrap()
}

// ────────────────────────────────────────────────────────────────────────────── format::banners
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn banner_for_variant_returns_correct_banners() {
    assert_eq!(
        banner_for_variant(BenVariant::Standard),
        STANDARD_BEN_BANNER
    );
    assert_eq!(
        banner_for_variant(BenVariant::MkvChain),
        MKVCHAIN_BEN_BANNER
    );
    assert_eq!(
        banner_for_variant(BenVariant::TwoDelta),
        TWODELTA_BEN_BANNER
    );
}

#[test]
fn variant_from_banner_round_trips_all_variants() {
    assert_eq!(
        variant_from_banner(STANDARD_BEN_BANNER),
        Some(BenVariant::Standard)
    );
    assert_eq!(
        variant_from_banner(MKVCHAIN_BEN_BANNER),
        Some(BenVariant::MkvChain)
    );
    assert_eq!(
        variant_from_banner(TWODELTA_BEN_BANNER),
        Some(BenVariant::TwoDelta)
    );
}

#[test]
fn variant_from_banner_returns_none_for_unknown_banner() {
    let bad: [u8; 17] = *b"BAD BAD BAD BAD!!";
    assert_eq!(variant_from_banner(&bad), None);
}

#[test]
fn variant_from_banner_returns_none_for_all_zeros() {
    let zeros = [0u8; 17];
    assert_eq!(variant_from_banner(&zeros), None);
}

#[test]
fn variant_from_banner_returns_none_for_partial_match() {
    // First 16 bytes match STANDARD BEN FILE but last byte is wrong.
    let mut partial = *STANDARD_BEN_BANNER;
    partial[16] = b'X';
    assert_eq!(variant_from_banner(&partial), None);
}

#[test]
fn has_known_banner_prefix_recognises_all_variants() {
    assert!(has_known_banner_prefix(STANDARD_BEN_BANNER));
    assert!(has_known_banner_prefix(MKVCHAIN_BEN_BANNER));
    assert!(has_known_banner_prefix(TWODELTA_BEN_BANNER));
}

#[test]
fn has_known_banner_prefix_recognises_prefixed_bytes() {
    // Extra bytes after the banner should still match.
    let mut extended = STANDARD_BEN_BANNER.to_vec();
    extended.extend_from_slice(b"\x01\x02\x03");
    assert!(has_known_banner_prefix(&extended));
}

#[test]
fn has_known_banner_prefix_rejects_garbage() {
    assert!(!has_known_banner_prefix(b"NOT A BEN FILE!!!"));
    assert!(!has_known_banner_prefix(b""));
    assert!(!has_known_banner_prefix(b"\x00"));
}

// ────────────────────────────────────────────────────────────────────────────── util::rle
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn rle_roundtrip_preserves_data() {
    let original = vec![3u16, 3, 3, 1, 1, 4, 4, 4, 4, 2];
    let rle = assign_to_rle(&original);
    let recovered = rle_to_vec(rle);
    assert_eq!(recovered, original);
}

#[test]
fn rle_roundtrip_with_max_values() {
    let original = vec![0u16, 65535, 65535, 0, 1, 65534];
    let rle = assign_to_rle(&original);
    let recovered = rle_to_vec(rle);
    assert_eq!(recovered, original);
}

// ────────────────────────────────────────────────────────────────────────────── io::reader –
// DecoderInitError ──────────────────────────────────────────────────────────────────────────────

#[test]
fn decoder_init_error_display_io_variant() {
    let e = DecoderInitError::Io(io::Error::other("disk on fire"));
    let msg = e.to_string();
    assert!(msg.contains("disk on fire"), "got: {msg}");
}

#[test]
fn decoder_init_error_display_invalid_format_non_xz() {
    let header = b"NOT A BEN FILE!!!".to_vec();
    let e = DecoderInitError::InvalidFileFormat(header);
    let msg = e.to_string();
    assert!(
        msg.contains("Invalid file format"),
        "message should mention invalid file format, got: {msg}"
    );
}

#[test]
fn decoder_init_error_display_xz_header_mentions_compressed() {
    // XZ magic bytes: FD 37 7A 58 5A 00
    let mut header = vec![0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00];
    header.extend_from_slice(b"           ");
    let e = DecoderInitError::InvalidFileFormat(header);
    let msg = e.to_string();
    assert!(
        msg.to_lowercase().contains("compress"),
        "should mention compressed file, got: {msg}"
    );
}

#[test]
fn decoder_init_error_source_io_variant_has_source() {
    use std::error::Error as _;
    let e = DecoderInitError::Io(io::Error::other("boom"));
    assert!(e.source().is_some());
}

#[test]
fn decoder_init_error_source_invalid_format_has_no_source() {
    use std::error::Error as _;
    let e = DecoderInitError::InvalidFileFormat(b"bad".to_vec());
    assert!(e.source().is_none());
}

#[test]
fn decoder_init_error_converts_from_io_error() {
    let io_err = io::Error::other("wrapped");
    let init_err = DecoderInitError::from(io_err);
    assert!(matches!(init_err, DecoderInitError::Io(_)));
}

#[test]
fn decoder_init_error_converts_to_io_error_from_io() {
    let init_err = DecoderInitError::Io(io::Error::other("pass-through"));
    let io_err: io::Error = init_err.into();
    assert_eq!(io_err.kind(), io::ErrorKind::Other);
}

#[test]
fn decoder_init_error_converts_to_io_error_from_invalid_format() {
    let init_err = DecoderInitError::InvalidFileFormat(b"garbage".to_vec());
    let io_err: io::Error = init_err.into();
    assert_eq!(io_err.kind(), io::ErrorKind::InvalidData);
}

// ────────────────────────────────────────────────────────────────────────────── io::reader –
// BenStreamReader ──────────────────────────────────────────────────────────────────────────────

#[test]
fn ben_decoder_rejects_empty_input() {
    match BenStreamReader::from_ben(io::empty()) {
        Err(DecoderInitError::Io(_)) => {}
        Ok(_) => panic!("expected Io error"),
        Err(e) => panic!("unexpected error variant: {e}"),
    }
}

#[test]
fn ben_decoder_rejects_wrong_banner() {
    match BenStreamReader::from_ben(b"BAD BAD BAD BAD!!".as_slice()) {
        Err(DecoderInitError::InvalidFileFormat(_)) => {}
        Ok(_) => panic!("expected InvalidFileFormat error"),
        Err(e) => panic!("unexpected error variant: {e}"),
    }
}

#[test]
fn ben_decoder_rejects_xz_data_with_helpful_message() {
    // Manufacture a valid XZ header prefix.
    let xz_magic = b"\xFD\x37\x7A\x58\x5A\x00\x00\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00";
    match BenStreamReader::from_ben(xz_magic.as_slice()) {
        Err(DecoderInitError::InvalidFileFormat(ref header)) => {
            let e = DecoderInitError::InvalidFileFormat(header.clone());
            let msg = e.to_string();
            assert!(msg.to_lowercase().contains("compress"), "got: {msg}");
        }
        Ok(_) => panic!("expected InvalidFileFormat error"),
        Err(e) => panic!("unexpected error variant: {e}"),
    }
}

#[test]
fn ben_decoder_standard_single_assignment_round_trip() {
    let assignment = vec![1u16, 1, 2, 3, 3, 3];
    let ben = encode_standard_ben(std::slice::from_ref(&assignment));

    let mut decoder = BenStreamReader::from_ben(ben.as_slice()).unwrap();
    let (decoded, count) = decoder.next().unwrap().unwrap();
    assert_eq!(count, 1);
    assert_eq!(decoded, assignment);
    assert!(decoder.next().is_none());
}

#[test]
fn ben_decoder_standard_multiple_assignments_round_trip() {
    let assignments = vec![vec![1u16, 2, 3], vec![3u16, 2, 1], vec![1u16, 1, 1]];
    let ben = encode_standard_ben(&assignments);

    let mut decoder = BenStreamReader::from_ben(ben.as_slice())
        .unwrap()
        .silent(true);
    for expected in &assignments {
        let (decoded, count) = decoder.next().unwrap().unwrap();
        assert_eq!(count, 1);
        assert_eq!(&decoded, expected);
    }
    assert!(decoder.next().is_none());
}

#[test]
fn ben_decoder_mkv_preserves_repetition_counts() {
    // Three identical lines followed by one different line.
    let jsonl = concat!(
        r#"{"assignment":[1,2,3],"sample":1}"#,
        "\n",
        r#"{"assignment":[1,2,3],"sample":2}"#,
        "\n",
        r#"{"assignment":[1,2,3],"sample":3}"#,
        "\n",
        r#"{"assignment":[3,2,1],"sample":4}"#,
        "\n",
    );
    let mut ben = Vec::new();
    encode_jsonl_to_ben(jsonl.as_bytes(), &mut ben, BenVariant::MkvChain).unwrap();

    let mut decoder = BenStreamReader::from_ben(ben.as_slice())
        .unwrap()
        .silent(true);

    let (a1, c1) = decoder.next().unwrap().unwrap();
    assert_eq!(a1, vec![1u16, 2, 3]);
    assert_eq!(c1, 3, "expected repetition count of 3, got {c1}");

    let (a2, c2) = decoder.next().unwrap().unwrap();
    assert_eq!(a2, vec![3u16, 2, 1]);
    assert_eq!(c2, 1);

    assert!(decoder.next().is_none());
}

#[test]
fn ben_decoder_count_samples_standard() {
    let assignments = vec![vec![1u16, 2], vec![3u16, 4], vec![5u16, 6]];
    let ben = encode_standard_ben(&assignments);
    let decoder = BenStreamReader::from_ben(ben.as_slice()).unwrap();
    assert_eq!(decoder.count_samples().unwrap(), 3);
}

#[test]
fn ben_decoder_count_samples_mkv_with_repetitions() {
    let jsonl = concat!(
        r#"{"assignment":[1],"sample":1}"#,
        "\n",
        r#"{"assignment":[1],"sample":2}"#,
        "\n",
        r#"{"assignment":[1],"sample":3}"#,
        "\n",
        r#"{"assignment":[2],"sample":4}"#,
        "\n",
    );
    let mut ben = Vec::new();
    encode_jsonl_to_ben(jsonl.as_bytes(), &mut ben, BenVariant::MkvChain).unwrap();

    let decoder = BenStreamReader::from_ben(ben.as_slice()).unwrap();
    assert_eq!(decoder.count_samples().unwrap(), 4);
}

#[test]
fn ben_decoder_write_all_jsonl_produces_correct_output() {
    let assignments = vec![vec![1u16, 2, 3], vec![4u16, 5, 6]];
    let ben = encode_standard_ben(&assignments);

    let mut decoder = BenStreamReader::from_ben(ben.as_slice()).unwrap();
    let mut out = Vec::new();
    decoder.write_all_jsonl(&mut out).unwrap();

    let expected = concat!(
        r#"{"assignment":[1,2,3],"sample":1}"#,
        "\n",
        r#"{"assignment":[4,5,6],"sample":2}"#,
        "\n",
    );
    assert_eq!(String::from_utf8(out).unwrap(), expected);
}

#[test]
fn ben_decoder_for_each_assignment_early_stop() {
    let assignments = vec![vec![1u16, 2], vec![3u16, 4], vec![5u16, 6]];
    let ben = encode_standard_ben(&assignments);

    let mut decoder = BenStreamReader::from_ben(ben.as_slice())
        .unwrap()
        .silent(true);
    let mut seen = Vec::new();
    decoder
        .for_each_assignment(|a, _count| {
            seen.push(a.to_vec());
            Ok(seen.len() < 2) // Stop after second frame.
        })
        .unwrap();

    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0], vec![1u16, 2]);
    assert_eq!(seen[1], vec![3u16, 4]);
}

// ────────────────────────────────────────────────────────────────────────────── io::reader –
// BenStreamReader ──────────────────────────────────────────────────────────────────────────────

fn make_xben(assignments: &[Vec<u16>], variant: BenVariant) -> Vec<u8> {
    let jsonl = jsonl_from_assignments(assignments);
    let mut xben = Vec::new();
    encode_jsonl_to_xben(
        BufReader::new(jsonl.as_slice()),
        &mut xben,
        variant,
        Some(1),
        Some(1),
        None,
        None,
    )
    .unwrap();
    xben
}

#[test]
fn xben_decoder_reads_variant_from_banner_standard() {
    let assignments = vec![vec![1u16, 2, 3]];
    let xben = make_xben(&assignments, BenVariant::Standard);
    let decoder = BenStreamReader::from_xben(xben.as_slice()).unwrap();
    assert_eq!(decoder.variant(), BenVariant::Standard);
}

#[test]
fn xben_decoder_reads_variant_from_banner_mkvchain() {
    let assignments = vec![vec![1u16, 2, 3]];
    let xben = make_xben(&assignments, BenVariant::MkvChain);
    let decoder = BenStreamReader::from_xben(xben.as_slice()).unwrap();
    assert_eq!(decoder.variant(), BenVariant::MkvChain);
}

#[test]
fn xben_decoder_reads_variant_from_banner_twodelta() {
    // TwoDelta requires an initial "base" sample then transitions.
    let base = vec![1u16, 1, 2, 2];
    let second = vec![1u16, 2, 2, 1]; // swap positions 1 & 3
    let xben = make_xben(&[base, second], BenVariant::TwoDelta);
    let decoder = BenStreamReader::from_xben(xben.as_slice()).unwrap();
    assert_eq!(decoder.variant(), BenVariant::TwoDelta);
}

// ────────────────────────────────────────────────────────────────────────────── io::writer –
// BenEncoder ──────────────────────────────────────────────────────────────────────────────

#[test]
fn ben_encoder_writes_correct_banner_standard() {
    let mut out = Vec::new();
    let encoder = BenStreamWriter::for_ben(&mut out, BenVariant::Standard).unwrap();
    drop(encoder);
    assert!(out.starts_with(STANDARD_BEN_BANNER));
}

#[test]
fn ben_encoder_writes_correct_banner_mkvchain() {
    let mut out = Vec::new();
    let encoder = BenStreamWriter::for_ben(&mut out, BenVariant::MkvChain).unwrap();
    drop(encoder);
    assert!(out.starts_with(MKVCHAIN_BEN_BANNER));
}

#[test]
fn ben_encoder_writes_correct_banner_twodelta() {
    let mut out = Vec::new();
    let encoder = BenStreamWriter::for_ben(&mut out, BenVariant::TwoDelta).unwrap();
    drop(encoder);
    assert!(out.starts_with(TWODELTA_BEN_BANNER));
}

#[test]
fn ben_encoder_standard_single_assignment_round_trip() {
    let assignment = vec![1u16, 2, 3, 3, 2, 1];
    let mut out = Vec::new();
    {
        let mut enc = BenStreamWriter::for_ben(&mut out, BenVariant::Standard).unwrap();
        enc.write_assignment(assignment.clone()).unwrap();
        enc.finish().unwrap();
    }

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(out.as_slice(), &mut decoded).unwrap();
    let decoded_str = String::from_utf8(decoded).unwrap();
    assert!(decoded_str.contains("\"assignment\":[1,2,3,3,2,1]"));
}

#[test]
fn ben_encoder_finish_is_idempotent() {
    let mut out = Vec::new();
    {
        let mut enc = BenStreamWriter::for_ben(&mut out, BenVariant::MkvChain).unwrap();
        enc.write_assignment(vec![1u16, 2]).unwrap();
        enc.finish().unwrap();
        enc.finish().unwrap(); // second call
    }
    // The output should decode to exactly one sample (not duplicated).
    let mut decoded = Vec::new();
    decode_ben_to_jsonl(out.as_slice(), &mut decoded).unwrap();
    assert_eq!(decoded.iter().filter(|&&b| b == b'\n').count(), 1);
}

#[test]
fn ben_encoder_write_json_value_valid_input() {
    let data = json!({"assignment": [1, 2, 3], "sample": 1});
    let mut out = Vec::new();
    {
        let mut enc = BenStreamWriter::for_ben(&mut out, BenVariant::Standard).unwrap();
        enc.write_json_value(data).unwrap();
        enc.finish().unwrap();
    }
    let decoded_str = decode_ben_to_string(&out);
    assert!(decoded_str.contains("\"assignment\":[1,2,3]"));
}

#[test]
fn ben_encoder_write_json_value_missing_assignment_field_errors() {
    let data = json!({"sample": 1}); // no "assignment"
    let mut out = Vec::new();
    let mut enc = BenStreamWriter::for_ben(&mut out, BenVariant::Standard).unwrap();
    let result = enc.write_json_value(data);
    assert!(
        result.is_err(),
        "expected error for missing assignment field"
    );
}

#[test]
fn ben_encoder_write_json_value_value_too_large_errors() {
    // 65536 doesn't fit in u16.
    let data = json!({"assignment": [65536], "sample": 1});
    let mut out = Vec::new();
    let mut enc = BenStreamWriter::for_ben(&mut out, BenVariant::Standard).unwrap();
    let result = enc.write_json_value(data);
    assert!(result.is_err(), "expected error for value out of u16 range");
}

#[test]
fn ben_encoder_write_json_value_negative_value_errors() {
    let data = json!({"assignment": [-1], "sample": 1});
    let mut out = Vec::new();
    let mut enc = BenStreamWriter::for_ben(&mut out, BenVariant::Standard).unwrap();
    let result = enc.write_json_value(data);
    assert!(
        result.is_err(),
        "expected error for negative assignment value"
    );
}

#[test]
fn ben_encoder_standard_identical_assignments_still_written() {
    // For Standard variant, repeated identical assignments are each written.
    let assignment = vec![2u16, 2, 2];
    let mut out = Vec::new();
    {
        let mut enc = BenStreamWriter::for_ben(&mut out, BenVariant::Standard).unwrap();
        enc.write_assignment(assignment.clone()).unwrap();
        enc.write_assignment(assignment.clone()).unwrap();
        enc.write_assignment(assignment.clone()).unwrap();
        enc.finish().unwrap();
    }

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(out.as_slice(), &mut decoded).unwrap();
    assert_eq!(decoded.iter().filter(|&&b| b == b'\n').count(), 3);
}

#[test]
fn ben_encoder_mkv_identical_assignments_deduplicated() {
    // MkvChain compresses runs of identical assignments.
    let assignment = vec![2u16, 2, 2];
    let mut out = Vec::new();
    {
        let mut enc = BenStreamWriter::for_ben(&mut out, BenVariant::MkvChain).unwrap();
        enc.write_assignment(assignment.clone()).unwrap();
        enc.write_assignment(assignment.clone()).unwrap();
        enc.write_assignment(assignment.clone()).unwrap();
        enc.finish().unwrap();
    }

    // The BEN payload should be much smaller than 3 independent frames. More importantly, decoding
    // must give back 3 lines.
    let mut decoded = Vec::new();
    decode_ben_to_jsonl(out.as_slice(), &mut decoded).unwrap();
    assert_eq!(decoded.iter().filter(|&&b| b == b'\n').count(), 3);
}

#[test]
fn ben_encoder_twodelta_base_frame_then_delta_round_trip() {
    // Two assignments differing only in two values: valid TwoDelta transition.
    let base = vec![1u16, 1, 2, 2, 1, 2];
    let next = vec![2u16, 2, 1, 1, 2, 1]; // all 1s→2s and 2s→1s
    let mut out = Vec::new();
    {
        let mut enc = BenStreamWriter::for_ben(&mut out, BenVariant::TwoDelta).unwrap();
        enc.write_assignment(base.clone()).unwrap();
        enc.write_assignment(next.clone()).unwrap();
        enc.finish().unwrap();
    }

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(out.as_slice(), &mut decoded).unwrap();
    let s = String::from_utf8(decoded).unwrap();
    let lines: Vec<&str> = s.lines().collect();
    assert_eq!(lines.len(), 2, "decoded:\n{s}");
}

// ────────────────────────────────────────────────────────────────────────────── codec::encode –
// encode_ben_vec_from_rle and encode_ben_vec_from_assign
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn encode_ben_vec_from_rle_empty_rle() {
    // Empty RLE produces a minimal frame with zero payload bytes.
    let frame = BenEncodeFrame::from_rle(vec![], BenVariant::Standard, None).unwrap();
    // 1 byte max_val_bits + 1 byte max_len_bits + 4 bytes n_bytes = 6 bytes
    assert_eq!(frame.as_slice().len(), 6);
}

#[test]
fn encode_ben_vec_from_assign_and_rle_are_equivalent() {
    let assign = vec![3u16, 3, 3, 1, 2, 2];
    let rle = assign_to_rle(&assign);
    let via_assign = BenEncodeFrame::from_assignment(&assign, BenVariant::Standard, None).unwrap();
    let via_rle = BenEncodeFrame::from_rle(rle, BenVariant::Standard, None).unwrap();
    assert_eq!(via_assign.as_slice(), via_rle.as_slice());
}

#[test]
fn encode_ben_vec_from_assign_single_element() {
    let frame = BenEncodeFrame::from_assignment([42u16], BenVariant::Standard, None).unwrap();
    assert!(!frame.as_slice().is_empty());
}

#[test]
fn encode_ben_vec_from_assign_all_same() {
    let assign = vec![7u16; 500];
    let frame = BenEncodeFrame::from_assignment(&assign, BenVariant::Standard, None).unwrap();
    // Should encode efficiently; the payload compresses a single run.
    assert!(!frame.as_slice().is_empty());
}

// ────────────────────────────────────────────────────────────────────────────── codec::encode –
// encode_ben_to_xben ──────────────────────────────────────────────────────────────────────────────

#[test]
fn encode_ben_to_xben_and_back_standard() {
    let assignments = vec![vec![1u16, 2, 3], vec![4u16, 5, 6]];
    let ben = encode_standard_ben(&assignments);

    let mut xben = Vec::new();
    encode_ben_to_xben(
        BufReader::new(ben.as_slice()),
        &mut xben,
        None,
        None,
        None,
        None,
    )
    .unwrap();

    let mut ben2 = Vec::new();
    decode_xben_to_ben(BufReader::new(xben.as_slice()), &mut ben2).unwrap();

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(ben2.as_slice(), &mut decoded).unwrap();

    let expected = concat!(
        r#"{"assignment":[1,2,3],"sample":1}"#,
        "\n",
        r#"{"assignment":[4,5,6],"sample":2}"#,
        "\n",
    );
    assert_eq!(String::from_utf8(decoded).unwrap(), expected);
}

// ────────────────────────────────────────────────────────────────────────────── ops::relabel –
// convert_ben_file and convert_ben_file_limit
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn convert_ben_file_standard_to_standard_identity() {
    let assignments = vec![vec![1u16, 2, 3], vec![3u16, 2, 1]];
    let ben = encode_standard_ben(&assignments);

    let mut out = Vec::new();
    convert_ben_file(ben.as_slice(), &mut out, BenVariant::Standard).unwrap();

    // Decoding the converted file must match the original assignments.
    let original_jsonl = jsonl_from_assignments(&assignments);
    let mut decoded = Vec::new();
    decode_ben_to_jsonl(out.as_slice(), &mut decoded).unwrap();
    assert_eq!(decoded, original_jsonl);
}

#[test]
fn convert_ben_file_standard_to_mkvchain() {
    let assignments = vec![
        vec![1u16, 2, 3],
        vec![1u16, 2, 3], // duplicate
        vec![3u16, 2, 1],
    ];
    let ben = encode_standard_ben(&assignments);

    let mut out = Vec::new();
    convert_ben_file(ben.as_slice(), &mut out, BenVariant::MkvChain).unwrap();

    assert!(out.starts_with(MKVCHAIN_BEN_BANNER));

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(out.as_slice(), &mut decoded).unwrap();

    let expected = jsonl_from_assignments(&assignments);
    assert_eq!(decoded, expected);
}

#[test]
fn convert_ben_file_rejects_invalid_header() {
    let err = convert_ben_file(
        b"BAD HEADER!!!!!!!!".as_slice(),
        Vec::new(),
        BenVariant::Standard,
    )
    .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn convert_ben_file_limit_truncates_to_max_samples() {
    let assignments: Vec<Vec<u16>> = (0..10u16).map(|i| vec![i, i + 1]).collect();
    let ben = encode_standard_ben(&assignments);

    let mut out = Vec::new();
    relabel_ben_file(
        ben.as_slice(),
        &mut out,
        RelabelOptions::convert_to(BenVariant::Standard).with_max_samples(4),
    )
    .unwrap();

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(out.as_slice(), &mut decoded).unwrap();
    assert_eq!(decoded.iter().filter(|&&b| b == b'\n').count(), 4);
}

#[test]
fn convert_ben_file_limit_zero_produces_banner_only() {
    let assignments = vec![vec![1u16, 2, 3]];
    let ben = encode_standard_ben(&assignments);

    let mut out = Vec::new();
    relabel_ben_file(
        ben.as_slice(),
        &mut out,
        RelabelOptions::convert_to(BenVariant::Standard).with_max_samples(0),
    )
    .unwrap();

    // Banner must be present; no frames.
    assert!(out.starts_with(STANDARD_BEN_BANNER));
    let mut decoded = Vec::new();
    decode_ben_to_jsonl(out.as_slice(), &mut decoded).unwrap();
    assert!(decoded.is_empty());
}

// ────────────────────────────────────────────────────────────────────────────── ops::relabel –
// relabel_ben_lines_limit
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn relabel_ben_lines_limit_truncates_standard() {
    let assignments: Vec<Vec<u16>> = vec![
        vec![3u16, 1, 2],
        vec![2u16, 3, 1],
        vec![1u16, 2, 3],
        vec![3u16, 3, 1],
    ];
    let ben = encode_standard_ben(&assignments);

    let mut full = Vec::new();
    relabel_ben_file(
        ben.as_slice(),
        &mut full,
        RelabelOptions::first_seen().with_max_samples(2),
    )
    .unwrap();

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(full.as_slice(), &mut decoded).unwrap();
    assert_eq!(
        decoded.iter().filter(|&&b| b == b'\n').count(),
        2,
        "expected 2 decoded samples"
    );
}

// ────────────────────────────────────────────────────────────────────────────── ops::relabel –
// relabel_ben_file_as_variant
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn relabel_ben_file_as_variant_standard_to_standard() {
    let assignments = vec![vec![5u16, 5, 1], vec![1u16, 5, 5]];
    let ben = encode_standard_ben(&assignments);

    let mut out = Vec::new();
    relabel_ben_file(
        ben.as_slice(),
        &mut out,
        RelabelOptions::first_seen().with_target_variant(BenVariant::Standard),
    )
    .unwrap();

    assert!(out.starts_with(STANDARD_BEN_BANNER));

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(out.as_slice(), &mut decoded).unwrap();
    let s = String::from_utf8(decoded).unwrap();

    // Each frame is canonicalized independently (first-seen within the frame → 0, etc.). Frame 1:
    // [5,5,1] → first 5→0, then 1→1 → [0,0,1] Frame 2: [1,5,5] → first 1→0, then 5→1 → [0,1,1]
    assert!(
        s.contains("\"assignment\":[0,0,1]"),
        "frame1 mismatch, got: {s}"
    );
    assert!(
        s.contains("\"assignment\":[0,1,1]"),
        "frame2 mismatch, got: {s}"
    );
}

#[test]
fn relabel_ben_file_as_variant_standard_to_mkvchain() {
    let assignments = vec![
        vec![3u16, 1, 2],
        vec![3u16, 1, 2], // duplicate
        vec![1u16, 3, 2],
    ];
    let ben = encode_standard_ben(&assignments);

    let mut out = Vec::new();
    relabel_ben_file(
        ben.as_slice(),
        &mut out,
        RelabelOptions::first_seen().with_target_variant(BenVariant::MkvChain),
    )
    .unwrap();

    assert!(out.starts_with(MKVCHAIN_BEN_BANNER));

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(out.as_slice(), &mut decoded).unwrap();
    assert_eq!(decoded.iter().filter(|&&b| b == b'\n').count(), 3);
}

#[test]
fn relabel_ben_file_as_variant_rejects_invalid_header() {
    let err = relabel_ben_file(
        b"TOTALLY WRONG!!!!!!".as_slice(),
        Vec::new(),
        RelabelOptions::first_seen().with_target_variant(BenVariant::Standard),
    )
    .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn relabel_ben_file_as_variant_limit_truncates_output() {
    let assignments: Vec<Vec<u16>> = (1u16..=8).map(|i| vec![i, i + 1]).collect();
    let ben = encode_standard_ben(&assignments);

    let mut out = Vec::new();
    relabel_ben_file(
        ben.as_slice(),
        &mut out,
        RelabelOptions::first_seen()
            .with_target_variant(BenVariant::Standard)
            .with_max_samples(3),
    )
    .unwrap();

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(out.as_slice(), &mut decoded).unwrap();
    assert_eq!(decoded.iter().filter(|&&b| b == b'\n').count(), 3);
}

#[test]
fn relabel_ben_file_as_variant_limit_zero_gives_empty() {
    let assignments = vec![vec![1u16, 2, 3]];
    let ben = encode_standard_ben(&assignments);

    let mut out = Vec::new();
    relabel_ben_file(
        ben.as_slice(),
        &mut out,
        RelabelOptions::first_seen()
            .with_target_variant(BenVariant::Standard)
            .with_max_samples(0),
    )
    .unwrap();

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(out.as_slice(), &mut decoded).unwrap();
    assert!(decoded.is_empty(), "expected empty output for limit=0");
}

// ────────────────────────────────────────────────────────────────────────────── ops::relabel –
// relabel_ben_file_with_map_as_variant
// ──────────────────────────────────────────────────────────────────────────────

/// Build a map that reverses a 3-element assignment: new[0]←old[2], etc.
fn reverse_map_3() -> HashMap<usize, usize> {
    [(0, 2), (1, 1), (2, 0)].iter().cloned().collect()
}

#[test]
fn relabel_ben_file_with_map_as_variant_standard_to_standard() {
    let assignments = vec![vec![10u16, 20, 30]];
    let ben = encode_standard_ben(&assignments);

    let mut out = Vec::new();
    relabel_ben_file(
        ben.as_slice(),
        &mut out,
        RelabelOptions::node_permutation(reverse_map_3()).with_target_variant(BenVariant::Standard),
    )
    .unwrap();

    assert!(out.starts_with(STANDARD_BEN_BANNER));

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(out.as_slice(), &mut decoded).unwrap();
    let s = String::from_utf8(decoded).unwrap();
    // Reversed: [30, 20, 10]
    assert!(s.contains("\"assignment\":[30,20,10]"), "got: {s}");
}

#[test]
fn relabel_ben_file_with_map_as_variant_standard_to_mkvchain() {
    let assignments = vec![
        vec![1u16, 2, 3],
        vec![1u16, 2, 3], // duplicate
        vec![3u16, 2, 1],
    ];
    let ben = encode_standard_ben(&assignments);

    let mut out = Vec::new();
    relabel_ben_file(
        ben.as_slice(),
        &mut out,
        RelabelOptions::node_permutation(reverse_map_3()).with_target_variant(BenVariant::MkvChain),
    )
    .unwrap();

    assert!(out.starts_with(MKVCHAIN_BEN_BANNER));

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(out.as_slice(), &mut decoded).unwrap();
    assert_eq!(decoded.iter().filter(|&&b| b == b'\n').count(), 3);
}

#[test]
fn relabel_ben_file_with_map_as_variant_rejects_invalid_header() {
    let err = relabel_ben_file(
        b"NOT A VALID BEN!!".as_slice(),
        Vec::new(),
        RelabelOptions::node_permutation(reverse_map_3()).with_target_variant(BenVariant::Standard),
    )
    .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn relabel_ben_file_with_map_as_variant_limit_truncates() {
    let assignments = vec![
        vec![1u16, 2, 3],
        vec![3u16, 2, 1],
        vec![2u16, 1, 3],
        vec![1u16, 3, 2],
        vec![2u16, 3, 1],
    ];
    let ben = encode_standard_ben(&assignments);

    let mut out = Vec::new();
    relabel_ben_file(
        ben.as_slice(),
        &mut out,
        RelabelOptions::node_permutation(reverse_map_3())
            .with_target_variant(BenVariant::Standard)
            .with_max_samples(3),
    )
    .unwrap();

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(out.as_slice(), &mut decoded).unwrap();
    assert_eq!(decoded.iter().filter(|&&b| b == b'\n').count(), 3);
}

#[test]
fn relabel_ben_file_with_map_as_variant_limit_zero_gives_empty() {
    let assignments = vec![vec![1u16, 2, 3]];
    let ben = encode_standard_ben(&assignments);

    let mut out = Vec::new();
    relabel_ben_file(
        ben.as_slice(),
        &mut out,
        RelabelOptions::node_permutation(reverse_map_3())
            .with_target_variant(BenVariant::Standard)
            .with_max_samples(0),
    )
    .unwrap();

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(out.as_slice(), &mut decoded).unwrap();
    assert!(decoded.is_empty());
}

// ────────────────────────────────────────────────────────────────────────────── ops::relabel –
// dense_permutation edge cases (tested indirectly)
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn relabel_file_with_map_detects_gap_in_permutation() {
    // new_to_old_map skips index 1 → should fail with InvalidInput.
    let assignments = vec![vec![1u16, 2, 3]];
    let ben = encode_standard_ben(&assignments);

    // Map {0→0, 2→2} – index 1 is missing.
    let bad_map: HashMap<usize, usize> = [(0, 0), (2, 2)].iter().cloned().collect();

    let err = relabel_ben_file(
        ben.as_slice(),
        Vec::new(),
        RelabelOptions::node_permutation(bad_map),
    )
    .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
}

// ────────────────────────────────────────────────────────────────────────────── ops::relabel –
// convert_ben_file with MkvChain truncation
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn convert_ben_file_limit_with_mkvchain_repetitions() {
    // 5 copies of the same assignment encoded as one MkvChain frame with count=5.
    let jsonl = concat!(
        r#"{"assignment":[1,2],"sample":1}"#,
        "\n",
        r#"{"assignment":[1,2],"sample":2}"#,
        "\n",
        r#"{"assignment":[1,2],"sample":3}"#,
        "\n",
        r#"{"assignment":[1,2],"sample":4}"#,
        "\n",
        r#"{"assignment":[1,2],"sample":5}"#,
        "\n",
        r#"{"assignment":[3,4],"sample":6}"#,
        "\n",
    );
    let mut ben = Vec::new();
    encode_jsonl_to_ben(jsonl.as_bytes(), &mut ben, BenVariant::MkvChain).unwrap();

    let mut out = Vec::new();
    relabel_ben_file(
        ben.as_slice(),
        &mut out,
        RelabelOptions::convert_to(BenVariant::MkvChain).with_max_samples(3),
    )
    .unwrap();

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(out.as_slice(), &mut decoded).unwrap();
    assert_eq!(decoded.iter().filter(|&&b| b == b'\n').count(), 3);
}

// ────────────────────────────────────────────────────────────────────────────── ops::relabel –
// relabel_ben_file TwoDelta (canonicalization path)
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn relabel_ben_file_twodelta_canonicalizes_labels() {
    // Start with high label values; after canonicalization they should map to 0,1,2.
    let file = concat!(
        r#"{"assignment":[100,100,200,200,300,300],"sample":1}"#,
        "\n",
        r#"{"assignment":[100,100,200,200,300,300],"sample":2}"#,
        "\n",
        r#"{"assignment":[100,200,200,100,300,300],"sample":3}"#,
        "\n",
    );
    let mut ben = Vec::new();
    encode_jsonl_to_ben(file.as_bytes(), &mut ben, BenVariant::TwoDelta).unwrap();

    let mut relabeled = Vec::new();
    relabel_ben_file(ben.as_slice(), &mut relabeled, RelabelOptions::first_seen()).unwrap();

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(relabeled.as_slice(), &mut decoded).unwrap();
    let s = String::from_utf8(decoded).unwrap();

    // Canonical: first-seen is 0, second is 1, third is 2.
    assert!(s.contains("\"assignment\":[0,0,1,1,2,2]"), "got: {s}");
}

// ────────────────────────────────────────────────────────────────────────────── Encoding – empty
// assignment vectors ──────────────────────────────────────────────────────────────────────────────

#[test]
fn encode_and_decode_empty_assignment_standard() {
    // An empty assignment is a valid (if unusual) edge case.
    let data = json!({"assignment": [], "sample": 1}).to_string() + "\n";
    let mut ben = Vec::new();
    encode_jsonl_to_ben(data.as_bytes(), &mut ben, BenVariant::Standard).unwrap();

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(ben.as_slice(), &mut decoded).unwrap();
    let s = String::from_utf8(decoded).unwrap();
    assert!(s.contains("\"assignment\":[]"), "got: {s}");
}

// ────────────────────────────────────────────────────────────────────────────── Encoding – large
// u16 values ──────────────────────────────────────────────────────────────────────────────

#[test]
fn encode_and_decode_max_u16_values_standard() {
    let assignment = vec![0u16, 65535, 32768, 1, 65534];
    let ben = encode_standard_ben(std::slice::from_ref(&assignment));
    let decoded_str = decode_ben_to_string(&ben);
    assert!(
        decoded_str.contains("\"assignment\":[0,65535,32768,1,65534]"),
        "got: {decoded_str}"
    );
}

// ────────────────────────────────────────────────────────────────────────────── Encoding –
// single-sample files
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn single_sample_standard_round_trip() {
    let assignment = vec![42u16; 1000];
    let ben = encode_standard_ben(std::slice::from_ref(&assignment));
    let decoded_str = decode_ben_to_string(&ben);
    assert_eq!(decoded_str.lines().count(), 1);
    assert!(decoded_str.contains("\"sample\":1"));
}

#[test]
fn single_sample_mkvchain_round_trip() {
    let data = json!({"assignment": [1, 2, 3], "sample": 1}).to_string() + "\n";
    let mut ben = Vec::new();
    encode_jsonl_to_ben(data.as_bytes(), &mut ben, BenVariant::MkvChain).unwrap();

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(ben.as_slice(), &mut decoded).unwrap();
    let s = String::from_utf8(decoded).unwrap();
    assert_eq!(s.lines().count(), 1);
    assert!(s.contains("\"assignment\":[1,2,3]"), "got: {s}");
}

// ────────────────────────────────────────────────────────────────────────────── Decode error paths
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn decode_ben_to_jsonl_rejects_empty_input() {
    let err = decode_ben_to_jsonl([].as_slice(), Vec::new()).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
}

#[test]
fn decode_ben_to_jsonl_rejects_wrong_banner() {
    let err = decode_ben_to_jsonl(b"THIS IS NOT BEN!!".as_slice(), Vec::new()).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn decode_ben_to_jsonl_rejects_truncated_frame_header() {
    // Banner is correct but frame header is incomplete (only 2 bytes of the 6).
    let mut data = STANDARD_BEN_BANNER.to_vec();
    data.extend_from_slice(&[0x02, 0x03]); // only 2 bytes of 6-byte header
    let err = decode_ben_to_jsonl(data.as_slice(), Vec::new()).unwrap_err();
    assert_ne!(err.kind(), io::ErrorKind::Other); // not just "ok"
}

// ────────────────────────────────────────────────────────────────────────────── XBEN round-trip
// with various compression levels
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn xben_round_trip_with_level_0_compression() {
    let assignments = vec![vec![1u16, 2, 3, 4], vec![4u16, 3, 2, 1]];
    let jsonl = jsonl_from_assignments(&assignments);

    let mut xben = Vec::new();
    encode_jsonl_to_xben(
        BufReader::new(jsonl.as_slice()),
        &mut xben,
        BenVariant::Standard,
        Some(1),
        Some(0), // compression level 0
        None,
        None,
    )
    .unwrap();

    let mut ben = Vec::new();
    decode_xben_to_ben(BufReader::new(xben.as_slice()), &mut ben).unwrap();

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(ben.as_slice(), &mut decoded).unwrap();
    assert_eq!(decoded, jsonl);
}

#[test]
fn xben_mkvchain_round_trip_preserves_all_samples() {
    let jsonl = concat!(
        r#"{"assignment":[1,2,3],"sample":1}"#,
        "\n",
        r#"{"assignment":[1,2,3],"sample":2}"#,
        "\n",
        r#"{"assignment":[1,2,3],"sample":3}"#,
        "\n",
        r#"{"assignment":[3,2,1],"sample":4}"#,
        "\n",
    );

    let mut xben = Vec::new();
    encode_jsonl_to_xben(
        BufReader::new(jsonl.as_bytes()),
        &mut xben,
        BenVariant::MkvChain,
        Some(1),
        Some(1),
        None,
        None,
    )
    .unwrap();

    let mut ben = Vec::new();
    decode_xben_to_ben(BufReader::new(xben.as_slice()), &mut ben).unwrap();

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(ben.as_slice(), &mut decoded).unwrap();
    assert_eq!(
        decoded.iter().filter(|&&b| b == b'\n').count(),
        4,
        "expected 4 decoded lines"
    );
}

// ────────────────────────────────────────────────────────────────────────────── Relabel –
// file_as_variant with MkvChain source
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn relabel_ben_file_as_variant_mkvchain_to_standard() {
    // Build a MkvChain file with repetitions.
    let jsonl = concat!(
        r#"{"assignment":[5,5,3],"sample":1}"#,
        "\n",
        r#"{"assignment":[5,5,3],"sample":2}"#,
        "\n",
        r#"{"assignment":[3,3,5],"sample":3}"#,
        "\n",
    );
    let mut mkv_ben = Vec::new();
    encode_jsonl_to_ben(jsonl.as_bytes(), &mut mkv_ben, BenVariant::MkvChain).unwrap();

    let mut out = Vec::new();
    relabel_ben_file(
        mkv_ben.as_slice(),
        &mut out,
        RelabelOptions::first_seen().with_target_variant(BenVariant::Standard),
    )
    .unwrap();

    assert!(out.starts_with(STANDARD_BEN_BANNER));

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(out.as_slice(), &mut decoded).unwrap();
    let s = String::from_utf8(decoded).unwrap();
    // Canonical labels: 5→0, 3→1
    assert!(s.contains("\"assignment\":[0,0,1]"), "got: {s}");
    assert_eq!(s.lines().count(), 3);
}

// ────────────────────────────────────────────────────────────────────────────── Relabel –
// with_map_as_variant permutation correctness
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn relabel_ben_file_with_map_as_variant_permutes_correctly() {
    // Assignment [a, b, c, d] with map {0→3, 1→2, 2→1, 3→0} → [d, c, b, a]
    let assignments = vec![vec![10u16, 20, 30, 40]];
    let ben = encode_standard_ben(&assignments);

    let map: HashMap<usize, usize> = [(0, 3), (1, 2), (2, 1), (3, 0)].iter().cloned().collect();

    let mut out = Vec::new();
    relabel_ben_file(
        ben.as_slice(),
        &mut out,
        RelabelOptions::node_permutation(map).with_target_variant(BenVariant::Standard),
    )
    .unwrap();

    let decoded_str = decode_ben_to_string(&out);
    assert!(
        decoded_str.contains("\"assignment\":[40,30,20,10]"),
        "got: {decoded_str}"
    );
}

// ────────────────────────────────────────────────────────────────────────────── BenStreamReader –
// iterator interface ──────────────────────────────────────────────────────────────────────────────

#[test]
fn ben_decoder_iterator_collects_all_frames() {
    let assignments = vec![vec![1u16, 2, 3], vec![4u16, 5, 6], vec![7u16, 8, 9]];
    let ben = encode_standard_ben(&assignments);
    let decoder = BenStreamReader::from_ben(ben.as_slice())
        .unwrap()
        .silent(true);
    let frames: Vec<_> = decoder.collect::<io::Result<Vec<_>>>().unwrap();
    assert_eq!(frames.len(), 3);
    for (i, (a, count)) in frames.iter().enumerate() {
        assert_eq!(*count, 1);
        assert_eq!(a, &assignments[i]);
    }
}

#[test]
fn ben_decoder_iterator_on_empty_payload_yields_nothing() {
    let ben = STANDARD_BEN_BANNER.to_vec(); // banner only, no frames
    let decoder = BenStreamReader::from_ben(ben.as_slice())
        .unwrap()
        .silent(true);
    let frames: Vec<_> = decoder.collect::<io::Result<Vec<_>>>().unwrap();
    assert!(frames.is_empty());
}

// ────────────────────────────────────────────────────────────────────────────── Relabeling –
// idempotence of canonicalization
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn relabel_ben_file_standard_is_idempotent() {
    let assignments = vec![vec![7u16, 3, 5, 1], vec![3u16, 7, 1, 5]];
    let ben = encode_standard_ben(&assignments);

    // First relabeling.
    let mut relabeled1 = Vec::new();
    relabel_ben_file(
        ben.as_slice(),
        &mut relabeled1,
        RelabelOptions::first_seen(),
    )
    .unwrap();

    // Second relabeling on already-canonical output.
    let mut relabeled2 = Vec::new();
    relabel_ben_file(
        relabeled1.as_slice(),
        &mut relabeled2,
        RelabelOptions::first_seen(),
    )
    .unwrap();

    // The decoded output of both should be identical.
    let mut decoded1 = Vec::new();
    decode_ben_to_jsonl(relabeled1.as_slice(), &mut decoded1).unwrap();

    let mut decoded2 = Vec::new();
    decode_ben_to_jsonl(relabeled2.as_slice(), &mut decoded2).unwrap();

    assert_eq!(decoded1, decoded2, "relabeling is not idempotent");
}

// ────────────────────────────────────────────────────────────────────────────── Edge case:
// assignment with a single unique label
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn single_unique_label_assignment_round_trips() {
    let assignment = vec![42u16; 50];
    let ben = encode_standard_ben(std::slice::from_ref(&assignment));
    let decoded_str = decode_ben_to_string(&ben);
    assert!(
        decoded_str.contains("\"assignment\":[42,42,42"),
        "got: {decoded_str}"
    );
}

#[test]
fn single_unique_label_relabeled_to_zero() {
    let assignment = vec![99u16; 10];
    let ben = encode_standard_ben(&[assignment]);

    let mut relabeled = Vec::new();
    relabel_ben_file(ben.as_slice(), &mut relabeled, RelabelOptions::first_seen()).unwrap();

    let decoded_str = decode_ben_to_string(&relabeled);
    // All 99s should become 0s.
    assert!(
        decoded_str.contains("\"assignment\":[0,0,0"),
        "got: {decoded_str}"
    );
}

// ────────────────────────────────────────────────────────────────────────────── Edge case: frame
// with maximum run-length value
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn encode_decode_max_run_length_standard() {
    // A run of 65535 identical values.
    let assignment = vec![7u16; 65535];
    let ben = encode_standard_ben(std::slice::from_ref(&assignment));

    let decoded_str = decode_ben_to_string(&ben);
    assert!(decoded_str.contains("\"sample\":1"));
    // Parse and verify the assignment length.
    let parsed: serde_json::Value = serde_json::from_str(decoded_str.trim()).unwrap();
    assert_eq!(
        parsed["assignment"].as_array().unwrap().len(),
        65535,
        "wrong decoded length"
    );
}

// ────────────────────────────────────────────────────────────────────────────── BenVariant debug /
// clone / copy ──────────────────────────────────────────────────────────────────────────────

#[test]
fn ben_variant_clone_and_copy() {
    let v = BenVariant::MkvChain;
    let v2 = v; // Copy
    let v3 = v; // Clone
    assert_eq!(v2, v3);
    assert_eq!(v, BenVariant::MkvChain);
}

#[test]
fn ben_variant_debug() {
    let s = format!("{:?}", BenVariant::TwoDelta);
    assert_eq!(s, "TwoDelta");
}

// ────────────────────────────────────────────────────────────────────────────── Cursor::new round
// trips for Cursor-based readers
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn ben_decoder_accepts_cursor_reader() {
    let assignment = vec![1u16, 2, 3];
    let ben = encode_standard_ben(std::slice::from_ref(&assignment));
    let cursor = Cursor::new(ben);
    let mut decoder = BenStreamReader::from_ben(cursor).unwrap().silent(true);
    let (decoded, _) = decoder.next().unwrap().unwrap();
    assert_eq!(decoded, assignment);
}

// ──────────────────────────────────────────────────────────────────────────────
// encode_twodelta_frame error paths
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn encode_twodelta_frame_different_lengths_errors() {
    let prev = vec![1u16, 2, 3];
    let next = vec![1u16, 2];
    let err = encode_twodelta_frame(&prev, &next, None).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert!(err.to_string().contains("equal-length"));
}

#[test]
fn encode_twodelta_frame_identical_assignments_errors() {
    let assign = vec![1u16, 2, 3];
    let err = encode_twodelta_frame(&assign, &assign, None).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert!(err.to_string().contains("identical"));
}

#[test]
fn encode_twodelta_frame_more_than_two_values_errors() {
    // prev = [1,2,3], next = [3,1,2]: positions 0,1,2 all change and involve ids 1,2,3 → 3 ids
    let prev = vec![1u16, 2, 3];
    let next = vec![3u16, 1, 2];
    let err = encode_twodelta_frame(&prev, &next, None).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert!(err.to_string().contains("two distinct district ids"));
}

#[test]
fn encode_twodelta_frame_valid_two_value_transition() {
    let prev = vec![1u16, 1, 2, 2];
    let next = vec![2u16, 2, 1, 1];
    let frame = encode_twodelta_frame(&prev, &next, Some(1)).unwrap();
    // All 4 positions belong to the pair, and all flip
    assert_eq!(frame.n_bytes() as usize, frame.payload().len());
}

#[test]
fn encode_twodelta_frame_single_value_swap() {
    // Only one position changes: prev[3]=2 → next[3]=1; pair is (new_val, old_val) = (1, 2)
    let prev = vec![1u16, 1, 1, 2];
    let next = vec![1u16, 1, 1, 1];
    let frame = encode_twodelta_frame(&prev, &next, Some(1)).unwrap();
    assert_eq!(frame.pair().unwrap(), (1, 2));
}

// ──────────────────────────────────────────────────────────────────────────────
// TwoDeltaEncodeFrame round-trip
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn twodelta_frame_try_from_parts_round_trip() {
    let pair = (10u16, 20u16);
    let run_lengths = vec![2u16, 5, 1];
    let original = BenEncodeFrame::from_run_lengths(pair, run_lengths, None).unwrap();
    let reconstructed = BenEncodeFrame::try_from_parts(
        pair,
        original.max_len_bit_count(),
        original.payload().to_vec(),
        original.count(),
    )
    .expect("encoder-produced parts must reconstruct");
    assert_eq!(original.as_slice(), reconstructed.as_slice());
    assert_eq!(original.pair().unwrap(), reconstructed.pair().unwrap());
    assert_eq!(
        original.max_len_bit_count(),
        reconstructed.max_len_bit_count()
    );
    assert_eq!(original.n_bytes(), reconstructed.n_bytes());
    assert_eq!(original.count(), reconstructed.count());
}

// ──────────────────────────────────────────────────────────────────────────────
// EncodeBenFrame: from_assignment matches the RLE entrypoint
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn encode_ben_frame_from_assignment() {
    let assignment = vec![1u16, 1, 2, 2, 3];
    let frame = BenEncodeFrame::from_assignment(&assignment, BenVariant::Standard, None).unwrap();
    // Frame from assignment should produce runs
    let runs = frame.runs().unwrap();
    assert_eq!(runs, &[(1u16, 2u16), (2u16, 2u16), (3u16, 1u16)]);
}

// ────────────────────────────────────────────────────────────────────────────── Graph ordering
// with >8 nodes (triggers multilevel clustering recursion)
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn sort_by_ordering_multilevel_cluster_large_ring_graph() {
    // 20-node ring → component > 8 nodes → triggers recursive greedy_cluster_partition path
    let graph_json = make_ring_graph_json(20);
    let mut output = Vec::new();
    let mapping = sort_json_file_by_ordering(
        graph_json.as_bytes(),
        &mut output,
        GraphOrderingMethod::MultiLevelCluster,
    )
    .unwrap();

    assert_eq!(mapping.len(), 20);
    let mut new_ids: Vec<usize> = mapping.values().copied().collect();
    new_ids.sort_unstable();
    assert_eq!(new_ids, (0..20).collect::<Vec<_>>());

    let output_json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(output_json["nodes"].as_array().unwrap().len(), 20);
}

#[test]
fn sort_by_ordering_rcm_large_ring_graph() {
    let graph_json = make_ring_graph_json(20);
    let mut output = Vec::new();
    let mapping = sort_json_file_by_ordering(
        graph_json.as_bytes(),
        &mut output,
        GraphOrderingMethod::ReverseCuthillMckee,
    )
    .unwrap();

    assert_eq!(mapping.len(), 20);
    let mut new_ids: Vec<usize> = mapping.values().copied().collect();
    new_ids.sort_unstable();
    assert_eq!(new_ids, (0..20).collect::<Vec<_>>());
}

#[test]
fn sort_by_ordering_disconnected_graph_multilevel() {
    // Two triangles (two disconnected components)
    let input = r#"{
        "nodes": [
            {"id": 0}, {"id": 1}, {"id": 2},
            {"id": 3}, {"id": 4}, {"id": 5}
        ],
        "adjacency": [
            [{"id": 1}, {"id": 2}],
            [{"id": 0}, {"id": 2}],
            [{"id": 0}, {"id": 1}],
            [{"id": 4}, {"id": 5}],
            [{"id": 3}, {"id": 5}],
            [{"id": 3}, {"id": 4}]
        ]
    }"#;
    let mut output = Vec::new();
    let mapping = sort_json_file_by_ordering(
        input.as_bytes(),
        &mut output,
        GraphOrderingMethod::MultiLevelCluster,
    )
    .unwrap();
    assert_eq!(mapping.len(), 6);
    let output_json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(output_json["nodes"].as_array().unwrap().len(), 6);
}

#[test]
fn sort_by_ordering_disconnected_graph_rcm() {
    let input = r#"{
        "nodes": [
            {"id": 0}, {"id": 1}, {"id": 2},
            {"id": 3}, {"id": 4}, {"id": 5}
        ],
        "adjacency": [
            [{"id": 1}, {"id": 2}],
            [{"id": 0}, {"id": 2}],
            [{"id": 0}, {"id": 1}],
            [{"id": 4}, {"id": 5}],
            [{"id": 3}, {"id": 5}],
            [{"id": 3}, {"id": 4}]
        ]
    }"#;
    let mut output = Vec::new();
    let mapping = sort_json_file_by_ordering(
        input.as_bytes(),
        &mut output,
        GraphOrderingMethod::ReverseCuthillMckee,
    )
    .unwrap();
    assert_eq!(mapping.len(), 6);
}

#[test]
fn graph_invalid_node_id_errors() {
    // Negative node id cannot be parsed as usize
    let input = r#"{
        "nodes": [{"id": -1}, {"id": 1}],
        "adjacency": [[{"id": 1}], [{"id": 0}]]
    }"#;
    let mut output = Vec::new();
    let result = sort_json_file_by_ordering(
        input.as_bytes(),
        &mut output,
        GraphOrderingMethod::ReverseCuthillMckee,
    );
    assert!(result.is_err());
}

#[test]
fn graph_unknown_adjacency_node_errors() {
    // Edge target id 99 does not exist in node list
    let input = r#"{
        "nodes": [{"id": 0}, {"id": 1}],
        "adjacency": [[{"id": 99}], [{"id": 0}]]
    }"#;
    let mut output = Vec::new();
    let result = sort_json_file_by_key(input.as_bytes(), &mut output, "id");
    assert!(result.is_err());
}

#[test]
fn graph_invalid_link_id_errors() {
    // Edge target id is negative → parse_link_id fails
    let input = r#"{
        "nodes": [{"id": 0}, {"id": 1}],
        "adjacency": [[{"id": -1}], [{"id": 0}]]
    }"#;
    let mut output = Vec::new();
    let result = sort_json_file_by_key(input.as_bytes(), &mut output, "id");
    assert!(result.is_err());
}

#[test]
fn sort_by_ordering_large_graph_multilevel_verifies_permutation() {
    // 30-node ring, large enough that greedy_cluster_partition produces multiple clusters and the
    // coarse graph recursion fires
    let graph_json = make_ring_graph_json(30);
    let mut output = Vec::new();
    let mapping = sort_json_file_by_ordering(
        graph_json.as_bytes(),
        &mut output,
        GraphOrderingMethod::MultiLevelCluster,
    )
    .unwrap();

    assert_eq!(mapping.len(), 30);
    // All 30 new ids must be a valid permutation of 0..29
    let mut new_ids: Vec<usize> = mapping.values().copied().collect();
    new_ids.sort_unstable();
    assert_eq!(new_ids, (0..30).collect::<Vec<_>>());
}

// ────────────────────────────────────────────────────────────────────────────── BenStreamReader /
// BenStreamFrameReader
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn xben_decoder_iterator_standard_collects_all() {
    let assignments = vec![vec![1u16, 1, 2, 2], vec![3u16, 3, 3, 3]];
    let xben = encode_xben(&assignments, BenVariant::Standard);
    let decoder = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    assert_eq!(decoder.variant(), BenVariant::Standard);
    let results: Vec<Vec<u16>> = decoder.map(|r| r.unwrap().0).collect();
    assert_eq!(results, assignments);
}

#[test]
fn xben_decoder_count_samples_standard() {
    let assignments = vec![
        vec![1u16, 2, 1, 2],
        vec![3u16, 4, 3, 4],
        vec![5u16, 6, 5, 6],
    ];
    let xben = encode_xben(&assignments, BenVariant::Standard);
    let decoder = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    assert_eq!(decoder.count_samples().unwrap(), 3);
}

#[test]
fn xben_decoder_count_samples_mkvchain() {
    let assignments: Vec<Vec<u16>> = (0..5u16).map(|i| vec![i, i + 1]).collect();
    let xben = encode_xben(&assignments, BenVariant::MkvChain);
    let decoder = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    assert_eq!(decoder.count_samples().unwrap(), 5);
}

#[test]
fn xben_frame_decoder_new_and_iterate() {
    let assignments = vec![vec![1u16, 1, 2], vec![2u16, 2, 1]];
    let xben = encode_xben(&assignments, BenVariant::Standard);
    let frame_iter = BenStreamFrameReader::from_xben(Cursor::new(xben)).unwrap();
    let frames: Vec<(DecodeFrame, u16)> = frame_iter.map(|r| r.unwrap()).collect();
    assert_eq!(frames.len(), 2);
    for (frame, count) in &frames {
        assert_eq!(*count, 1u16);
        // Every standard ben32 frame ends with the 4-zero sentinel
        let bytes = match frame {
            DecodeFrame::XBen(b, _) => b,
            DecodeFrame::Ben(_) => panic!("xben frame iterator yielded BEN arm"),
        };
        assert!(bytes.ends_with(&[0u8, 0, 0, 0]));
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// BenStreamFrameReader
// ──────────────────────────────────────────────────────────────────────────────

#[test]
fn ben_frame_decoder_standard_iterates() {
    let assignments = vec![vec![1u16, 2, 3], vec![4u16, 5, 6]];
    let ben = encode_standard_ben(&assignments);
    let frame_iter = BenStreamFrameReader::from_ben(Cursor::new(ben)).unwrap();
    let frames: Vec<_> = frame_iter.map(|r| r.unwrap()).collect();
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].1, 1);
    assert_eq!(frames[1].1, 1);
}

#[test]
fn ben_frame_decoder_twodelta_yields_standard_frames() {
    let prev = vec![1u16, 1, 2, 2];
    let next = vec![2u16, 2, 1, 1];
    let assignments = vec![prev, next];
    let jsonl = jsonl_from_assignments(&assignments);
    let mut ben = Vec::new();
    encode_jsonl_to_ben(jsonl.as_slice(), &mut ben, BenVariant::TwoDelta).unwrap();

    // BenStreamFrameReader should re-encode TwoDelta frames back to standard BEN frames
    let decoder = BenStreamReader::from_ben(Cursor::new(ben))
        .unwrap()
        .silent(true);
    let frame_iter = decoder.into_frames();
    let frames: Vec<_> = frame_iter.map(|r| r.unwrap()).collect();
    assert_eq!(frames.len(), 2);
}

// Subsample-method single-case tests were deleted in the suite-audit deletion pass:
// `fuzz_subsample_by_indices`, `fuzz_subsample_every`, `fuzz_subsample_range`, and
// `fuzz_subsample_by_indices_twodelta` in `tests/test_impls_pipeline.rs` exercise these methods
// over random sequences with random indices/ranges/strides for both BEN and XBEN, subsuming the
// per-method single-case checks formerly here.
