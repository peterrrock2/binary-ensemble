use crate::codec::decode::jsonl_decode_ben32;
use crate::codec::decode::{decode_ben_to_jsonl, decode_xben_to_ben, decode_xben_to_jsonl};
use crate::codec::encode::{encode_ben_to_xben, xz_compress};
use crate::util::rle::rle_to_vec;
use crate::BenVariant;
use serde_json::{json, Value};
use std::io::{self, BufReader};

// The bit-packed payload for assignment [(1,4),(2,1),(3,3)] = [1,1,1,1,2,3,3,3].
// max_val_bit_count=2, max_len_bit_count=3, n_bytes=2:
//   bits 00-04: 01100 → val=01=1, len=100=4
//   bits 05-09: 10001 → val=10=2, len=001=1
//   bits 10-14: 11011 → val=11=3, len=011=3
//   bit  15:    0     → padding
const FRAME_HEADER: &[u8] = &[2, 3, 0, 0, 0, 2];
const FRAME_PAYLOAD: &[u8] = &[0b01100_100, 0b01_11011_0];

fn mkv_ben(count: u16) -> Vec<u8> {
    let mut ben = b"MKVCHAIN BEN FILE".to_vec();
    ben.extend_from_slice(FRAME_HEADER);
    ben.extend_from_slice(FRAME_PAYLOAD);
    ben.extend_from_slice(&count.to_be_bytes());
    ben
}

fn expected_line(assignment: &[u16], sample: usize) -> String {
    json!({"assignment": assignment, "sample": sample}).to_string() + "\n"
}

// ─── decode_ben_to_jsonl ───────────────────────────────────────────────

#[test]
fn decode_ben_to_jsonl_count_one() {
    let ben = mkv_ben(1);
    let mut out = Vec::new();
    decode_ben_to_jsonl(ben.as_slice(), &mut out).unwrap();

    let assign = rle_to_vec(vec![(1u16, 4), (2, 1), (3, 3)]);
    assert_eq!(out, expected_line(&assign, 1).as_bytes());
}

#[test]
fn decode_ben_to_jsonl_count_three_expands_to_three_lines() {
    let ben = mkv_ben(3);
    let mut out = Vec::new();
    decode_ben_to_jsonl(ben.as_slice(), &mut out).unwrap();

    let assign = rle_to_vec(vec![(1u16, 4), (2, 1), (3, 3)]);
    let expected: String = (1..=3).map(|i| expected_line(&assign, i)).collect();
    assert_eq!(out, expected.as_bytes());
}

#[test]
fn decode_ben_to_jsonl_sample_numbers_continue_across_frames() {
    // Frame 1: [1,1,1,1,2,3,3,3] count=2  →  samples 1,2
    // Frame 2: [23]              count=3  →  samples 3,4,5
    let mut ben = b"MKVCHAIN BEN FILE".to_vec();
    ben.extend_from_slice(FRAME_HEADER);
    ben.extend_from_slice(FRAME_PAYLOAD);
    ben.extend_from_slice(&2u16.to_be_bytes());
    // Frame for assignment [23]: max_val_bits=5, max_len_bits=1, n_bytes=1
    // payload 0b101111_00 = bits 10111_1 → val=10111=23, len=1=1
    ben.extend_from_slice(&[5, 1, 0, 0, 0, 1, 0b101111_00]);
    ben.extend_from_slice(&3u16.to_be_bytes());

    let mut out = Vec::new();
    decode_ben_to_jsonl(ben.as_slice(), &mut out).unwrap();

    let a1 = rle_to_vec(vec![(1u16, 4), (2, 1), (3, 3)]);
    let a2 = [23u16];
    let expected: String = (1..=2)
        .map(|i| expected_line(&a1, i))
        .chain((3..=5).map(|i| expected_line(&a2, i)))
        .collect();
    assert_eq!(out, expected.as_bytes());
}

#[test]
fn decode_ben_to_jsonl_16bit_value_with_count() {
    // Frame bytes from test_jsonl_decode_ben_16_bit_val (assignment [1,1,1,1,512,3,3,3])
    // with count=2 appended.
    let mut ben = b"MKVCHAIN BEN FILE".to_vec();
    ben.extend_from_slice(&[10, 3, 0, 0, 0, 5]);
    ben.extend_from_slice(&[
        0b00000000,
        0b01100_100,
        0b00000000,
        0b01_000000,
        0b0011011_0,
    ]);
    ben.extend_from_slice(&2u16.to_be_bytes());

    let mut out = Vec::new();
    decode_ben_to_jsonl(ben.as_slice(), &mut out).unwrap();

    let assign = rle_to_vec(vec![(1u16, 4), (512, 1), (3, 3)]);
    let expected: String = (1..=2).map(|i| expected_line(&assign, i)).collect();
    assert_eq!(out, expected.as_bytes());
}

#[test]
fn decode_ben_to_jsonl_empty_stream_produces_no_output() {
    let ben = b"MKVCHAIN BEN FILE".to_vec();
    let mut out = Vec::new();
    decode_ben_to_jsonl(ben.as_slice(), &mut out).unwrap();
    assert!(out.is_empty());
}

// ─── jsonl_decode_ben32 ────────────────────────────────────────────────

#[test]
fn jsonl_decode_ben32_mkvchain_count_one() {
    // ben32: [(1,4),(2,1),(3,3)] + terminator + count=1
    let input: Vec<u8> = vec![
        0, 1, 0, 4, // (1, 4)
        0, 2, 0, 1, // (2, 1)
        0, 3, 0, 3, // (3, 3)
        0, 0, 0, 0, // terminator
        0, 1, // count = 1
    ];
    let mut out = Vec::new();
    jsonl_decode_ben32(input.as_slice(), &mut out, 0, BenVariant::MkvChain).unwrap();

    let assign = rle_to_vec(vec![(1u16, 4), (2, 1), (3, 3)]);
    assert_eq!(out, expected_line(&assign, 1).as_bytes());
}

#[test]
fn jsonl_decode_ben32_mkvchain_count_five_expands_correctly() {
    // Single record with count=5 → 5 lines
    let mut input: Vec<u8> = vec![0, 23, 0, 1, 0, 0, 0, 0];
    input.extend_from_slice(&5u16.to_be_bytes());

    let mut out = Vec::new();
    jsonl_decode_ben32(input.as_slice(), &mut out, 0, BenVariant::MkvChain).unwrap();

    let expected: String = (1..=5).map(|i| expected_line(&[23], i)).collect();
    assert_eq!(out, expected.as_bytes());
}

#[test]
fn jsonl_decode_ben32_mkvchain_two_records_correct_sample_numbers() {
    // Record 1: [23]          count=2  → samples 1,2
    // Record 2: [1,2,3,4]     count=1  → sample 3
    let mut input: Vec<u8> = vec![0, 23, 0, 1, 0, 0, 0, 0];
    input.extend_from_slice(&2u16.to_be_bytes());
    input.extend_from_slice(&[0, 1, 0, 1, 0, 2, 0, 1, 0, 3, 0, 1, 0, 4, 0, 1, 0, 0, 0, 0]);
    input.extend_from_slice(&1u16.to_be_bytes());

    let mut out = Vec::new();
    jsonl_decode_ben32(input.as_slice(), &mut out, 0, BenVariant::MkvChain).unwrap();

    let expected =
        expected_line(&[23], 1) + &expected_line(&[23], 2) + &expected_line(&[1, 2, 3, 4], 3);
    assert_eq!(out, expected.as_bytes());
}

#[test]
fn jsonl_decode_ben32_mkvchain_starting_sample_offset() {
    // starting_sample=5 → first output line has sample=6
    let mut input: Vec<u8> = vec![0, 7, 0, 1, 0, 0, 0, 0];
    input.extend_from_slice(&2u16.to_be_bytes());

    let mut out = Vec::new();
    jsonl_decode_ben32(input.as_slice(), &mut out, 5, BenVariant::MkvChain).unwrap();

    let expected = expected_line(&[7], 6) + &expected_line(&[7], 7);
    assert_eq!(out, expected.as_bytes());
}

// ─── decode_xben_to_ben round-trip ────────────────────────────────────

#[test]
fn decode_xben_to_ben_mkvchain_roundtrip() {
    let ben = mkv_ben(1);
    let mut xben = Vec::new();
    encode_ben_to_xben(
        BufReader::new(ben.as_slice()),
        &mut xben,
        Some(1),
        Some(0),
        None,
    )
    .unwrap();

    let mut decoded = Vec::new();
    decode_xben_to_ben(BufReader::new(xben.as_slice()), &mut decoded).unwrap();

    // Verify by decoding the reconstructed BEN to JSONL
    let mut jsonl = Vec::new();
    decode_ben_to_jsonl(decoded.as_slice(), &mut jsonl).unwrap();

    let assign = rle_to_vec(vec![(1u16, 4), (2, 1), (3, 3)]);
    assert_eq!(jsonl, expected_line(&assign, 1).as_bytes());
}

#[test]
fn decode_xben_to_jsonl_mkvchain_count_expands() {
    // count=4 frame, verify XBEN → JSONL produces 4 lines
    let ben = mkv_ben(4);
    let mut xben = Vec::new();
    encode_ben_to_xben(
        BufReader::new(ben.as_slice()),
        &mut xben,
        Some(1),
        Some(0),
        None,
    )
    .unwrap();

    let mut jsonl = Vec::new();
    decode_xben_to_jsonl(BufReader::new(xben.as_slice()), &mut jsonl).unwrap();

    let assign = rle_to_vec(vec![(1u16, 4), (2, 1), (3, 3)]);
    let expected: String = (1..=4).map(|i| expected_line(&assign, i)).collect();
    assert_eq!(jsonl, expected.as_bytes());
}

// ─── error paths ──────────────────────────────────────────────────────

#[test]
fn decode_ben_to_jsonl_truncated_count_field_errors() {
    // Frame with only 1 byte of the 2-byte count field
    let mut ben = b"MKVCHAIN BEN FILE".to_vec();
    ben.extend_from_slice(FRAME_HEADER);
    ben.extend_from_slice(FRAME_PAYLOAD);
    ben.push(0x00); // only one byte of count instead of two

    let err = decode_ben_to_jsonl(ben.as_slice(), Vec::new()).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
}

#[test]
fn decode_xben_to_jsonl_rejects_mkvchain_partial_overflow() {
    // Compress just the banner + 3 garbage bytes → no valid frames
    let mut xz = Vec::new();
    let mut inner = b"MKVCHAIN BEN FILE".to_vec();
    inner.extend_from_slice(&[1, 2, 3]);
    xz_compress(BufReader::new(inner.as_slice()), &mut xz, Some(1), Some(0)).unwrap();

    let mut out = Vec::new();
    decode_xben_to_jsonl(BufReader::new(xz.as_slice()), &mut out).unwrap();
    assert!(out.is_empty());
}

// ─── decode_ben_to_jsonl — byte-level frame encoding counterparts ──────
// These mirror the Standard tests in standard.rs exactly, differing only in
// the MKVCHAIN banner and the trailing u16 BE count field appended to each frame.

#[test]
fn decode_ben_to_jsonl_exact() {
    // Same 5-byte payload as test_jsonl_decode_ben_exact, count=1.
    let mut ben = b"MKVCHAIN BEN FILE".to_vec();
    ben.extend_from_slice(&[2, 3, 0, 0, 0, 5]);
    ben.extend_from_slice(&[
        0b01100_100,
        0b01_11011_1,
        0b0010_1111,
        0b1_01001_10,
        0b001_11001_,
    ]);
    ben.extend_from_slice(&1u16.to_be_bytes());

    let mut out = Vec::new();
    decode_ben_to_jsonl(ben.as_slice(), &mut out).unwrap();

    let rle_assign = vec![
        (1u16, 4),
        (2, 1),
        (3, 3),
        (2, 2),
        (3, 7),
        (1, 1),
        (2, 1),
        (3, 1),
    ];
    let expected = json!({
        "assignment": rle_to_vec(rle_assign).iter().map(|x| json!(x)).collect::<Vec<Value>>(),
        "sample": 1
    })
    .to_string()
        + "\n";
    assert_eq!(out, expected.as_bytes());
}

#[test]
fn decode_ben_to_jsonl_16bit_len() {
    let mut ben = b"MKVCHAIN BEN FILE".to_vec();
    ben.extend_from_slice(&[2, 10, 0, 0, 0, 5]);
    ben.extend_from_slice(&[
        0b01000000,
        0b0100_1010,
        0b00000000_,
        0b11000000,
        0b0011_0000,
    ]);
    ben.extend_from_slice(&1u16.to_be_bytes());

    let mut out = Vec::new();
    decode_ben_to_jsonl(ben.as_slice(), &mut out).unwrap();

    let rle_assign = vec![(1u16, 4), (2, 512), (3, 3)];
    let expected = json!({
        "assignment": rle_to_vec(rle_assign).iter().map(|x| json!(x)).collect::<Vec<Value>>(),
        "sample": 1
    })
    .to_string()
        + "\n";
    assert_eq!(out, expected.as_bytes());
}

#[test]
fn decode_ben_to_jsonl_max_val_65535() {
    let mut ben = b"MKVCHAIN BEN FILE".to_vec();
    ben.extend_from_slice(&[16, 4, 0, 0, 0, 8]);
    ben.extend_from_slice(&[
        0b00000000,
        0b00010111,
        0b0100_1111,
        0b11111111,
        0b11111111_,
        0b00000000,
        0b00001000,
        0b0011_0000,
    ]);
    ben.extend_from_slice(&1u16.to_be_bytes());

    let mut out = Vec::new();
    decode_ben_to_jsonl(ben.as_slice(), &mut out).unwrap();

    let rle_assign = vec![(23u16, 4), (65535, 15), (8, 3)];
    let expected = json!({
        "assignment": rle_to_vec(rle_assign).iter().map(|x| json!(x)).collect::<Vec<Value>>(),
        "sample": 1
    })
    .to_string()
        + "\n";
    assert_eq!(out, expected.as_bytes());
}

#[test]
fn decode_ben_to_jsonl_max_len_65535() {
    let mut ben = b"MKVCHAIN BEN FILE".to_vec();
    ben.extend_from_slice(&[6, 16, 0, 0, 0, 9]);
    ben.extend_from_slice(&[
        0b01011100,
        0b00000000,
        0b000100_11,
        0b11001111,
        0b11111111,
        0b1111_0010,
        0b00000000,
        0b000000000,
        0b11_000000,
    ]);
    ben.extend_from_slice(&1u16.to_be_bytes());

    let mut out = Vec::new();
    decode_ben_to_jsonl(ben.as_slice(), &mut out).unwrap();

    let rle_assign = vec![(23u16, 4), (60, 65535), (8, 3)];
    let expected = json!({
        "assignment": rle_to_vec(rle_assign).iter().map(|x| json!(x)).collect::<Vec<Value>>(),
        "sample": 1
    })
    .to_string()
        + "\n";
    assert_eq!(out, expected.as_bytes());
}

#[test]
fn decode_ben_to_jsonl_max_val_and_len_65535() {
    let mut ben = b"MKVCHAIN BEN FILE".to_vec();
    ben.extend_from_slice(&[16, 16, 0, 0, 0, 12]);
    ben.extend_from_slice(&[
        0b00000000,
        0b00000001,
        0b00000000,
        0b00000011_,
        0b11111111,
        0b11111111,
        0b11111111,
        0b11111111_,
        0b00000000,
        0b00001000,
        0b00000000,
        0b00000100_,
    ]);
    ben.extend_from_slice(&1u16.to_be_bytes());

    let mut out = Vec::new();
    decode_ben_to_jsonl(ben.as_slice(), &mut out).unwrap();

    let rle_assign = vec![(1u16, 3), (65535, 65535), (8, 4)];
    let expected = json!({
        "assignment": rle_to_vec(rle_assign).iter().map(|x| json!(x)).collect::<Vec<Value>>(),
        "sample": 1
    })
    .to_string()
        + "\n";
    assert_eq!(out, expected.as_bytes());
}

#[test]
fn decode_ben_to_jsonl_single_element() {
    // Assignment [23], count=1.
    let mut ben = b"MKVCHAIN BEN FILE".to_vec();
    ben.extend_from_slice(&[5, 1, 0, 0, 0, 1, 0b101111_00]);
    ben.extend_from_slice(&1u16.to_be_bytes());

    let mut out = Vec::new();
    decode_ben_to_jsonl(ben.as_slice(), &mut out).unwrap();

    assert_eq!(out, expected_line(&[23u16], 1).as_bytes());
}

#[test]
fn decode_ben_to_jsonl_single_one() {
    // Assignment [1], count=1.
    let mut ben = b"MKVCHAIN BEN FILE".to_vec();
    ben.extend_from_slice(&[1, 1, 0, 0, 0, 1, 0b11_000000]);
    ben.extend_from_slice(&1u16.to_be_bytes());

    let mut out = Vec::new();
    decode_ben_to_jsonl(ben.as_slice(), &mut out).unwrap();

    assert_eq!(out, expected_line(&[1u16], 1).as_bytes());
}

#[test]
fn decode_ben_to_jsonl_three_frames() {
    // Three distinct frames, each count=1 — mirrors test_decode_ben_multiple_simple_lines.
    let mut ben = b"MKVCHAIN BEN FILE".to_vec();
    // Frame 1: rle [(1,4),(2,4),(3,4),(4,4)]
    ben.extend_from_slice(&[3, 3, 0, 0, 0, 3, 0b001100_01, 0b0100_0111, 0b00_100100]);
    ben.extend_from_slice(&1u16.to_be_bytes());
    // Frame 2: rle [(2,2),(3,7),(1,1),(2,1),(3,1)]
    ben.extend_from_slice(&[
        2,
        3,
        0,
        0,
        0,
        4,
        0b10010_111,
        0b11_01001_1,
        0b0001_1100,
        0b1_0000000,
    ]);
    ben.extend_from_slice(&1u16.to_be_bytes());
    // Frame 3: rle [(1..10, each 1)]
    ben.extend_from_slice(&[
        4,
        1,
        0,
        0,
        0,
        7,
        0b00011_001,
        0b01_00111_0,
        0b1001_0101,
        0b1_01101_01,
        0b111_10001,
        0b10011_101,
        0b01_000000,
    ]);
    ben.extend_from_slice(&1u16.to_be_bytes());

    let mut out = Vec::new();
    decode_ben_to_jsonl(ben.as_slice(), &mut out).unwrap();

    let rle_lst: Vec<Vec<(u16, u16)>> = vec![
        vec![(1, 4), (2, 4), (3, 4), (4, 4)],
        vec![(2, 2), (3, 7), (1, 1), (2, 1), (3, 1)],
        vec![
            (1, 1),
            (2, 1),
            (3, 1),
            (4, 1),
            (5, 1),
            (6, 1),
            (7, 1),
            (8, 1),
            (9, 1),
            (10, 1),
        ],
    ];
    let expected: String = rle_lst
        .into_iter()
        .enumerate()
        .map(|(i, rle)| {
            json!({
                "assignment": rle_to_vec(rle).iter().map(|x| json!(x)).collect::<Vec<Value>>(),
                "sample": i + 1
            })
            .to_string()
                + "\n"
        })
        .collect();
    assert_eq!(out, expected.as_bytes());
}

// ─── jsonl_decode_ben32 — byte-level counterparts ─────────────────────
// Each Standard ben32 record has [pairs...][0,0,0,0] terminator.
// Each MkvChain ben32 record appends a u16 BE count after the terminator.

#[test]
fn jsonl_decode_ben32_16bit_val() {
    let mut input = vec![0, 1, 0, 4, 2, 0, 0, 1, 0, 3, 0, 3, 0, 0, 0, 0];
    input.extend_from_slice(&1u16.to_be_bytes());

    let mut out = Vec::new();
    jsonl_decode_ben32(input.as_slice(), &mut out, 0, BenVariant::MkvChain).unwrap();

    let rle_assign = vec![(1u16, 4), (512, 1), (3, 3)];
    let expected = json!({
        "assignment": rle_to_vec(rle_assign).iter().map(|x| json!(x)).collect::<Vec<Value>>(),
        "sample": 1
    })
    .to_string()
        + "\n";
    assert_eq!(out, expected.as_bytes());
}

#[test]
fn jsonl_decode_ben32_16bit_len() {
    let mut input = vec![0, 1, 0, 4, 0, 2, 2, 0, 0, 3, 0, 3, 0, 0, 0, 0];
    input.extend_from_slice(&1u16.to_be_bytes());

    let mut out = Vec::new();
    jsonl_decode_ben32(input.as_slice(), &mut out, 0, BenVariant::MkvChain).unwrap();

    let rle_assign = vec![(1u16, 4), (2, 512), (3, 3)];
    let expected = json!({
        "assignment": rle_to_vec(rle_assign).iter().map(|x| json!(x)).collect::<Vec<Value>>(),
        "sample": 1
    })
    .to_string()
        + "\n";
    assert_eq!(out, expected.as_bytes());
}

#[test]
fn jsonl_decode_ben32_max_val_65535() {
    let mut input = vec![0, 23, 0, 4, 255, 255, 0, 15, 0, 8, 0, 3, 0, 0, 0, 0];
    input.extend_from_slice(&1u16.to_be_bytes());

    let mut out = Vec::new();
    jsonl_decode_ben32(input.as_slice(), &mut out, 0, BenVariant::MkvChain).unwrap();

    let rle_assign = vec![(23u16, 4), (65535, 15), (8, 3)];
    let expected = json!({
        "assignment": rle_to_vec(rle_assign).iter().map(|x| json!(x)).collect::<Vec<Value>>(),
        "sample": 1
    })
    .to_string()
        + "\n";
    assert_eq!(out, expected.as_bytes());
}

#[test]
fn jsonl_decode_ben32_max_len_65535() {
    let mut input = vec![0, 23, 0, 4, 0, 60, 255, 255, 0, 8, 0, 3, 0, 0, 0, 0];
    input.extend_from_slice(&1u16.to_be_bytes());

    let mut out = Vec::new();
    jsonl_decode_ben32(input.as_slice(), &mut out, 0, BenVariant::MkvChain).unwrap();

    let rle_assign = vec![(23u16, 4), (60, 65535), (8, 3)];
    let expected = json!({
        "assignment": rle_to_vec(rle_assign).iter().map(|x| json!(x)).collect::<Vec<Value>>(),
        "sample": 1
    })
    .to_string()
        + "\n";
    assert_eq!(out, expected.as_bytes());
}

#[test]
fn jsonl_decode_ben32_single_element() {
    let mut input = vec![0, 23, 0, 1, 0, 0, 0, 0];
    input.extend_from_slice(&1u16.to_be_bytes());

    let mut out = Vec::new();
    jsonl_decode_ben32(input.as_slice(), &mut out, 0, BenVariant::MkvChain).unwrap();

    let expected = json!({"assignment": [23u16], "sample": 1}).to_string() + "\n";
    assert_eq!(out, expected.as_bytes());
}

#[test]
fn jsonl_decode_ben32_three_frames() {
    // Three ben32 records with count=1 each — mirrors test_decode_ben32_multiple_simple_lines.
    let mut input: Vec<u8> = Vec::new();
    // Record 1: rle [(1,4),(2,4),(3,4),(4,4)]
    input.extend_from_slice(&[0, 1, 0, 4, 0, 2, 0, 4, 0, 3, 0, 4, 0, 4, 0, 4, 0, 0, 0, 0]);
    input.extend_from_slice(&1u16.to_be_bytes());
    // Record 2: rle [(2,2),(3,7),(1,1),(2,1),(3,1)]
    input.extend_from_slice(&[
        0, 2, 0, 2, 0, 3, 0, 7, 0, 1, 0, 1, 0, 2, 0, 1, 0, 3, 0, 1, 0, 0, 0, 0,
    ]);
    input.extend_from_slice(&1u16.to_be_bytes());
    // Record 3: rle [(1..10, each 1)]
    input.extend_from_slice(&[
        0, 1, 0, 1, 0, 2, 0, 1, 0, 3, 0, 1, 0, 4, 0, 1, 0, 5, 0, 1, 0, 6, 0, 1, 0, 7, 0, 1, 0, 8,
        0, 1, 0, 9, 0, 1, 0, 10, 0, 1, 0, 0, 0, 0,
    ]);
    input.extend_from_slice(&1u16.to_be_bytes());

    let mut out = Vec::new();
    jsonl_decode_ben32(input.as_slice(), &mut out, 0, BenVariant::MkvChain).unwrap();

    let rle_lst: Vec<Vec<(u16, u16)>> = vec![
        vec![(1, 4), (2, 4), (3, 4), (4, 4)],
        vec![(2, 2), (3, 7), (1, 1), (2, 1), (3, 1)],
        vec![
            (1, 1),
            (2, 1),
            (3, 1),
            (4, 1),
            (5, 1),
            (6, 1),
            (7, 1),
            (8, 1),
            (9, 1),
            (10, 1),
        ],
    ];
    let expected: String = rle_lst
        .into_iter()
        .enumerate()
        .map(|(i, rle)| {
            json!({
                "assignment": rle_to_vec(rle).iter().map(|x| json!(x)).collect::<Vec<Value>>(),
                "sample": i + 1
            })
            .to_string()
                + "\n"
        })
        .collect();
    assert_eq!(out, expected.as_bytes());
}
