use super::*;
use std::io;

// ── BenDecodeFrame ──────────────────────────────────────────────────────────

#[test]
fn ben_decode_frame_from_reader_standard_frame() {
    // Header: max_val_bits=2, max_len_bits=3, n_bytes=2
    // Payload: 2 bytes
    let data: Vec<u8> = vec![2, 3, 0, 0, 0, 2, 0xAB, 0xCD];
    let mut cursor = io::Cursor::new(data);
    let frame = BenDecodeFrame::from_reader(&mut cursor).unwrap().unwrap();
    assert_eq!(frame.max_val_bit_count, 2);
    assert_eq!(frame.max_len_bit_count, 3);
    assert_eq!(frame.n_bytes, 2);
    assert_eq!(frame.raw_bytes, vec![0xAB, 0xCD]);
}

#[test]
fn ben_decode_frame_from_reader_eof_returns_none() {
    let data: Vec<u8> = vec![];
    let mut cursor = io::Cursor::new(data);
    let result = BenDecodeFrame::from_reader(&mut cursor).unwrap();
    assert!(result.is_none());
}

#[test]
fn ben_decode_frame_from_reader_truncated_header_errors() {
    // Only 1 byte — too short for a full header
    let data: Vec<u8> = vec![2];
    let mut cursor = io::Cursor::new(data);
    let err = BenDecodeFrame::from_reader(&mut cursor).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
}

#[test]
fn ben_decode_frame_to_bytes() {
    let frame = BenDecodeFrame {
        max_val_bit_count: 2,
        max_len_bit_count: 3,
        n_bytes: 2,
        raw_bytes: vec![0xAB, 0xCD],
    };
    let bytes = frame.to_bytes();
    assert_eq!(bytes, vec![0xAB, 0xCD]);
    // Original frame still usable (not consumed)
    assert_eq!(frame.raw_bytes, vec![0xAB, 0xCD]);
}

#[test]
fn ben_decode_frame_into_bytes() {
    let frame = BenDecodeFrame {
        max_val_bit_count: 2,
        max_len_bit_count: 3,
        n_bytes: 2,
        raw_bytes: vec![0xAB, 0xCD],
    };
    let bytes = frame.into_bytes();
    assert_eq!(bytes, vec![0xAB, 0xCD]);
}

#[test]
fn ben_decode_frame_as_ref() {
    let frame = BenDecodeFrame {
        max_val_bit_count: 2,
        max_len_bit_count: 3,
        n_bytes: 2,
        raw_bytes: vec![0xAB, 0xCD],
    };
    let slice: &[u8] = frame.as_ref();
    assert_eq!(slice, &[0xAB, 0xCD]);
}

#[test]
fn ben_decode_frame_deref() {
    let frame = BenDecodeFrame {
        max_val_bit_count: 2,
        max_len_bit_count: 3,
        n_bytes: 2,
        raw_bytes: vec![0xAB, 0xCD],
    };
    // Deref lets us call slice methods directly
    assert_eq!(frame.len(), 2);
    assert_eq!(frame[0], 0xAB);
    assert_eq!(frame[1], 0xCD);
}

#[test]
fn ben_decode_frame_partial_eq_vec() {
    let frame = BenDecodeFrame {
        max_val_bit_count: 2,
        max_len_bit_count: 3,
        n_bytes: 2,
        raw_bytes: vec![0xAB, 0xCD],
    };
    let v = vec![0xAB, 0xCD];
    // Both directions
    assert_eq!(frame, v);
    assert_eq!(v, frame);
    // Inequality
    let v2 = vec![0xFF];
    assert_ne!(frame, v2);
    assert_ne!(v2, frame);
}

// ── MkvBenDecodeFrame ───────────────────────────────────────────────────────

#[test]
fn mkv_decode_frame_from_reader() {
    // Header: max_val_bits=2, max_len_bits=3, n_bytes=2
    // Payload: 2 bytes
    // Count: u16 BE = 5
    let data: Vec<u8> = vec![2, 3, 0, 0, 0, 2, 0xAB, 0xCD, 0, 5];
    let mut cursor = io::Cursor::new(data);
    let frame = MkvBenDecodeFrame::from_reader(&mut cursor).unwrap().unwrap();
    assert_eq!(frame.max_val_bit_count, 2);
    assert_eq!(frame.max_len_bit_count, 3);
    assert_eq!(frame.n_bytes, 2);
    assert_eq!(frame.raw_bytes, vec![0xAB, 0xCD]);
    assert_eq!(frame.count, 5);
}

#[test]
fn mkv_decode_frame_from_reader_eof_returns_none() {
    let data: Vec<u8> = vec![];
    let mut cursor = io::Cursor::new(data);
    let result = MkvBenDecodeFrame::from_reader(&mut cursor).unwrap();
    assert!(result.is_none());
}

#[test]
fn mkv_decode_frame_from_reader_truncated_count_errors() {
    // Valid header + payload, but missing count bytes
    let data: Vec<u8> = vec![2, 3, 0, 0, 0, 1, 0xFF];
    let mut cursor = io::Cursor::new(data);
    let err = MkvBenDecodeFrame::from_reader(&mut cursor).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
}

#[test]
fn mkv_decode_frame_to_bytes() {
    let frame = MkvBenDecodeFrame {
        max_val_bit_count: 2,
        max_len_bit_count: 3,
        n_bytes: 1,
        raw_bytes: vec![0xFF],
        count: 3,
    };
    let bytes = frame.to_bytes();
    assert_eq!(bytes, vec![0xFF]);
    assert_eq!(frame.raw_bytes, vec![0xFF]);
}

#[test]
fn mkv_decode_frame_into_bytes() {
    let frame = MkvBenDecodeFrame {
        max_val_bit_count: 2,
        max_len_bit_count: 3,
        n_bytes: 1,
        raw_bytes: vec![0xFF],
        count: 3,
    };
    let bytes = frame.into_bytes();
    assert_eq!(bytes, vec![0xFF]);
}

#[test]
fn mkv_decode_frame_as_ref() {
    let frame = MkvBenDecodeFrame {
        max_val_bit_count: 2,
        max_len_bit_count: 3,
        n_bytes: 1,
        raw_bytes: vec![0xFF],
        count: 3,
    };
    let slice: &[u8] = frame.as_ref();
    assert_eq!(slice, &[0xFF]);
}

#[test]
fn mkv_decode_frame_deref() {
    let frame = MkvBenDecodeFrame {
        max_val_bit_count: 2,
        max_len_bit_count: 3,
        n_bytes: 1,
        raw_bytes: vec![0xFF],
        count: 3,
    };
    assert_eq!(frame.len(), 1);
    assert_eq!(frame[0], 0xFF);
}

#[test]
fn mkv_decode_frame_partial_eq_vec() {
    let frame = MkvBenDecodeFrame {
        max_val_bit_count: 2,
        max_len_bit_count: 3,
        n_bytes: 1,
        raw_bytes: vec![0xFF],
        count: 3,
    };
    let v = vec![0xFF];
    assert_eq!(frame, v);
    assert_eq!(v, frame);
    let v2 = vec![0x00];
    assert_ne!(frame, v2);
    assert_ne!(v2, frame);
}

// ── MkvBenEncodeFrame ───────────────────────────────────────────────────────

#[test]
fn mkv_encode_frame_from_rle_count_none_defaults_to_1() {
    let runs = vec![(1u16, 4u16), (2, 1)];
    let frame = MkvBenEncodeFrame::from_rle(runs.clone(), None);
    assert_eq!(frame.count, 1);
    assert_eq!(frame.runs, runs);
}

#[test]
fn mkv_encode_frame_from_rle_with_count() {
    let runs = vec![(1u16, 4u16), (2, 1)];
    let frame = MkvBenEncodeFrame::from_rle(runs.clone(), Some(7));
    assert_eq!(frame.count, 7);
}

#[test]
fn mkv_encode_frame_to_bytes() {
    let frame = MkvBenEncodeFrame::from_rle(vec![(1u16, 2u16)], Some(1));
    let bytes = frame.to_bytes();
    assert_eq!(bytes, frame.raw_bytes);
    // Frame still usable
    assert!(!frame.raw_bytes.is_empty());
}

#[test]
fn mkv_encode_frame_into_bytes() {
    let frame = MkvBenEncodeFrame::from_rle(vec![(1u16, 2u16)], Some(1));
    let expected = frame.raw_bytes.clone();
    let bytes = frame.into_bytes();
    assert_eq!(bytes, expected);
}

#[test]
fn mkv_encode_frame_as_ref() {
    let frame = MkvBenEncodeFrame::from_rle(vec![(1u16, 2u16)], Some(1));
    let slice: &[u8] = frame.as_ref();
    assert_eq!(slice, &frame.raw_bytes);
}

#[test]
fn mkv_encode_frame_deref() {
    let frame = MkvBenEncodeFrame::from_rle(vec![(1u16, 2u16)], Some(1));
    assert_eq!(frame.len(), frame.raw_bytes.len());
}

#[test]
fn mkv_encode_frame_partial_eq_vec() {
    let frame = MkvBenEncodeFrame::from_rle(vec![(1u16, 2u16)], Some(1));
    let v = frame.raw_bytes.clone();
    assert_eq!(frame, v);
    assert_eq!(v, frame);
    let v2 = vec![0xFF, 0xFF, 0xFF];
    assert_ne!(frame, v2);
    assert_ne!(v2, frame);
}

// ── TwoDeltaDecodeFrame ─────────────────────────────────────────────────────

#[test]
fn twodelta_decode_frame_from_reader() {
    // pair: (0, 2), (0, 1), max_len_bits: 1, n_bytes: 0,0,0,1, payload: 0xC0, count: 0,1
    let data: Vec<u8> = vec![0, 2, 0, 1, 1, 0, 0, 0, 1, 0xC0, 0, 1];
    let mut cursor = io::Cursor::new(data);
    let frame = TwoDeltaDecodeFrame::from_reader(&mut cursor)
        .unwrap()
        .unwrap();
    assert_eq!(frame.pair, (2, 1));
    assert_eq!(frame.count, 1);
    assert!(!frame.run_lengths.is_empty());
}

#[test]
fn twodelta_decode_frame_from_reader_eof_returns_none() {
    let data: Vec<u8> = vec![];
    let mut cursor = io::Cursor::new(data);
    let result = TwoDeltaDecodeFrame::from_reader(&mut cursor).unwrap();
    assert!(result.is_none());
}

#[test]
fn twodelta_decode_frame_from_reader_truncated_errors() {
    // Only pair_a, missing pair_b
    let data: Vec<u8> = vec![0, 2];
    let mut cursor = io::Cursor::new(data);
    let err = TwoDeltaDecodeFrame::from_reader(&mut cursor).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
}

// ── Encode→Decode Roundtrips ────────────────────────────────────────────────

#[test]
fn ben_encode_decode_roundtrip_standard() {
    use crate::codec::decode::decode_ben_line;
    // Encode a Standard frame, then decode it via BenDecodeFrame::from_reader
    let runs = vec![(1u16, 4), (2, 1), (3, 3)];
    let encode_frame = BenEncodeFrame::from_rle(runs.clone(), None);

    // from_reader expects just the header+payload (no banner)
    let mut cursor = io::Cursor::new(encode_frame.as_slice());
    let decode_frame = BenDecodeFrame::from_reader(&mut cursor).unwrap().unwrap();

    assert_eq!(decode_frame.max_val_bit_count, encode_frame.max_val_bit_count);
    assert_eq!(decode_frame.max_len_bit_count, encode_frame.max_len_bit_count);
    assert_eq!(decode_frame.n_bytes, encode_frame.n_bytes);

    // Verify the payload decodes back to the original RLE runs
    let decoded_runs = decode_ben_line(
        io::Cursor::new(&decode_frame.raw_bytes),
        decode_frame.max_val_bit_count,
        decode_frame.max_len_bit_count,
        decode_frame.n_bytes,
    )
    .unwrap();
    assert_eq!(decoded_runs, runs);
}

#[test]
fn mkv_encode_decode_roundtrip() {
    use crate::codec::decode::decode_ben_line;
    let runs = vec![(1u16, 4), (2, 1), (3, 3)];
    let encode_frame = MkvBenEncodeFrame::from_rle(runs.clone(), Some(42));

    let mut cursor = io::Cursor::new(encode_frame.as_slice());
    let decode_frame = MkvBenDecodeFrame::from_reader(&mut cursor).unwrap().unwrap();

    assert_eq!(decode_frame.max_val_bit_count, encode_frame.max_val_bit_count);
    assert_eq!(decode_frame.max_len_bit_count, encode_frame.max_len_bit_count);
    assert_eq!(decode_frame.n_bytes, encode_frame.n_bytes);
    assert_eq!(decode_frame.count, 42);

    let decoded_runs = decode_ben_line(
        io::Cursor::new(&decode_frame.raw_bytes),
        decode_frame.max_val_bit_count,
        decode_frame.max_len_bit_count,
        decode_frame.n_bytes,
    )
    .unwrap();
    assert_eq!(decoded_runs, runs);
}

#[test]
fn twodelta_encode_decode_roundtrip() {
    use crate::codec::frames::twodelta_encode::TwoDeltaEncodeFrame;
    let run_lengths = vec![3u16, 2, 1, 4];
    let encode_frame =
        TwoDeltaEncodeFrame::from_run_lengths((5, 10), run_lengths.clone(), Some(7));

    // Write the raw_bytes (which include pair, max_len_bits, n_bytes, payload, count)
    let mut cursor = io::Cursor::new(encode_frame.as_slice());
    let decode_frame = TwoDeltaDecodeFrame::from_reader(&mut cursor).unwrap().unwrap();

    assert_eq!(decode_frame.pair, (5, 10));
    assert_eq!(decode_frame.count, 7);
    assert_eq!(decode_frame.run_lengths, run_lengths);
}

// ── Back-to-back frame reads ────────────────────────────────────────────────

#[test]
fn ben_decode_two_frames_back_to_back() {
    let f1 = BenEncodeFrame::from_rle(vec![(1u16, 2), (3, 4)], None);
    let f2 = BenEncodeFrame::from_rle(vec![(7u16, 1), (8, 1), (9, 1)], None);

    let mut data = Vec::new();
    data.extend_from_slice(f1.as_slice());
    data.extend_from_slice(f2.as_slice());

    let mut cursor = io::Cursor::new(data);
    let d1 = BenDecodeFrame::from_reader(&mut cursor).unwrap().unwrap();
    let d2 = BenDecodeFrame::from_reader(&mut cursor).unwrap().unwrap();
    let d3 = BenDecodeFrame::from_reader(&mut cursor).unwrap();

    assert_eq!(d1.max_val_bit_count, f1.max_val_bit_count);
    assert_eq!(d2.max_val_bit_count, f2.max_val_bit_count);
    assert!(d3.is_none()); // clean EOF
}

#[test]
fn mkv_decode_two_frames_back_to_back() {
    let f1 = MkvBenEncodeFrame::from_rle(vec![(1u16, 2)], Some(10));
    let f2 = MkvBenEncodeFrame::from_rle(vec![(5u16, 5)], Some(20));

    let mut data = Vec::new();
    data.extend_from_slice(f1.as_slice());
    data.extend_from_slice(f2.as_slice());

    let mut cursor = io::Cursor::new(data);
    let d1 = MkvBenDecodeFrame::from_reader(&mut cursor).unwrap().unwrap();
    let d2 = MkvBenDecodeFrame::from_reader(&mut cursor).unwrap().unwrap();
    let d3 = MkvBenDecodeFrame::from_reader(&mut cursor).unwrap();

    assert_eq!(d1.count, 10);
    assert_eq!(d2.count, 20);
    assert!(d3.is_none());
}

#[test]
fn twodelta_decode_two_frames_back_to_back() {
    use crate::codec::frames::twodelta_encode::TwoDeltaEncodeFrame;
    let f1 = TwoDeltaEncodeFrame::from_run_lengths((1, 2), vec![3, 2], Some(1));
    let f2 = TwoDeltaEncodeFrame::from_run_lengths((3, 4), vec![1, 1, 1], Some(5));

    let mut data = Vec::new();
    data.extend_from_slice(f1.as_slice());
    data.extend_from_slice(f2.as_slice());

    let mut cursor = io::Cursor::new(data);
    let d1 = TwoDeltaDecodeFrame::from_reader(&mut cursor).unwrap().unwrap();
    let d2 = TwoDeltaDecodeFrame::from_reader(&mut cursor).unwrap().unwrap();
    let d3 = TwoDeltaDecodeFrame::from_reader(&mut cursor).unwrap();

    assert_eq!(d1.pair, (1, 2));
    assert_eq!(d1.run_lengths, vec![3, 2]);
    assert_eq!(d1.count, 1);
    assert_eq!(d2.pair, (3, 4));
    assert_eq!(d2.run_lengths, vec![1, 1, 1]);
    assert_eq!(d2.count, 5);
    assert!(d3.is_none());
}

// ── Boundary values ─────────────────────────────────────────────────────────

#[test]
fn mkv_decode_frame_count_max_u16() {
    let f = MkvBenEncodeFrame::from_rle(vec![(1u16, 1)], Some(u16::MAX));
    let mut cursor = io::Cursor::new(f.as_slice());
    let d = MkvBenDecodeFrame::from_reader(&mut cursor).unwrap().unwrap();
    assert_eq!(d.count, u16::MAX);
}

#[test]
fn twodelta_decode_frame_count_max_u16() {
    use crate::codec::frames::twodelta_encode::TwoDeltaEncodeFrame;
    let f = TwoDeltaEncodeFrame::from_run_lengths((1, 2), vec![1, 1], Some(u16::MAX));
    let mut cursor = io::Cursor::new(f.as_slice());
    let d = TwoDeltaDecodeFrame::from_reader(&mut cursor).unwrap().unwrap();
    assert_eq!(d.count, u16::MAX);
}

#[test]
fn ben_encode_single_run_frame() {
    use crate::codec::decode::decode_ben_line;
    let runs = vec![(1u16, 1)];
    let frame = BenEncodeFrame::from_rle(runs.clone(), None);

    let mut cursor = io::Cursor::new(frame.as_slice());
    let decoded = BenDecodeFrame::from_reader(&mut cursor).unwrap().unwrap();

    let decoded_runs = decode_ben_line(
        io::Cursor::new(&decoded.raw_bytes),
        decoded.max_val_bit_count,
        decoded.max_len_bit_count,
        decoded.n_bytes,
    )
    .unwrap();
    assert_eq!(decoded_runs, runs);
}

#[test]
fn ben_encode_large_values_near_u16_max() {
    use crate::codec::decode::decode_ben_line;
    let runs = vec![(65534u16, 65535u16), (1, 1)];
    let frame = BenEncodeFrame::from_rle(runs.clone(), None);

    let mut cursor = io::Cursor::new(frame.as_slice());
    let decoded = BenDecodeFrame::from_reader(&mut cursor).unwrap().unwrap();

    let decoded_runs = decode_ben_line(
        io::Cursor::new(&decoded.raw_bytes),
        decoded.max_val_bit_count,
        decoded.max_len_bit_count,
        decoded.n_bytes,
    )
    .unwrap();
    assert_eq!(decoded_runs, runs);
}

#[test]
fn twodelta_from_run_lengths_then_from_parts_roundtrip() {
    use crate::codec::frames::twodelta_encode::TwoDeltaEncodeFrame;
    // Verify that packing via from_run_lengths then unpacking via from_parts
    // reproduces the same run_length_vector
    let run_lengths = vec![5u16, 3, 7, 1, 2];
    let encoded = TwoDeltaEncodeFrame::from_run_lengths((10, 20), run_lengths.clone(), None);

    let reconstructed = TwoDeltaEncodeFrame::from_parts(
        encoded.pair,
        encoded.max_len_bit_count,
        encoded.payload().to_vec(),
    );
    assert_eq!(reconstructed.run_length_vector, run_lengths);
    assert_eq!(reconstructed.pair, (10, 20));
}

#[test]
fn twodelta_from_run_lengths_single_run() {
    use crate::codec::frames::twodelta_encode::TwoDeltaEncodeFrame;
    let run_lengths = vec![100u16];
    let encoded = TwoDeltaEncodeFrame::from_run_lengths((1, 2), run_lengths.clone(), None);

    let mut cursor = io::Cursor::new(encoded.as_slice());
    let decoded = TwoDeltaDecodeFrame::from_reader(&mut cursor).unwrap().unwrap();
    assert_eq!(decoded.run_lengths, run_lengths);
}

// ── BenEncodeFrame trait impls ──────────────────────────────────────────────

#[test]
fn ben_encode_frame_partial_eq_vec_both_directions() {
    let frame = BenEncodeFrame::from_rle(vec![(1u16, 2)], None);
    let v = frame.raw_bytes.clone();
    assert_eq!(frame, v);
    assert_eq!(v, frame);
    let v2 = vec![0xFF, 0xFF, 0xFF];
    assert_ne!(frame, v2);
    assert_ne!(v2, frame);
}

#[test]
fn ben_encode_frame_as_ref_and_deref() {
    let frame = BenEncodeFrame::from_rle(vec![(1u16, 2)], None);
    let slice: &[u8] = frame.as_ref();
    assert_eq!(slice, &frame.raw_bytes[..]);
    assert_eq!(frame.len(), frame.raw_bytes.len());
}

#[test]
fn ben_encode_frame_to_bytes_and_into_bytes() {
    let frame = BenEncodeFrame::from_rle(vec![(1u16, 2)], None);
    let to = frame.to_bytes();
    let expected = frame.raw_bytes.clone();
    assert_eq!(to, expected);
    let into = frame.into_bytes();
    assert_eq!(into, expected);
}

// ── TwoDeltaEncodeFrame trait impls ─────────────────────────────────────────

#[test]
fn twodelta_encode_frame_as_ref_and_deref() {
    use crate::codec::frames::twodelta_encode::TwoDeltaEncodeFrame;
    let frame = TwoDeltaEncodeFrame::from_run_lengths((1, 2), vec![3, 2], None);
    let slice: &[u8] = frame.as_ref();
    assert_eq!(slice, &frame.raw_bytes[..]);
    assert_eq!(frame.len(), frame.raw_bytes.len());
}

#[test]
fn twodelta_encode_frame_to_bytes_and_into_bytes() {
    use crate::codec::frames::twodelta_encode::TwoDeltaEncodeFrame;
    let frame = TwoDeltaEncodeFrame::from_run_lengths((1, 2), vec![3, 2], None);
    let to = frame.to_bytes();
    let expected = frame.raw_bytes.clone();
    assert_eq!(to, expected);
    let into = frame.into_bytes();
    assert_eq!(into, expected);
}
