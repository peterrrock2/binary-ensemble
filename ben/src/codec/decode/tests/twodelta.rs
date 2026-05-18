use crate::codec::decode::{
    apply_twodelta_runs_to_assignment, decode_ben_to_jsonl, decode_twodelta_frame,
    decode_xben_to_jsonl,
};
use crate::codec::encode::{encode_ben_to_xben, encode_twodelta_frame};
use crate::codec::frames::BenEncodeFrame;
use crate::io::writer::BenStreamWriter;
use crate::util::rle::rle_to_vec;
use crate::BenVariant;
use serde_json::{json, Value};
use std::io::BufReader;

// Build a TwoDelta BEN stream for the given sequence of assignments.
fn make_twodelta_ben(assignments: &[Vec<u16>]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut w = BenStreamWriter::for_ben(&mut out, BenVariant::TwoDelta).unwrap();
        for a in assignments {
            w.write_assignment(a.clone()).unwrap();
        }
    }
    out
}

fn expected_line(assignment: &[u16], sample: usize) -> String {
    json!({"assignment": assignment, "sample": sample}).to_string() + "\n"
}

// ─── apply_twodelta_runs_to_assignment ─────────────────────────────────

#[test]
fn apply_runs_basic_two_position_swap() {
    // prev: [1,2,1,2], run_lengths=[2,2] starting with value 1 → first 2 pair positions get value
    // 1, next 2 get value 2 pair positions (where val is 1 or 2): 0,1,2,3 run 1 (len=2, val=1): pos
    // 0,1 → 1,1; run 2 (len=2, val=2): pos 2,3 → 2,2
    let prev = vec![1u16, 2, 1, 2];
    let result = apply_twodelta_runs_to_assignment(prev, (1, 2), &[2, 2]).unwrap();
    assert_eq!(result, vec![1, 1, 2, 2]);
}

#[test]
fn apply_runs_non_pair_positions_unchanged() {
    // prev: [1,2,3,1,2], pair=(1,2), run_lengths=[2,2] pair positions: 0,1,3,4 (index 2 holds value
    // 3 → unchanged) run 1 (len=2, val=1): pos 0,1 → 1,1 run 2 (len=2, val=2): pos 3,4 → 2,2
    let prev = vec![1u16, 2, 3, 1, 2];
    let result = apply_twodelta_runs_to_assignment(prev, (1, 2), &[2, 2]).unwrap();
    assert_eq!(result, vec![1, 1, 3, 2, 2]);
}

#[test]
fn apply_runs_full_reversal() {
    // prev: [1,1,2,2], pair=(2,1), run_lengths=[2,2] pair positions: 0,1,2,3; pair.0=2 comes first
    // run 1 (len=2, val=2): pos 0,1 → 2,2; run 2 (len=2, val=1): pos 2,3 → 1,1
    let prev = vec![1u16, 1, 2, 2];
    let result = apply_twodelta_runs_to_assignment(prev, (2, 1), &[2, 2]).unwrap();
    assert_eq!(result, vec![2, 2, 1, 1]);
}

#[test]
fn apply_runs_exhausted_before_all_positions_covered_errors() {
    // prev: [1,2,1], pair=(1,2), run_lengths=[1] — too short After consuming run 0 (1 position with
    // value 1), run 1 missing → error
    let prev = vec![1u16, 2, 1];
    let err = apply_twodelta_runs_to_assignment(prev, (1, 2), &[1]).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn apply_runs_alternating_single_positions() {
    // prev: [1,2,1,2,1], pair=(1,2), run_lengths=[1,1,1,1,1] Each pair position flips: run
    // alternates 1,2,1,2,1
    let prev = vec![1u16, 2, 1, 2, 1];
    let result = apply_twodelta_runs_to_assignment(prev, (1, 2), &[1, 1, 1, 1, 1]).unwrap();
    // run[0]=1 → pos0=1; run[1]=1 → pos1=2; run[2]=1 → pos2=1; etc.
    assert_eq!(result, vec![1, 2, 1, 2, 1]);
}

// ─── decode_twodelta_frame ─────────────────────────────────────────────

#[test]
fn decode_twodelta_frame_basic() {
    let frame = BenEncodeFrame::from_run_lengths((1, 2), vec![2, 2], None);
    let prev = vec![1u16, 2, 1, 2];
    let result = decode_twodelta_frame(prev, &frame).unwrap();
    assert_eq!(result, vec![1, 1, 2, 2]);
}

#[test]
fn decode_twodelta_frame_full_swap() {
    // pair=(2,1) means run starts with value 2; run_lengths=[2,2] prev [1,2,1,2]: pair positions
    // 0,1,2,3 → [2,2,1,1]
    let frame = BenEncodeFrame::from_run_lengths((2, 1), vec![2, 2], None);
    let prev = vec![1u16, 2, 1, 2];
    let result = decode_twodelta_frame(prev, &frame).unwrap();
    assert_eq!(result, vec![2, 2, 1, 1]);
}

#[test]
fn decode_twodelta_frame_chain_returns_to_original() {
    // Frame 1: (1,2) run=[2,2] applied to [1,2,1,2] → [1,1,2,2] Frame 2: (1,2) run=[1,1,1,1]
    // applied to [1,1,2,2] → [1,2,1,2]
    let f1 = BenEncodeFrame::from_run_lengths((1, 2), vec![2, 2], None);
    let f2 = BenEncodeFrame::from_run_lengths((1, 2), vec![1, 1, 1, 1], None);
    let initial = vec![1u16, 2, 1, 2];
    let after_f1 = decode_twodelta_frame(initial.clone(), &f1).unwrap();
    assert_eq!(after_f1, vec![1, 1, 2, 2]);
    let after_f2 = decode_twodelta_frame(after_f1, &f2).unwrap();
    assert_eq!(after_f2, initial);
}

#[test]
fn decode_twodelta_frame_roundtrip_with_encode() {
    // Verify that encode_twodelta_frame + decode_twodelta_frame is identity.
    let prev = vec![1u16, 1, 2, 2, 1, 2, 1, 2];
    let next = vec![2u16, 2, 1, 1, 1, 2, 1, 2];
    let frame = encode_twodelta_frame(&prev, &next, None).unwrap();
    let decoded = decode_twodelta_frame(prev, &frame).unwrap();
    assert_eq!(decoded, next);
}

#[test]
fn decode_twodelta_frame_larger_assignment() {
    let prev: Vec<u16> = (0..100).map(|i| if i < 50 { 1 } else { 2 }).collect();
    let next: Vec<u16> = (0..100).map(|i| if i < 50 { 2 } else { 1 }).collect();
    let frame = encode_twodelta_frame(&prev, &next, None).unwrap();
    let result = decode_twodelta_frame(prev, &frame).unwrap();
    assert_eq!(result, next);
}

// ─── decode_ben_to_jsonl with TwoDelta BEN streams ─────────────────────

#[test]
fn decode_ben_to_jsonl_twodelta_anchor_only() {
    let assignments = vec![vec![1u16, 2, 1, 2]];
    let ben = make_twodelta_ben(&assignments);
    let mut out = Vec::new();
    decode_ben_to_jsonl(ben.as_slice(), &mut out).unwrap();

    assert_eq!(out, expected_line(&[1, 2, 1, 2], 1).as_bytes());
}

#[test]
fn decode_ben_to_jsonl_twodelta_anchor_plus_one_delta() {
    let assignments = vec![
        vec![1u16, 2, 1, 2], // anchor
        vec![1u16, 1, 2, 2], // delta: swap positions 1 and 2
    ];
    let ben = make_twodelta_ben(&assignments);
    let mut out = Vec::new();
    decode_ben_to_jsonl(ben.as_slice(), &mut out).unwrap();

    let expected = expected_line(&[1, 2, 1, 2], 1) + &expected_line(&[1, 1, 2, 2], 2);
    assert_eq!(out, expected.as_bytes());
}

#[test]
fn decode_ben_to_jsonl_twodelta_chain_of_deltas() {
    let a0 = vec![1u16, 2, 1, 2];
    let a1 = vec![1u16, 1, 2, 2];
    let a2 = vec![2u16, 1, 2, 1];
    let a3 = vec![2u16, 2, 1, 1];
    let assignments = vec![a0.clone(), a1.clone(), a2.clone(), a3.clone()];
    let ben = make_twodelta_ben(&assignments);
    let mut out = Vec::new();
    decode_ben_to_jsonl(ben.as_slice(), &mut out).unwrap();

    let expected = expected_line(&a0, 1)
        + &expected_line(&a1, 2)
        + &expected_line(&a2, 3)
        + &expected_line(&a3, 4);
    assert_eq!(out, expected.as_bytes());
}

#[test]
fn decode_ben_to_jsonl_twodelta_repeated_anchor_expands() {
    // Writing the same assignment 3 times then a delta; anchor frame should have count=3.
    let anchor = vec![1u16, 2, 1, 2];
    let delta = vec![1u16, 1, 2, 2];
    let assignments = vec![
        anchor.clone(),
        anchor.clone(),
        anchor.clone(),
        delta.clone(),
    ];
    let ben = make_twodelta_ben(&assignments);
    let mut out = Vec::new();
    decode_ben_to_jsonl(ben.as_slice(), &mut out).unwrap();

    let expected = expected_line(&anchor, 1)
        + &expected_line(&anchor, 2)
        + &expected_line(&anchor, 3)
        + &expected_line(&delta, 4);
    assert_eq!(out, expected.as_bytes());
}

#[test]
fn decode_ben_to_jsonl_twodelta_multiple_repeated_deltas() {
    // Anchor repeated twice, then a delta repeated twice
    let anchor = vec![1u16, 2, 1, 2];
    let delta = vec![2u16, 1, 2, 1];
    let assignments = vec![anchor.clone(), anchor.clone(), delta.clone(), delta.clone()];
    let ben = make_twodelta_ben(&assignments);
    let mut out = Vec::new();
    decode_ben_to_jsonl(ben.as_slice(), &mut out).unwrap();

    let expected = expected_line(&anchor, 1)
        + &expected_line(&anchor, 2)
        + &expected_line(&delta, 3)
        + &expected_line(&delta, 4);
    assert_eq!(out, expected.as_bytes());
}

// ─── decode_ben_to_jsonl — byte-level anchor frame counterparts ──────── The TwoDelta first frame
// (anchor) is encoded in MkvChain format. These tests mirror every byte-level Standard / MkvChain
// decode_ben_to_jsonl test using the TWODELTA banner and the same bit-packed frame bytes, verifying
// that the anchor path decodes the same payload correctly regardless of variant.

#[test]
fn decode_ben_to_jsonl_underflow_anchor() {
    // Mirrors test_jsonl_decode_ben_underflow: 2-byte payload, 1 padding bit.
    let mut ben = b"TWODELTA BEN FILE".to_vec();
    ben.extend_from_slice(&[2, 3, 0, 0, 0, 2, 0b01100_100, 0b01_11011_0]);
    ben.extend_from_slice(&1u16.to_be_bytes());

    let mut out = Vec::new();
    decode_ben_to_jsonl(ben.as_slice(), &mut out).unwrap();

    let rle_assign = vec![(1u16, 4), (2, 1), (3, 3)];
    let expected = json!({
        "assignment": rle_to_vec(rle_assign).iter().map(|x| json!(x)).collect::<Vec<Value>>(),
        "sample": 1
    })
    .to_string()
        + "\n";
    assert_eq!(out, expected.as_bytes());
}

#[test]
fn decode_ben_to_jsonl_exact_anchor() {
    // Mirrors test_jsonl_decode_ben_exact: 5-byte payload, zero padding.
    let mut ben = b"TWODELTA BEN FILE".to_vec();
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
fn decode_ben_to_jsonl_16bit_val_anchor() {
    let mut ben = b"TWODELTA BEN FILE".to_vec();
    ben.extend_from_slice(&[10, 3, 0, 0, 0, 5]);
    ben.extend_from_slice(&[
        0b00000000,
        0b01100_100,
        0b00000000,
        0b01_000000,
        0b0011011_0,
    ]);
    ben.extend_from_slice(&1u16.to_be_bytes());

    let mut out = Vec::new();
    decode_ben_to_jsonl(ben.as_slice(), &mut out).unwrap();

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
fn decode_ben_to_jsonl_16bit_len_anchor() {
    let mut ben = b"TWODELTA BEN FILE".to_vec();
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
fn decode_ben_to_jsonl_max_val_65535_anchor() {
    let mut ben = b"TWODELTA BEN FILE".to_vec();
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
fn decode_ben_to_jsonl_max_len_65535_anchor() {
    let mut ben = b"TWODELTA BEN FILE".to_vec();
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
fn decode_ben_to_jsonl_max_val_and_len_65535_anchor() {
    let mut ben = b"TWODELTA BEN FILE".to_vec();
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
fn decode_ben_to_jsonl_single_element_anchor() {
    // Anchor assignment [23], count=1.
    let mut ben = b"TWODELTA BEN FILE".to_vec();
    ben.extend_from_slice(&[5, 1, 0, 0, 0, 1, 0b101111_00]);
    ben.extend_from_slice(&1u16.to_be_bytes());

    let mut out = Vec::new();
    decode_ben_to_jsonl(ben.as_slice(), &mut out).unwrap();

    assert_eq!(out, expected_line(&[23u16], 1).as_bytes());
}

#[test]
fn decode_ben_to_jsonl_single_one_anchor() {
    // Anchor assignment [1], count=1.
    let mut ben = b"TWODELTA BEN FILE".to_vec();
    ben.extend_from_slice(&[1, 1, 0, 0, 0, 1, 0b11_000000]);
    ben.extend_from_slice(&1u16.to_be_bytes());

    let mut out = Vec::new();
    decode_ben_to_jsonl(ben.as_slice(), &mut out).unwrap();

    assert_eq!(out, expected_line(&[1u16], 1).as_bytes());
}

#[test]
fn decode_ben_to_jsonl_three_frames_byte_level() {
    // Hand-crafted TwoDelta BEN stream with explicit wire bytes:
    //   anchor [1,2] (count=1) in MkvChain format
    //   delta  [1,2]→[2,1] (count=1) in TwoDelta format
    //   delta  [2,1]→[1,2] (count=1) in TwoDelta format
    //
    // Anchor [1,2]:
    //   max_val_bits=2, max_len_bits=1, n_bytes=1
    //   RLE (1,1),(2,1) → 3 bits each: 011_101_XX = 0b01110100 = 0x74
    //   raw_bytes = [2, 1, 0,0,0,1, 0x74, 0,1]
    //
    // Delta [1,2]→[2,1]:
    //   pair=(2,1), run_lengths=[1,1], max_len_bits=1, n_bytes=1
    //   payload: 2 × 1-bit values packed → 0b11000000 = 0xC0
    //   raw_bytes = [0,2, 0,1, 1, 0,0,0,1, 0xC0, 0,1]
    //
    // Delta [2,1]→[1,2]:
    //   pair=(1,2), run_lengths=[1,1], same encoding
    //   raw_bytes = [0,1, 0,2, 1, 0,0,0,1, 0xC0, 0,1]
    let mut ben = b"TWODELTA BEN FILE".to_vec();
    ben.extend_from_slice(&[2, 1, 0, 0, 0, 1, 0x74, 0, 1]);
    ben.extend_from_slice(&[0, 2, 0, 1, 1, 0, 0, 0, 1, 0xC0, 0, 1]);
    ben.extend_from_slice(&[0, 1, 0, 2, 1, 0, 0, 0, 1, 0xC0, 0, 1]);

    let mut out = Vec::new();
    decode_ben_to_jsonl(ben.as_slice(), &mut out).unwrap();

    let expected = expected_line(&[1u16, 2], 1)
        + &expected_line(&[2u16, 1], 2)
        + &expected_line(&[1u16, 2], 3);
    assert_eq!(out, expected.as_bytes());
}

// ─── decode_xben_to_jsonl round-trip ──────────────────────────────────

#[test]
fn decode_xben_to_jsonl_twodelta_anchor_only() {
    let anchor = vec![1u16, 2, 1, 2];
    let ben = make_twodelta_ben(&[anchor.clone()]);
    let mut xben = Vec::new();
    encode_ben_to_xben(
        BufReader::new(ben.as_slice()),
        &mut xben,
        Some(1),
        Some(0),
        None,
        None,
    )
    .unwrap();

    let mut jsonl = Vec::new();
    decode_xben_to_jsonl(BufReader::new(xben.as_slice()), &mut jsonl).unwrap();

    assert_eq!(jsonl, expected_line(&anchor, 1).as_bytes());
}

#[test]
fn decode_xben_to_jsonl_twodelta_chain_roundtrip() {
    let a0 = vec![1u16, 2, 1, 2];
    let a1 = vec![1u16, 1, 2, 2];
    let a2 = vec![2u16, 1, 2, 1];
    let assignments = vec![a0.clone(), a1.clone(), a2.clone()];

    let ben = make_twodelta_ben(&assignments);
    let mut xben = Vec::new();
    encode_ben_to_xben(
        BufReader::new(ben.as_slice()),
        &mut xben,
        Some(1),
        Some(0),
        None,
        None,
    )
    .unwrap();

    let mut jsonl = Vec::new();
    decode_xben_to_jsonl(BufReader::new(xben.as_slice()), &mut jsonl).unwrap();

    let expected = expected_line(&a0, 1) + &expected_line(&a1, 2) + &expected_line(&a2, 3);
    assert_eq!(jsonl, expected.as_bytes());
}

#[test]
fn decode_xben_to_jsonl_twodelta_with_repetitions() {
    // Repeated assignments in XBEN → correct expansion
    let anchor = vec![1u16, 2, 1, 2];
    let assignments = vec![anchor.clone(), anchor.clone(), anchor.clone()];
    let ben = make_twodelta_ben(&assignments);
    let mut xben = Vec::new();
    encode_ben_to_xben(
        BufReader::new(ben.as_slice()),
        &mut xben,
        Some(1),
        Some(0),
        None,
        None,
    )
    .unwrap();

    let mut jsonl = Vec::new();
    decode_xben_to_jsonl(BufReader::new(xben.as_slice()), &mut jsonl).unwrap();

    let expected: String = (1..=3).map(|i| expected_line(&anchor, i)).collect();
    assert_eq!(jsonl, expected.as_bytes());
}
