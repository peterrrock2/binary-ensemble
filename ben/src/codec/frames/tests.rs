use super::*;
use crate::BenVariant;
use std::io::{self, Read};

// ── Helpers ─────────────────────────────────────────────────────────────────

/// A reader that returns one successful byte then an I/O error.
struct ErrorAfterOneByte;

impl Read for ErrorAfterOneByte {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        buf[0] = 0x01;
        Err(io::Error::new(io::ErrorKind::BrokenPipe, "broken"))
    }
}

fn unwrap_standard(frame: BenDecodeFrame) -> (u8, u8, u32, Vec<u8>) {
    match frame {
        BenDecodeFrame::Standard {
            max_val_bit_count,
            max_len_bit_count,
            n_bytes,
            raw_bytes,
        } => (max_val_bit_count, max_len_bit_count, n_bytes, raw_bytes),
        other => panic!("expected Standard, got {:?}", other),
    }
}

fn unwrap_mkv(frame: BenDecodeFrame) -> (u8, u8, u32, Vec<u8>, u16) {
    match frame {
        BenDecodeFrame::MkvChain {
            max_val_bit_count,
            max_len_bit_count,
            n_bytes,
            raw_bytes,
            count,
        } => (
            max_val_bit_count,
            max_len_bit_count,
            n_bytes,
            raw_bytes,
            count,
        ),
        other => panic!("expected MkvChain, got {:?}", other),
    }
}

fn unwrap_twodelta(frame: BenDecodeFrame) -> ((u16, u16), Vec<u16>, u16) {
    match frame {
        BenDecodeFrame::TwoDelta {
            pair,
            run_lengths,
            count,
        } => (pair, run_lengths, count),
        other => panic!("expected TwoDelta, got {:?}", other),
    }
}

fn unwrap_encode_standard(frame: BenEncodeFrame) -> (Vec<(u16, u16)>, u8, u8, u32, Vec<u8>) {
    match frame {
        BenEncodeFrame::Standard {
            runs,
            max_val_bit_count,
            max_len_bit_count,
            n_bytes,
            raw_bytes,
        } => (
            runs,
            max_val_bit_count,
            max_len_bit_count,
            n_bytes,
            raw_bytes,
        ),
        other => panic!("expected Standard encode arm, got {:?}", other),
    }
}

/// The destructured fields of a [`BenEncodeFrame::MkvChain`] arm: `(runs, max_val_bit_count,
/// max_len_bit_count, n_bytes, raw_bytes, count)`.
type MkvChainEncodeFields = (Vec<(u16, u16)>, u8, u8, u32, Vec<u8>, u16);

fn unwrap_encode_mkv(frame: BenEncodeFrame) -> MkvChainEncodeFields {
    match frame {
        BenEncodeFrame::MkvChain {
            runs,
            max_val_bit_count,
            max_len_bit_count,
            n_bytes,
            raw_bytes,
            count,
        } => (
            runs,
            max_val_bit_count,
            max_len_bit_count,
            n_bytes,
            raw_bytes,
            count,
        ),
        other => panic!("expected MkvChain encode arm, got {:?}", other),
    }
}

fn unwrap_encode_twodelta(frame: BenEncodeFrame) -> ((u16, u16), u8, u32, Vec<u16>, Vec<u8>, u16) {
    match frame {
        BenEncodeFrame::TwoDelta {
            pair,
            max_len_bit_count,
            n_bytes,
            run_length_vector,
            raw_bytes,
            count,
        } => (
            pair,
            max_len_bit_count,
            n_bytes,
            run_length_vector,
            raw_bytes,
            count,
        ),
        other => panic!("expected TwoDelta encode arm, got {:?}", other),
    }
}

// ── BenDecodeFrame::from_reader (Standard) ──────────────────────────────────

#[test]
fn ben_decode_standard_from_reader() {
    // Header: max_val_bits=2, max_len_bits=3, n_bytes=2; payload 2 bytes.
    let data: Vec<u8> = vec![2, 3, 0, 0, 0, 2, 0xAB, 0xCD];
    let mut cursor = io::Cursor::new(data);
    let frame = BenDecodeFrame::from_reader(&mut cursor, BenVariant::Standard)
        .unwrap()
        .unwrap();
    let (mvb, mlb, n, payload) = unwrap_standard(frame);
    assert_eq!(mvb, 2);
    assert_eq!(mlb, 3);
    assert_eq!(n, 2);
    assert_eq!(payload, vec![0xAB, 0xCD]);
}

#[test]
fn ben_decode_standard_eof_returns_none() {
    let mut cursor = io::Cursor::new(Vec::<u8>::new());
    let result = BenDecodeFrame::from_reader(&mut cursor, BenVariant::Standard).unwrap();
    assert!(result.is_none());
}

#[test]
fn ben_decode_standard_truncated_header_errors() {
    let mut cursor = io::Cursor::new(vec![2u8]); // only one header byte
    let err = BenDecodeFrame::from_reader(&mut cursor, BenVariant::Standard).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
}

#[test]
fn ben_decode_standard_non_eof_read_error_propagates() {
    let err =
        BenDecodeFrame::from_reader(&mut ErrorAfterOneByte, BenVariant::Standard).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
}

#[test]
fn ben_decode_oversized_n_bytes_rejected_before_allocating() {
    // Headers declaring an absurd payload length must be rejected before the payload buffer is
    // allocated — no payload bytes are supplied here, so reaching the allocation would surface as
    // an UnexpectedEof (or worse, an OOM under fuzzing) instead of the cap's InvalidData.
    let oversized = u32::MAX.to_be_bytes();

    // Standard: [mvb, mlb, n_bytes].
    let mut data = vec![2u8, 3];
    data.extend_from_slice(&oversized);
    let err = BenDecodeFrame::from_reader(&mut io::Cursor::new(data), BenVariant::Standard)
        .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert!(err.to_string().contains("refusing to allocate"));

    // MkvChain: same header shape.
    let mut data = vec![2u8, 3];
    data.extend_from_slice(&oversized);
    let err = BenDecodeFrame::from_reader(&mut io::Cursor::new(data), BenVariant::MkvChain)
        .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert!(err.to_string().contains("refusing to allocate"));

    // TwoDelta: [pair_a, pair_b, max_len_bits, n_bytes].
    let mut data = vec![0u8, 1, 0, 2, 4];
    data.extend_from_slice(&oversized);
    let err = BenDecodeFrame::from_reader(&mut io::Cursor::new(data), BenVariant::TwoDelta)
        .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert!(err.to_string().contains("refusing to allocate"));
}

// ── BenDecodeFrame::from_reader (MkvChain) ──────────────────────────────────

#[test]
fn ben_decode_mkv_from_reader() {
    // Header (6) + payload (2) + count (2) = 10 bytes.
    let data: Vec<u8> = vec![2, 3, 0, 0, 0, 2, 0xAB, 0xCD, 0x00, 0x07];
    let mut cursor = io::Cursor::new(data);
    let frame = BenDecodeFrame::from_reader(&mut cursor, BenVariant::MkvChain)
        .unwrap()
        .unwrap();
    let (mvb, mlb, n, payload, count) = unwrap_mkv(frame);
    assert_eq!(mvb, 2);
    assert_eq!(mlb, 3);
    assert_eq!(n, 2);
    assert_eq!(payload, vec![0xAB, 0xCD]);
    assert_eq!(count, 7);
}

#[test]
fn ben_decode_mkv_eof_returns_none() {
    let mut cursor = io::Cursor::new(Vec::<u8>::new());
    let result = BenDecodeFrame::from_reader(&mut cursor, BenVariant::MkvChain).unwrap();
    assert!(result.is_none());
}

#[test]
fn ben_decode_mkv_truncated_count_errors() {
    // Header + payload but no count.
    let data: Vec<u8> = vec![2, 3, 0, 0, 0, 2, 0xAB, 0xCD];
    let mut cursor = io::Cursor::new(data);
    let err = BenDecodeFrame::from_reader(&mut cursor, BenVariant::MkvChain).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
}

#[test]
fn ben_decode_mkv_count_max_u16() {
    let data: Vec<u8> = vec![2, 3, 0, 0, 0, 2, 0xAB, 0xCD, 0xFF, 0xFF];
    let mut cursor = io::Cursor::new(data);
    let frame = BenDecodeFrame::from_reader(&mut cursor, BenVariant::MkvChain)
        .unwrap()
        .unwrap();
    let (_, _, _, _, count) = unwrap_mkv(frame);
    assert_eq!(count, u16::MAX);
}

#[test]
fn ben_decode_mkv_non_eof_read_error_propagates() {
    let err =
        BenDecodeFrame::from_reader(&mut ErrorAfterOneByte, BenVariant::MkvChain).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
}

// ── BenDecodeFrame::from_reader (TwoDelta) ──────────────────────────────────

#[test]
fn ben_decode_twodelta_from_reader() {
    // Build a TwoDelta encode frame, then read it back as a decode frame.
    let encoded = BenEncodeFrame::from_run_lengths((1, 2), vec![2, 2], Some(5));
    let bytes = encoded.into_bytes();

    let mut cursor = io::Cursor::new(bytes);
    let frame = BenDecodeFrame::from_reader(&mut cursor, BenVariant::TwoDelta)
        .unwrap()
        .unwrap();
    let (pair, run_lengths, count) = unwrap_twodelta(frame);
    assert_eq!(pair, (1, 2));
    assert_eq!(run_lengths, vec![2, 2]);
    assert_eq!(count, 5);
}

#[test]
fn ben_decode_twodelta_eof_returns_none() {
    let mut cursor = io::Cursor::new(Vec::<u8>::new());
    let result = BenDecodeFrame::from_reader(&mut cursor, BenVariant::TwoDelta).unwrap();
    assert!(result.is_none());
}

#[test]
fn ben_decode_twodelta_truncated_errors() {
    // Only the pair bytes; no max_len_bits, n_bytes, payload, or count.
    let data: Vec<u8> = vec![0, 1, 0, 2];
    let mut cursor = io::Cursor::new(data);
    let err = BenDecodeFrame::from_reader(&mut cursor, BenVariant::TwoDelta).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
}

#[test]
fn ben_decode_twodelta_invalid_max_len_bits_zero_errors() {
    // pair (4) + max_len_bits=0 (1) + n_bytes=0 (4) + count (2)
    let data: Vec<u8> = vec![0, 1, 0, 2, 0, 0, 0, 0, 0, 0, 0, 1];
    let mut cursor = io::Cursor::new(data);
    let err = BenDecodeFrame::from_reader(&mut cursor, BenVariant::TwoDelta).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn ben_decode_twodelta_count_max_u16() {
    let encoded = BenEncodeFrame::from_run_lengths((3, 4), vec![1, 1], Some(u16::MAX));
    let bytes = encoded.into_bytes();
    let mut cursor = io::Cursor::new(bytes);
    let frame = BenDecodeFrame::from_reader(&mut cursor, BenVariant::TwoDelta)
        .unwrap()
        .unwrap();
    let (_, _, count) = unwrap_twodelta(frame);
    assert_eq!(count, u16::MAX);
}

// ── BenEncodeFrame::from_rle ────────────────────────────────────────────────

#[test]
fn encode_from_rle_standard_carries_runs_and_bytes() {
    let runs = vec![(1u16, 2u16), (2, 3), (3, 1)];
    let frame = BenEncodeFrame::from_rle(runs.clone(), BenVariant::Standard, None);
    let (got_runs, mvb, mlb, n, raw) = unwrap_encode_standard(frame);
    assert_eq!(got_runs, runs);
    assert_eq!(mvb, 2); // max value 3 fits in 2 bits
    assert_eq!(mlb, 2); // max length 3 fits in 2 bits
    assert!(n > 0);
    assert_eq!(raw[0], mvb);
    assert_eq!(raw[1], mlb);
    assert_eq!(&raw[2..6], n.to_be_bytes().as_slice());
}

#[test]
fn encode_from_rle_mkv_count_none_defaults_to_one() {
    let runs = vec![(1u16, 2u16), (2, 3)];
    let frame = BenEncodeFrame::from_rle(runs, BenVariant::MkvChain, None);
    let (_, _, _, _, raw, count) = unwrap_encode_mkv(frame);
    assert_eq!(count, 1);
    let trailing = &raw[raw.len() - 2..];
    assert_eq!(trailing, 1u16.to_be_bytes());
}

#[test]
fn encode_from_rle_mkv_with_count() {
    let runs = vec![(1u16, 2u16)];
    let frame = BenEncodeFrame::from_rle(runs, BenVariant::MkvChain, Some(7));
    let (_, _, _, _, raw, count) = unwrap_encode_mkv(frame);
    assert_eq!(count, 7);
    let trailing = &raw[raw.len() - 2..];
    assert_eq!(trailing, 7u16.to_be_bytes());
}

#[test]
#[should_panic(expected = "TwoDelta")]
fn encode_from_rle_twodelta_panics() {
    let runs = vec![(1u16, 2u16)];
    let _ = BenEncodeFrame::from_rle(runs, BenVariant::TwoDelta, None);
}

#[test]
fn encode_single_run_frame() {
    let runs = vec![(5u16, 1u16)];
    let frame = BenEncodeFrame::from_rle(runs, BenVariant::Standard, None);
    let (_, mvb, mlb, _, _) = unwrap_encode_standard(frame);
    assert_eq!(mvb, 3); // 5 fits in 3 bits
    assert_eq!(mlb, 1); // 1 fits in 1 bit
}

#[test]
fn encode_large_values_near_u16_max() {
    let runs = vec![(u16::MAX, u16::MAX)];
    let frame = BenEncodeFrame::from_rle(runs, BenVariant::Standard, None);
    let (_, mvb, mlb, _, _) = unwrap_encode_standard(frame);
    assert_eq!(mvb, 16);
    assert_eq!(mlb, 16);
}

// ── BenEncodeFrame::from_assignment ─────────────────────────────────────────

#[test]
fn encode_from_assignment_standard() {
    let assignment = vec![1u16, 1, 2, 2, 3];
    let frame = BenEncodeFrame::from_assignment(&assignment, BenVariant::Standard, None);
    let (runs, _, _, _, _) = unwrap_encode_standard(frame);
    assert_eq!(runs, vec![(1, 2), (2, 2), (3, 1)]);
}

#[test]
fn encode_from_assignment_mkv_carries_count() {
    let assignment = vec![1u16, 1, 2, 2];
    let frame = BenEncodeFrame::from_assignment(&assignment, BenVariant::MkvChain, Some(9));
    let (_, _, _, _, _, count) = unwrap_encode_mkv(frame);
    assert_eq!(count, 9);
}

// ── BenEncodeFrame::from_run_lengths / from_parts (TwoDelta) ────────────────

#[test]
fn twodelta_from_run_lengths_count_none_defaults_to_one() {
    let frame = BenEncodeFrame::from_run_lengths((1, 2), vec![2, 2], None);
    let (pair, _, _, runs, _, count) = unwrap_encode_twodelta(frame);
    assert_eq!(pair, (1, 2));
    assert_eq!(runs, vec![2, 2]);
    assert_eq!(count, 1);
}

#[test]
fn twodelta_from_run_lengths_then_from_parts_roundtrip() {
    let original = BenEncodeFrame::from_run_lengths((3, 4), vec![5, 5, 5], Some(2));
    let bytes = original.as_slice().to_vec();
    let (pair, max_len_bits, n_bytes, _, _, count) = unwrap_encode_twodelta(original.clone());
    let payload_slice = &bytes[9..9 + n_bytes as usize];
    let rebuilt = BenEncodeFrame::from_parts(pair, max_len_bits, payload_slice.to_vec(), count);
    let (rb_pair, _, _, rb_runs, _, rb_count) = unwrap_encode_twodelta(rebuilt);
    assert_eq!(rb_pair, pair);
    assert_eq!(rb_runs, vec![5, 5, 5]);
    assert_eq!(rb_count, count);
}

#[test]
fn twodelta_from_parts_preserves_nontrivial_count() {
    let original = BenEncodeFrame::from_run_lengths((1, 9), vec![3, 3], Some(42));
    let bytes = original.as_slice().to_vec();
    let (_, max_len_bits, n_bytes, _, _, _) = unwrap_encode_twodelta(original);
    let payload = bytes[9..9 + n_bytes as usize].to_vec();
    let rebuilt = BenEncodeFrame::from_parts((1, 9), max_len_bits, payload, 42);
    let (_, _, _, _, _, count) = unwrap_encode_twodelta(rebuilt);
    assert_eq!(count, 42);
}

#[test]
fn twodelta_from_run_lengths_single_run() {
    let frame = BenEncodeFrame::from_run_lengths((1, 2), vec![5], Some(3));
    let (pair, _, _, runs, _, count) = unwrap_encode_twodelta(frame);
    assert_eq!(pair, (1, 2));
    assert_eq!(runs, vec![5]);
    assert_eq!(count, 3);
}

// ── Encode/decode roundtrips ────────────────────────────────────────────────

#[test]
fn standard_encode_decode_roundtrip() {
    let runs = vec![(1u16, 4u16), (2, 3), (3, 1)];
    let encoded = BenEncodeFrame::from_rle(runs.clone(), BenVariant::Standard, None);
    let bytes = encoded.into_bytes();

    let mut cursor = io::Cursor::new(bytes);
    let decoded = BenDecodeFrame::from_reader(&mut cursor, BenVariant::Standard)
        .unwrap()
        .unwrap();
    let (mvb, mlb, n_bytes, raw) = unwrap_standard(decoded);
    assert_eq!(mvb, 2);
    assert_eq!(mlb, 3);
    assert!(n_bytes > 0);
    assert!(!raw.is_empty());
}

#[test]
fn mkv_encode_decode_roundtrip() {
    let runs = vec![(1u16, 4u16), (2, 3)];
    let encoded = BenEncodeFrame::from_rle(runs, BenVariant::MkvChain, Some(11));
    let bytes = encoded.into_bytes();

    let mut cursor = io::Cursor::new(bytes);
    let decoded = BenDecodeFrame::from_reader(&mut cursor, BenVariant::MkvChain)
        .unwrap()
        .unwrap();
    let (_, _, _, _, count) = unwrap_mkv(decoded);
    assert_eq!(count, 11);
}

#[test]
fn twodelta_encode_decode_roundtrip() {
    let encoded = BenEncodeFrame::from_run_lengths((4, 7), vec![3, 3, 3], Some(8));
    let bytes = encoded.into_bytes();

    let mut cursor = io::Cursor::new(bytes);
    let decoded = BenDecodeFrame::from_reader(&mut cursor, BenVariant::TwoDelta)
        .unwrap()
        .unwrap();
    let (pair, runs, count) = unwrap_twodelta(decoded);
    assert_eq!(pair, (4, 7));
    assert_eq!(runs, vec![3, 3, 3]);
    assert_eq!(count, 8);
}

// ── Back-to-back parsing ────────────────────────────────────────────────────

#[test]
fn standard_decode_two_frames_back_to_back() {
    let f1 = BenEncodeFrame::from_rle(vec![(1, 2), (2, 1)], BenVariant::Standard, None);
    let f2 = BenEncodeFrame::from_rle(vec![(3, 1), (4, 2)], BenVariant::Standard, None);
    let mut bytes = f1.into_bytes();
    bytes.extend(f2.into_bytes());

    let mut cursor = io::Cursor::new(bytes);
    let _ = BenDecodeFrame::from_reader(&mut cursor, BenVariant::Standard)
        .unwrap()
        .unwrap();
    let _ = BenDecodeFrame::from_reader(&mut cursor, BenVariant::Standard)
        .unwrap()
        .unwrap();
    let none = BenDecodeFrame::from_reader(&mut cursor, BenVariant::Standard).unwrap();
    assert!(none.is_none());
}

#[test]
fn mkv_decode_two_frames_back_to_back() {
    let f1 = BenEncodeFrame::from_rle(vec![(1, 2)], BenVariant::MkvChain, Some(3));
    let f2 = BenEncodeFrame::from_rle(vec![(2, 4)], BenVariant::MkvChain, Some(5));
    let mut bytes = f1.into_bytes();
    bytes.extend(f2.into_bytes());

    let mut cursor = io::Cursor::new(bytes);
    let d1 = BenDecodeFrame::from_reader(&mut cursor, BenVariant::MkvChain)
        .unwrap()
        .unwrap();
    let d2 = BenDecodeFrame::from_reader(&mut cursor, BenVariant::MkvChain)
        .unwrap()
        .unwrap();
    assert_eq!(d1.count(), 3);
    assert_eq!(d2.count(), 5);
}

#[test]
fn twodelta_decode_two_frames_back_to_back() {
    let f1 = BenEncodeFrame::from_run_lengths((1, 2), vec![2, 2], Some(1));
    let f2 = BenEncodeFrame::from_run_lengths((3, 4), vec![1, 1, 1, 1], Some(1));
    let mut bytes = f1.into_bytes();
    bytes.extend(f2.into_bytes());

    let mut cursor = io::Cursor::new(bytes);
    let d1 = BenDecodeFrame::from_reader(&mut cursor, BenVariant::TwoDelta)
        .unwrap()
        .unwrap();
    let d2 = BenDecodeFrame::from_reader(&mut cursor, BenVariant::TwoDelta)
        .unwrap()
        .unwrap();
    assert_eq!(unwrap_twodelta(d1).0, (1, 2));
    assert_eq!(unwrap_twodelta(d2).0, (3, 4));
}

// ── Inspector methods (count, variant, raw_bytes) ───────────────────────────

#[test]
fn decode_count_returns_one_for_standard() {
    let encoded = BenEncodeFrame::from_rle(vec![(1, 1)], BenVariant::Standard, None);
    let mut cursor = io::Cursor::new(encoded.into_bytes());
    let frame = BenDecodeFrame::from_reader(&mut cursor, BenVariant::Standard)
        .unwrap()
        .unwrap();
    assert_eq!(frame.count(), 1);
}

#[test]
fn decode_variant_method() {
    let encoded = BenEncodeFrame::from_rle(vec![(1, 1)], BenVariant::Standard, None);
    let mut cursor = io::Cursor::new(encoded.into_bytes());
    let frame = BenDecodeFrame::from_reader(&mut cursor, BenVariant::Standard)
        .unwrap()
        .unwrap();
    assert_eq!(frame.variant(), BenVariant::Standard);

    let encoded = BenEncodeFrame::from_rle(vec![(1, 1)], BenVariant::MkvChain, Some(2));
    let mut cursor = io::Cursor::new(encoded.into_bytes());
    let frame = BenDecodeFrame::from_reader(&mut cursor, BenVariant::MkvChain)
        .unwrap()
        .unwrap();
    assert_eq!(frame.variant(), BenVariant::MkvChain);

    let encoded = BenEncodeFrame::from_run_lengths((1, 2), vec![1, 1], Some(1));
    let mut cursor = io::Cursor::new(encoded.into_bytes());
    let frame = BenDecodeFrame::from_reader(&mut cursor, BenVariant::TwoDelta)
        .unwrap()
        .unwrap();
    assert_eq!(frame.variant(), BenVariant::TwoDelta);
}

#[test]
fn decode_raw_bytes_returns_some_for_snapshot_arms_none_for_twodelta() {
    let std_encoded = BenEncodeFrame::from_rle(vec![(1, 1)], BenVariant::Standard, None);
    let mut cursor = io::Cursor::new(std_encoded.into_bytes());
    let std_frame = BenDecodeFrame::from_reader(&mut cursor, BenVariant::Standard)
        .unwrap()
        .unwrap();
    assert!(std_frame.raw_bytes().is_some());

    let mkv_encoded = BenEncodeFrame::from_rle(vec![(1, 1)], BenVariant::MkvChain, Some(1));
    let mut cursor = io::Cursor::new(mkv_encoded.into_bytes());
    let mkv_frame = BenDecodeFrame::from_reader(&mut cursor, BenVariant::MkvChain)
        .unwrap()
        .unwrap();
    assert!(mkv_frame.raw_bytes().is_some());

    let td_encoded = BenEncodeFrame::from_run_lengths((1, 2), vec![1, 1], Some(1));
    let mut cursor = io::Cursor::new(td_encoded.into_bytes());
    let td_frame = BenDecodeFrame::from_reader(&mut cursor, BenVariant::TwoDelta)
        .unwrap()
        .unwrap();
    assert!(td_frame.raw_bytes().is_none());
}

// ── Encode-side inspectors and conversions ──────────────────────────────────

#[test]
fn encode_as_slice_to_bytes_into_bytes_agree() {
    let encoded = BenEncodeFrame::from_rle(vec![(1, 2), (3, 4)], BenVariant::Standard, None);
    let s = encoded.as_slice().to_vec();
    let t = encoded.to_bytes();
    let i = encoded.into_bytes();
    assert_eq!(s, t);
    assert_eq!(s, i);
}

#[test]
fn encode_count_method() {
    let std_frame = BenEncodeFrame::from_rle(vec![(1, 1)], BenVariant::Standard, None);
    assert_eq!(std_frame.count(), 1);

    let mkv_frame = BenEncodeFrame::from_rle(vec![(1, 1)], BenVariant::MkvChain, Some(7));
    assert_eq!(mkv_frame.count(), 7);

    let td_frame = BenEncodeFrame::from_run_lengths((1, 2), vec![1, 1], Some(13));
    assert_eq!(td_frame.count(), 13);
}

#[test]
fn encode_variant_method() {
    let std_frame = BenEncodeFrame::from_rle(vec![(1, 1)], BenVariant::Standard, None);
    assert_eq!(std_frame.variant(), BenVariant::Standard);

    let mkv_frame = BenEncodeFrame::from_rle(vec![(1, 1)], BenVariant::MkvChain, None);
    assert_eq!(mkv_frame.variant(), BenVariant::MkvChain);

    let td_frame = BenEncodeFrame::from_run_lengths((1, 2), vec![1, 1], None);
    assert_eq!(td_frame.variant(), BenVariant::TwoDelta);
}

#[test]
fn encode_payload_returns_packed_payload_region() {
    let frame = BenEncodeFrame::from_rle(vec![(1, 2), (3, 4)], BenVariant::Standard, None);
    let bytes = frame.as_slice().to_vec();
    let payload = frame.payload().to_vec();
    // For Standard, payload is bytes[6..6+n_bytes].
    let (_, _, _, n_bytes, _) = unwrap_encode_standard(frame);
    assert_eq!(payload, bytes[6..6 + n_bytes as usize]);
}

#[test]
fn encode_as_ref_and_deref_match_as_slice() {
    let frame = BenEncodeFrame::from_rle(vec![(1, 2)], BenVariant::Standard, None);
    let s = frame.as_slice();
    let r: &[u8] = frame.as_ref();
    assert_eq!(s, r);
    // Deref makes slice methods callable directly.
    assert_eq!(frame.len(), s.len());
}

#[test]
fn encode_partial_eq_vec_both_directions() {
    let frame = BenEncodeFrame::from_rle(vec![(1, 2)], BenVariant::Standard, None);
    let bytes: Vec<u8> = frame.as_slice().to_vec();
    assert_eq!(frame, bytes);
    assert_eq!(bytes, frame);
}

// ── BenDecodeFrame::expand ──────────────────────────────────────────────────

#[test]
fn decode_expand_standard_assignment() {
    // An assignment of [1, 1, 2, 2, 3] becomes RLE [(1,2),(2,2),(3,1)].
    let encoded = BenEncodeFrame::from_assignment([1u16, 1, 2, 2, 3], BenVariant::Standard, None);
    let mut cursor = io::Cursor::new(encoded.into_bytes());
    let decoded = BenDecodeFrame::from_reader(&mut cursor, BenVariant::Standard)
        .unwrap()
        .unwrap();
    let assignment = decoded.expand(None).unwrap();
    assert_eq!(assignment, vec![1, 1, 2, 2, 3]);
}

#[test]
fn decode_expand_twodelta_requires_prev() {
    let encoded = BenEncodeFrame::from_run_lengths((1, 2), vec![2, 2], Some(1));
    let mut cursor = io::Cursor::new(encoded.into_bytes());
    let decoded = BenDecodeFrame::from_reader(&mut cursor, BenVariant::TwoDelta)
        .unwrap()
        .unwrap();
    let err = decoded.expand(None).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}
