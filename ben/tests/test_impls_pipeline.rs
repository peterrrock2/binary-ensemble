#![allow(clippy::needless_collect)]

use binary_ensemble::codec::decode::{
    decode_ben_line, decode_ben_to_jsonl, decode_xben_to_ben, decode_xben_to_jsonl, xz_decompress,
};
use binary_ensemble::codec::encode::{
    encode_ben_to_xben, encode_jsonl_to_ben, encode_jsonl_to_xben, xz_compress,
};
use binary_ensemble::codec::{BenConstruct, BenEncodeFrame};
use binary_ensemble::io::reader::{
    build_frame_iter, count_samples_from_file, AssignmentReader, DecodeFrame, DecoderInitError,
    SubsampleFrameDecoder, XZAssignmentReader,
};
use binary_ensemble::io::writer::AssignmentWriter;
use binary_ensemble::ops::extract::extract_assignment_ben;
use binary_ensemble::BenVariant;

use proptest::prelude::*;
use serde_json::json;
use std::error::Error as _;
use std::fs;
use std::io::{BufReader, Cursor, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

// ---------- Helpers ----------

/// Expand an RLE sequence into a flat assignment Vec<u16>.
fn expand_rle(rle: &[(u16, u16)], cap: usize) -> Vec<u16> {
    let mut v = Vec::with_capacity(cap);
    for &(val, len) in rle {
        let take = (len as usize).min(cap.saturating_sub(v.len()));
        v.extend(std::iter::repeat(val).take(take));
        if v.len() >= cap {
            break;
        }
    }
    v
}

/// Generate a JSONL buffer from a sequence of assignment vectors.
fn jsonl_from_assignments(assignments: &[Vec<u16>]) -> Vec<u8> {
    let mut buf = Vec::new();
    for (i, a) in assignments.iter().enumerate() {
        let line = json!({ "assignment": a, "sample": i + 1 }).to_string();
        writeln!(&mut buf, "{line}").unwrap();
    }
    buf
}

/// From a decoded `(assignment, count)` stream, reconstitute JSONL.
fn jsonl_from_records(records: &[(Vec<u16>, u16)], start_at: usize) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut sample = start_at;
    for (a, c) in records {
        for _ in 0..*c {
            sample += 1;
            let line = json!({"assignment": a, "sample": sample}).to_string();
            writeln!(&mut buf, "{line}").unwrap();
        }
    }
    buf
}

/// Collect any iterator/into-iterator of `io::Result<MkvRecord>` into a Vec.
fn collect_records<I>(it: I) -> std::io::Result<Vec<(Vec<u16>, u16)>>
where
    I: IntoIterator<Item = std::io::Result<(Vec<u16>, u16)>>, // = MkvRecord
{
    let mut out = Vec::new();
    for rec in it {
        out.push(rec?);
    }
    Ok(out)
}

fn collect_frames<I>(it: I) -> std::io::Result<Vec<(binary_ensemble::io::reader::DecodeFrame, u16)>>
where
    I: IntoIterator<Item = std::io::Result<(binary_ensemble::io::reader::DecodeFrame, u16)>>,
{
    let mut out = Vec::new();
    for rec in it {
        out.push(rec?);
    }
    Ok(out)
}

// ---------- proptest strategies ----------

/// Strategy for a single assignment vector:
/// Generate as RLE runs (value in [1, max_val], length in [1, max_run]),
/// expand to a bounded length.
fn strat_assignment(max_val: u16, max_run: u16, max_len: usize) -> impl Strategy<Value = Vec<u16>> {
    // up to ~50 runs per vector to keep things small/fast
    let runs = 1..=50usize;
    (
        runs,
        prop::collection::vec((1u16..=max_val, 1u16..=max_run), 1..=50),
    )
        .prop_map(move |(_n, rle)| expand_rle(&rle, max_len))
        .prop_filter("non-empty vector", |v| !v.is_empty())
}

/// Strategy for a sequence of assignments with possible duplicates (to exercise MKV grouping).
fn strat_assignment_seq() -> impl Strategy<Value = Vec<Vec<u16>>> {
    // up to 60 samples (keep test runtime bounded)
    prop::collection::vec(strat_assignment(2000, 300, 1500), 1..=60)
        // Inject occasional exact duplicates by randomly repeating a previous element.
        .prop_map(|mut seq| {
            if seq.len() >= 2 {
                for i in (1..seq.len()).step_by(5) {
                    seq[i] = seq[i - 1].clone();
                }
            }
            seq
        })
}

/// Strategy for sequences where every transition is valid for TwoDelta.
fn strat_twodelta_seq() -> impl Strategy<Value = Vec<Vec<u16>>> {
    (
        strat_assignment(32, 24, 300),
        prop::collection::vec(any::<u64>(), 0..=40),
    )
        .prop_map(|(base, ops)| {
            let mut current = base;
            let mut seq = vec![current.clone()];

            for op in ops {
                let mut next = current.clone();
                let mut distinct: Vec<u16> = current.clone();
                distinct.sort_unstable();
                distinct.dedup();

                if distinct.len() < 2 || op % 5 == 0 {
                    seq.push(next.clone());
                    current = next;
                    continue;
                }

                let a = distinct[(op as usize) % distinct.len()];
                let mut b = distinct[((op >> 8) as usize) % distinct.len()];
                if a == b {
                    b = distinct
                        [(distinct.iter().position(|&x| x == a).unwrap() + 1) % distinct.len()];
                }

                let positions: Vec<usize> = current
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, &value)| ((value == a) || (value == b)).then_some(idx))
                    .collect();

                if positions.is_empty() {
                    seq.push(next.clone());
                    current = next;
                    continue;
                }

                let mut remaining = positions.len();
                let mut write_idx = 0usize;
                let mut seed = op.rotate_left(13) ^ 0x9E37_79B9_7F4A_7C15;
                let mut value = if op & 1 == 0 { a } else { b };

                while remaining > 0 {
                    let run_len = 1 + (seed as usize % remaining);
                    for _ in 0..run_len {
                        next[positions[write_idx]] = value;
                        write_idx += 1;
                    }
                    remaining -= run_len;
                    value = if value == a { b } else { a };
                    seed = seed.rotate_left(7) ^ 0xA076_1D64_78BD_642F;
                }

                seq.push(next.clone());
                current = next;
            }

            seq
        })
        .prop_filter("TwoDelta sequences must be non-empty", |seq| {
            !seq.is_empty()
        })
}

// Random (small) thread count and compression level for MT encoder.
fn strat_threads_levels() -> impl Strategy<Value = (u32, u32)> {
    (1u32..=4, 0u32..=9)
}

// ---------- Tests ----------

proptest! {
    // JSONL -> BEN(Standard) -> JSONL round-trip via BenEncoder/AssignmentReader entry points.
    #[test]
    fn fuzz_roundtrip_ben_standard(seq in strat_assignment_seq()) {
        let jsonl = jsonl_from_assignments(&seq);
        let mut ben = Vec::new();
        encode_jsonl_to_ben(BufReader::new(jsonl.as_slice()), &mut ben, BenVariant::Standard).unwrap();

        let mut out = Vec::new();
        decode_ben_to_jsonl(ben.as_slice(), &mut out).unwrap();

        prop_assert_eq!(out, jsonl);
    }

    // JSONL -> BEN(MkvChain) -> JSONL round-trip.
    #[test]
    fn fuzz_roundtrip_ben_mkv(seq in strat_assignment_seq()) {
        let jsonl = jsonl_from_assignments(&seq);
        let mut ben = Vec::new();
        encode_jsonl_to_ben(BufReader::new(jsonl.as_slice()), &mut ben, BenVariant::MkvChain).unwrap();

        let mut out = Vec::new();
        decode_ben_to_jsonl(ben.as_slice(), &mut out).unwrap();

        prop_assert_eq!(out, jsonl);
    }

    // JSONL -> BEN(TwoDelta) -> JSONL round-trip.
    #[test]
    fn fuzz_roundtrip_ben_twodelta(seq in strat_twodelta_seq()) {
        let jsonl = jsonl_from_assignments(&seq);
        let mut ben = Vec::new();
        encode_jsonl_to_ben(BufReader::new(jsonl.as_slice()), &mut ben, BenVariant::TwoDelta).unwrap();

        let mut out = Vec::new();
        decode_ben_to_jsonl(ben.as_slice(), &mut out).unwrap();

        prop_assert_eq!(out, jsonl);
    }

    // JSONL -> XBEN(Standard)  -> BEN -> JSONL
    // Also vary threads & compression level.
    #[test]
    fn fuzz_roundtrip_xben_standard(seq in strat_assignment_seq(), params in strat_threads_levels()) {
        let (threads, level) = params;
        let jsonl = jsonl_from_assignments(&seq);

        let mut xben = Vec::new();
        encode_jsonl_to_xben(
            BufReader::new(jsonl.as_slice()),
            &mut xben,
            BenVariant::Standard,
            Some(threads),
            Some(level),
            None,
        ).unwrap();

        // Decode XBEN -> BEN -> JSONL
        let mut ben = Vec::new();
        decode_xben_to_ben(BufReader::new(xben.as_slice()), &mut ben).unwrap();

        let mut out = Vec::new();
        decode_ben_to_jsonl(ben.as_slice(), &mut out).unwrap();

        prop_assert_eq!(out, jsonl);
    }

    // JSONL -> XBEN(MkvChain) -> BEN -> JSONL
    #[test]
    fn fuzz_roundtrip_xben_mkv(seq in strat_assignment_seq(), params in strat_threads_levels()) {
        let (threads, level) = params;
        let jsonl = jsonl_from_assignments(&seq);

        let mut xben = Vec::new();
        encode_jsonl_to_xben(
            BufReader::new(jsonl.as_slice()),
            &mut xben,
            BenVariant::MkvChain,
            Some(threads),
            Some(level),
            None,
        ).unwrap();

        let mut ben = Vec::new();
        decode_xben_to_ben(BufReader::new(xben.as_slice()), &mut ben).unwrap();

        let mut out = Vec::new();
        decode_ben_to_jsonl(ben.as_slice(), &mut out).unwrap();

        prop_assert_eq!(out, jsonl);
    }

    // JSONL -> XBEN(TwoDelta) -> BEN -> JSONL
    #[test]
    fn fuzz_roundtrip_xben_twodelta(seq in strat_twodelta_seq(), params in strat_threads_levels()) {
        let (threads, level) = params;
        let jsonl = jsonl_from_assignments(&seq);

        let mut xben = Vec::new();
        encode_jsonl_to_xben(
            BufReader::new(jsonl.as_slice()),
            &mut xben,
            BenVariant::TwoDelta,
            Some(threads),
            Some(level),
            None,
        ).unwrap();

        let mut ben = Vec::new();
        decode_xben_to_ben(BufReader::new(xben.as_slice()), &mut ben).unwrap();

        let mut out = Vec::new();
        decode_ben_to_jsonl(ben.as_slice(), &mut out).unwrap();

        prop_assert_eq!(out, jsonl);
    }

    // Direct XBEN -> JSONL via jsonl_decode_xben matches the long path.
    #[test]
    fn fuzz_decode_xben_direct_equals_via_ben(seq in strat_assignment_seq(), params in strat_threads_levels()) {
        let (threads, level) = params;
        let jsonl = jsonl_from_assignments(&seq);

        let mut xben = Vec::new();
        encode_jsonl_to_xben(
            BufReader::new(jsonl.as_slice()),
            &mut xben,
            BenVariant::MkvChain,
            Some(threads),
            Some(level),
            None,
        ).unwrap();

        // Path A: direct to JSONL
        let mut direct = Vec::new();
        decode_xben_to_jsonl(BufReader::new(xben.as_slice()), &mut direct).unwrap();

        // Path B: XBEN -> BEN -> JSONL
        let mut ben = Vec::new();
        decode_xben_to_ben(BufReader::new(xben.as_slice()), &mut ben).unwrap();
        let mut via = Vec::new();
        decode_ben_to_jsonl(ben.as_slice(), &mut via).unwrap();

        prop_assert_eq!(direct, via);
    }

    // Iterator surface: XZAssignmentReader -> records matches direct JSONL
    #[test]
    fn fuzz_xbendecoder_iterator_matches_jsonl(seq in strat_assignment_seq(), params in strat_threads_levels()) {
        let (threads, level) = params;
        let jsonl = jsonl_from_assignments(&seq);

        let mut xben = Vec::new();
        encode_jsonl_to_xben(
            BufReader::new(jsonl.as_slice()),
            &mut xben,
            BenVariant::Standard,
            Some(threads),
            Some(level),
            None,
        ).unwrap();

        let mut dec = XZAssignmentReader::new(xben.as_slice()).unwrap();
        let recs = collect_records(&mut dec).unwrap();

        let iter_jsonl = jsonl_from_records(&recs, 0);

        // Also decode via the library jsonl_decode_xben and compare.
        let mut direct = Vec::new();
        decode_xben_to_jsonl(BufReader::new(xben.as_slice()), &mut direct).unwrap();

        prop_assert_eq!(iter_jsonl, direct);
    }

    // Iterator surface: XZAssignmentReader over TwoDelta XBEN matches direct JSONL.
    #[test]
    fn fuzz_xbendecoder_iterator_matches_jsonl_twodelta(seq in strat_twodelta_seq(), params in strat_threads_levels()) {
        let (threads, level) = params;
        let jsonl = jsonl_from_assignments(&seq);

        let mut xben = Vec::new();
        encode_jsonl_to_xben(
            BufReader::new(jsonl.as_slice()),
            &mut xben,
            BenVariant::TwoDelta,
            Some(threads),
            Some(level),
            None,
        ).unwrap();

        let mut dec = XZAssignmentReader::new(xben.as_slice()).unwrap();
        let recs = collect_records(&mut dec).unwrap();
        let iter_jsonl = jsonl_from_records(&recs, 0);

        let mut direct = Vec::new();
        decode_xben_to_jsonl(BufReader::new(xben.as_slice()), &mut direct).unwrap();

        prop_assert_eq!(iter_jsonl, direct);
    }

    // Iterator surface: AssignmentReader over BEN produced by BenEncoder.
    #[test]
    fn fuzz_bendecoder_iterator_matches_jsonl(seq in strat_assignment_seq()) {
        let jsonl = jsonl_from_assignments(&seq);

        // Build BEN(Standard)
        let mut ben = Vec::new();
        encode_jsonl_to_ben(BufReader::new(jsonl.as_slice()), &mut ben, BenVariant::Standard).unwrap();

        // Iterate AssignmentReader
        let mut dec = AssignmentReader::new(ben.as_slice()).unwrap();
        let recs = collect_records(&mut dec).unwrap();
        let out = jsonl_from_records(&recs, 0);
        prop_assert_eq!(out, jsonl);

    }

    // Iterator surface: AssignmentReader over TwoDelta BEN matches JSONL.
    #[test]
    fn fuzz_bendecoder_iterator_matches_jsonl_twodelta(seq in strat_twodelta_seq()) {
        let jsonl = jsonl_from_assignments(&seq);

        let mut ben = Vec::new();
        encode_jsonl_to_ben(BufReader::new(jsonl.as_slice()), &mut ben, BenVariant::TwoDelta).unwrap();

        let mut dec = AssignmentReader::new(ben.as_slice()).unwrap();
        let recs = collect_records(&mut dec).unwrap();
        let out = jsonl_from_records(&recs, 0);
        prop_assert_eq!(out, jsonl);
    }

    // SubsampleDecoder: select indices (by_indices)
    #[test]
    fn fuzz_subsample_by_indices(seq in strat_assignment_seq(), params in strat_threads_levels()) {
        let (threads, level) = params;
        let jsonl = jsonl_from_assignments(&seq);

        // Build an XBEN with MKV to exercise counts in SubsampleDecoder
        let mut xben = Vec::new();
        encode_jsonl_to_xben(
            BufReader::new(jsonl.as_slice()),
            &mut xben,
            BenVariant::MkvChain,
            Some(threads),
            Some(level),
            None,
        ).unwrap();

        // Choose some indices to keep (1-based). We derive from seq length.
        let n = seq.len().max(1);
        let mut want: Vec<usize> = (1..=n).step_by(3).collect(); // 1,4,7,…
        if want.is_empty() { want.push(1); }

        let xb = XZAssignmentReader::new(xben.as_slice()).unwrap();
        let mut sub = xb.into_subsample_by_indices(want.clone());
        let recs = collect_records(&mut sub).unwrap();

        // Ground truth: take those rows from original seq.
        let truth: Vec<Vec<u16>> = (1..=n)
            .zip(seq.iter())
            .filter(|(i, _)| want.contains(i))
            .map(|(_, v)| v.clone())
            .collect();

        // Expand records (assignment,count) into a flat sequence of assignments to compare.
        let mut picked: Vec<Vec<u16>> = Vec::new();
        for (a, c) in recs {
            for _ in 0..c { picked.push(a.clone()); }
        }

        prop_assert_eq!(picked, truth);
    }

    // SubsampleDecoder: every(step, offset)
    #[test]
    fn fuzz_subsample_every(seq in strat_assignment_seq(), params in strat_threads_levels(), step in 1usize..=7usize, offset in 1usize..=5usize) {
        let (threads, level) = params;
        let jsonl = jsonl_from_assignments(&seq);

        let mut xben = Vec::new();
        encode_jsonl_to_xben(
            BufReader::new(jsonl.as_slice()),
            &mut xben,
            BenVariant::MkvChain,
            Some(threads),
            Some(level),
            None,
        ).unwrap();

        let n = seq.len();
        let mut truth: Vec<Vec<u16>> = Vec::new();
        for i in 1..=n {
            if i >= offset && (i - offset) % step == 0 {
                truth.push(seq[i-1].clone());
            }
        }

        let xb = XZAssignmentReader::new(xben.as_slice()).unwrap();
        let mut sub = xb.into_subsample_every(step, offset);
        let recs = collect_records(&mut sub).unwrap();

        let mut picked: Vec<Vec<u16>> = Vec::new();
        for (a, c) in recs {
            for _ in 0..c { picked.push(a.clone()); }
        }

        prop_assert_eq!(picked, truth);
    }

    // SubsampleDecoder: by_range
    #[test]
    fn fuzz_subsample_range(seq in strat_assignment_seq(), params in strat_threads_levels(), start in 1usize..=5usize, len in 1usize..=10usize) {
        let (threads, level) = params;
        let jsonl = jsonl_from_assignments(&seq);

        let mut xben = Vec::new();
        encode_jsonl_to_xben(
            BufReader::new(jsonl.as_slice()),
            &mut xben,
            BenVariant::MkvChain,
            Some(threads),
            Some(level),
            None,
        ).unwrap();

        let n = seq.len();
        let s = start.min(n.max(1));
        let e = (s + len).min(n);

        let truth: Vec<Vec<u16>> = (s..=e).map(|i| seq[i-1].clone()).collect();

        let xb = XZAssignmentReader::new(xben.as_slice()).unwrap();
        let mut sub = xb.into_subsample_by_range(s, e);
        let recs = collect_records(&mut sub).unwrap();

        let mut picked: Vec<Vec<u16>> = Vec::new();
        for (a, c) in recs {
            for _ in 0..c { picked.push(a.clone()); }
        }

        prop_assert_eq!(picked, truth);
    }

    #[test]
    fn fuzz_subsample_by_indices_twodelta(seq in strat_twodelta_seq()) {
        let jsonl = jsonl_from_assignments(&seq);
        let mut ben = Vec::new();
        encode_jsonl_to_ben(BufReader::new(jsonl.as_slice()), &mut ben, BenVariant::TwoDelta).unwrap();

        let n = seq.len().max(1);
        let mut want: Vec<usize> = (1..=n).step_by(3).collect();
        if want.is_empty() {
            want.push(1);
        }

        let mut sub = AssignmentReader::new(ben.as_slice())
            .unwrap()
            .into_subsample_by_indices(want.clone());
        let recs = collect_records(&mut sub).unwrap();

        let truth: Vec<Vec<u16>> = (1..=n)
            .zip(seq.iter())
            .filter(|(i, _)| want.contains(i))
            .map(|(_, v)| v.clone())
            .collect();

        let mut picked: Vec<Vec<u16>> = Vec::new();
        for (assignment, count) in recs {
            for _ in 0..count {
                picked.push(assignment.clone());
            }
        }

        prop_assert_eq!(picked, truth);
    }

    // xz_compress / xz_decompress round-trip on arbitrary bytes.
    #[test]
    fn fuzz_xz_roundtrip(bytes in proptest::collection::vec(any::<u8>(), 0..=200_000), params in strat_threads_levels()) {
        let (threads, level) = params;

        let mut out = Vec::new();
        xz_compress(BufReader::new(bytes.as_slice()), &mut out, Some(threads), Some(level)).unwrap();

        let mut recovered = Vec::new();
        xz_decompress(BufReader::new(out.as_slice()), &mut recovered).unwrap();

        prop_assert_eq!(recovered, bytes);
    }
}

// ---------- Non-proptest unit checks for headers/validation ----------

#[test]
fn invalid_ben_header_yields_error() {
    let mut bogus = Vec::new();
    bogus.extend_from_slice(b"NOT A BEN HEADER!");
    bogus.resize(17, 0);

    let err = AssignmentReader::new(Cursor::new(bogus))
        .err()
        .expect("expeced InvalidFileFormat error");
    match err {
        DecoderInitError::InvalidFileFormat(_) => {}
        other => panic!("expected InvalidFileFormat, got {other:?}"),
    }
}

#[test]
fn xben_decoder_rejects_bad_banner() {
    // Valid XZ container but wrong banner should raise InvalidData
    // Build a minimal XBEN stream with a wrong banner inside.
    let mut inner = Vec::new();
    inner.extend_from_slice(b"BAD BAD BAD BAD!!"); // 17 bytes
    let mut xz = Vec::new();
    xz_compress(BufReader::new(inner.as_slice()), &mut xz, Some(1), Some(0)).unwrap();

    let err = XZAssignmentReader::new(xz.as_slice())
        .err()
        .expect("expeced InvalidFileFormat error");
    assert_eq!(
        std::io::Error::from(err).kind(),
        std::io::ErrorKind::InvalidData
    );
}

#[test]
fn subsample_every_respects_offset() {
    // Build an XBEN(MkvChain) stream with two identical samples
    let seq = vec![vec![1u16], vec![1u16]];
    let jsonl = jsonl_from_assignments(&seq);
    let mut xben = Vec::new();
    encode_jsonl_to_xben(
        std::io::BufReader::new(jsonl.as_slice()),
        &mut xben,
        BenVariant::MkvChain,
        Some(1),
        Some(0),
        None,
    )
    .unwrap();

    // Keep every 1 starting at offset=2 -> only second sample.
    let xb = XZAssignmentReader::new(xben.as_slice()).unwrap();
    let mut sub = xb.into_subsample_every(1, 2);
    let recs = collect_records(&mut sub).unwrap();

    let mut picked = Vec::new();
    for (a, c) in recs {
        for _ in 0..c {
            picked.push(a.clone());
        }
    }

    assert_eq!(picked, vec![vec![1u16]]);
}

#[test]
fn benencoder_finish_flushes_once() {
    let lines = r#"{"assignment":[1,1,1],"sample":1}
{"assignment":[1,1,1],"sample":2}
{"assignment":[2,2],"sample":3}
"#;

    let mut ben_vec = Vec::new();
    {
        let mut enc = AssignmentWriter::new(&mut ben_vec, BenVariant::MkvChain).unwrap();
        for line in lines.lines() {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            enc.write_json_value(v).unwrap();
        }
        enc.finish().unwrap();
        // second finish should be a no-op
        enc.finish().unwrap();
    } // Forces enc to drop

    let mut out = Vec::new();
    decode_ben_to_jsonl(ben_vec.as_slice(), &mut out).unwrap();
    assert_eq!(out, lines.as_bytes());
}

#[test]
fn xbenencoder_drop_flushes_tail_group() {
    let jsonl = r#"{"assignment":[5,5],"sample":1}
{"assignment":[5,5],"sample":2}
{"assignment":[5,5],"sample":3}
{"assignment":[7],"sample":4}
"#;
    // Scope to force Drop
    let xz = {
        let mut out = Vec::new();
        encode_jsonl_to_xben(
            BufReader::new(jsonl.as_bytes()),
            &mut out,
            BenVariant::MkvChain,
            Some(1),
            Some(0),
            None,
        )
        .unwrap();
        out
    };

    let mut ben = Vec::new();
    decode_xben_to_ben(xz.as_slice(), &mut ben).unwrap();

    let mut round = Vec::new();
    decode_ben_to_jsonl(ben.as_slice(), &mut round).unwrap();
    assert_eq!(round, jsonl.as_bytes());
}

#[test]
fn ben_new_invalid_header_detects_xz() {
    // XZ stream whose first bytes are an XZ header (not a ben banner)
    let mut xz = Vec::new();
    xz_compress(
        std::io::BufReader::new(b"hello".as_slice()),
        &mut xz,
        Some(1),
        Some(0),
    )
    .unwrap();

    // Try to treat it as BEN
    let err = AssignmentReader::new(xz.as_slice())
        .err()
        .expect("expected error");
    match err {
        DecoderInitError::InvalidFileFormat(bytes) => {
            // first 6 bytes should match XZ magic
            assert!(bytes.len() >= 6 && &bytes[..6] == b"\xFD\x37\x7A\x58\x5A\x00");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn xben_new_invalid_banner() {
    // Make an xz stream with a WRONG banner
    let mut wrong = Vec::new();
    // 17 bytes but not STANDARD/MKVCHAIN BEN FILE
    let inner = b"NOT A BEN HEADER!!";
    xz_compress(
        std::io::BufReader::new(inner.as_slice()),
        &mut wrong,
        Some(1),
        Some(0),
    )
    .unwrap();
    let err = XZAssignmentReader::new(wrong.as_slice())
        .err()
        .expect("expected invalid data");
    assert_eq!(
        std::io::Error::from(err).kind(),
        std::io::ErrorKind::InvalidData
    );
}

#[test]
fn xben_truncated_frame_reports_unexpected_eof() {
    // Build a tiny XBEN then truncate payload bytes
    let jsonl = r#"{"assignment":[1,1,1],"sample":1}
{"assignment":[1],"sample":2}
"#;
    let mut xz = Vec::new();
    encode_jsonl_to_xben(
        std::io::BufReader::new(jsonl.as_bytes()),
        &mut xz,
        BenVariant::Standard,
        Some(1),
        Some(0),
        None,
    )
    .unwrap();

    // Trim the last byte to force partial frame after decompress
    let trimmed = &xz[..xz.len() - 1];
    // Iterating should surface UnexpectedEof (partial frame)
    let mut it = XZAssignmentReader::new(trimmed).unwrap();
    // Drain until error
    while let Some(res) = it.next() {
        if let Err(e) = res {
            assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof);
            return;
        }
    }
    panic!("expected an UnexpectedEof error");
}

#[test]
fn encode_decode_ben32_odd_bit_packing_roundtrip() {
    // values up to 3 (2 bits), lengths big to make non-byte boundary
    let rle = vec![(1u16, 3u16), (2, 5), (3, 7)];
    let ben_frame = BenEncodeFrame::from_rle(rle.clone(), None);
    let ben = ben_frame.as_slice();
    // ben layout: [max_val_bits, max_len_bits, n_bytes, payload...]
    let max_val_bits = ben[0];
    let max_len_bits = ben[1];
    let n_bytes = u32::from_be_bytes([ben[2], ben[3], ben[4], ben[5]]);
    let payload = &ben[6..6 + n_bytes as usize];
    let decoded = decode_ben_line(payload, max_val_bits, max_len_bits, n_bytes).unwrap();
    assert_eq!(
        decoded,
        rle.into_iter()
            .flat_map(|(v, c)| std::iter::repeat((v, 1)).take(c as usize))
            .fold(Vec::<(u16, u16)>::new(), |mut acc, (v, _)| {
                if let Some(last) = acc.last_mut() {
                    if last.0 == v {
                        last.1 += 1;
                        return acc;
                    }
                }
                acc.push((v, 1));
                acc
            })
    );
}

#[test]
fn encode_jsonl_to_ben_rejects_bad_assignment_shapes() {
    let bads = [
        r#"{"assignment": "not an array", "sample":1}"#,
        r#"{"assignment": [1,2,3.5], "sample":1}"#,
        r#"{"sample":1}"#,
        &format!(r#"{{"assignment":[{}],"sample":1}}"#, (u32::MAX as u64)),
    ];
    for s in bads {
        let mut out = Vec::new();
        let err = encode_jsonl_to_ben(BufReader::new(s.as_bytes()), &mut out, BenVariant::Standard)
            .err()
            .expect("expected invalid data");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }
}

#[test]
fn subsample_by_indices_sorts_and_dedups() {
    // Build 5 distinct samples 1..=5
    let seq = vec![vec![1u16], vec![2], vec![3], vec![4], vec![5]];
    let jsonl = {
        let mut b = Vec::new();
        for (i, a) in seq.iter().enumerate() {
            writeln!(
                &mut b,
                "{}",
                serde_json::json!({"assignment":a,"sample":i+1})
            )
            .unwrap();
        }
        b
    };
    let mut xz = Vec::new();
    encode_jsonl_to_xben(
        std::io::BufReader::new(jsonl.as_slice()),
        &mut xz,
        BenVariant::Standard,
        Some(1),
        Some(0),
        None,
    )
    .unwrap();
    let xb = XZAssignmentReader::new(xz.as_slice()).unwrap();

    // Deliberately unsorted and duplicated indices
    let mut sub = xb.into_subsample_by_indices(vec![5, 2, 2, 1, 5, 3]);
    let recs = collect_records(&mut sub).unwrap();
    let mut picked = Vec::new();
    for (a, c) in recs {
        for _ in 0..c {
            picked.push(a[0]);
        }
    }
    assert_eq!(picked, vec![1, 2, 3, 5]); // sorted & deduped applied
}

#[test]
fn ben_encode_xben_respects_existing_ben_header() {
    let cases = [
        (
            BenVariant::Standard,
            r#"{"assignment":[1,1],"sample":1}
{"assignment":[2,2],"sample":2}
"#,
        ),
        (
            BenVariant::TwoDelta,
            r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[1,2,1,2],"sample":2}
{"assignment":[2,2,1,1],"sample":3}
"#,
        ),
    ];

    for (variant, jsonl) in cases {
        let mut ben = Vec::new();
        encode_jsonl_to_ben(BufReader::new(jsonl.as_bytes()), &mut ben, variant).unwrap();

        let mut xz = Vec::new();
        encode_ben_to_xben(
            BufReader::new(ben.as_slice()),
            &mut xz,
            Some(1),
            Some(0),
            None,
        )
        .expect("ben->xben failed");

        let mut ben_back = Vec::new();
        decode_xben_to_ben(BufReader::new(xz.as_slice()), &mut ben_back).unwrap();

        let mut jsonl_back = Vec::new();
        decode_ben_to_jsonl(ben_back.as_slice(), &mut jsonl_back).unwrap();
        assert_eq!(jsonl_back, jsonl.as_bytes());
    }
}

#[test]
fn xz_mt_params_are_capped_and_safe() {
    use std::io::BufReader;
    let jsonl = r#"{"assignment":[1,2,3],"sample":1}"#.to_string() + "\n";
    let mut xz = Vec::new();
    encode_jsonl_to_xben(
        BufReader::new(jsonl.as_bytes()),
        &mut xz,
        BenVariant::Standard,
        Some(10_000),
        Some(42),
        None,
    )
    .unwrap();
    let mut ben = Vec::new();
    decode_xben_to_ben(BufReader::new(xz.as_slice()), &mut ben).unwrap();
    let mut out = Vec::new();
    decode_ben_to_jsonl(ben.as_slice(), &mut out).unwrap();
    assert_eq!(out, jsonl.as_bytes());
}

#[test]
fn ben_encoder_write_assignment_path_roundtrips() {
    let mut ben = Vec::new();
    {
        let mut enc = AssignmentWriter::new(&mut ben, BenVariant::Standard).unwrap();
        enc.write_assignment(vec![9u16, 9, 2, 2, 2]).unwrap();
        enc.finish().unwrap();
    }

    let mut out = Vec::new();
    decode_ben_to_jsonl(ben.as_slice(), &mut out).unwrap();
    assert_eq!(
        out,
        br#"{"assignment":[9,9,2,2,2],"sample":1}
"#
    );
}

#[test]
fn ben_decoder_new_reports_short_header_as_io_error() {
    let err = AssignmentReader::new([1u8, 2, 3].as_slice()).err().unwrap();
    match err {
        DecoderInitError::Io(e) => assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof),
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn ben_decoder_write_all_jsonl_propagates_frame_errors() {
    let mut malformed = b"STANDARD BEN FILE".to_vec();
    malformed.extend_from_slice(&[3]); // start of a frame, but truncated

    let mut decoder = AssignmentReader::new(malformed.as_slice()).unwrap();
    let err = decoder.write_all_jsonl(Vec::new()).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::UnexpectedEof);
}

#[test]
fn ben_decoder_count_samples_propagates_frame_errors() {
    let mut malformed = b"STANDARD BEN FILE".to_vec();
    malformed.extend_from_slice(&[3]);

    let err = AssignmentReader::new(malformed.as_slice())
        .unwrap()
        .count_samples()
        .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::UnexpectedEof);
}

#[test]
fn xben_frame_decoder_new_and_truncated_iteration_paths() {
    let jsonl = r#"{"assignment":[1,1,1],"sample":1}
{"assignment":[2,2],"sample":2}
"#;
    let mut xz = Vec::new();
    encode_jsonl_to_xben(
        BufReader::new(jsonl.as_bytes()),
        &mut xz,
        BenVariant::Standard,
        Some(1),
        Some(0),
        None,
    )
    .unwrap();

    let mut frames =
        binary_ensemble::io::reader::XZAssignmentFrameReader::new(xz.as_slice()).unwrap();
    assert!(frames.next().unwrap().is_ok());

    let trimmed = &xz[..xz.len() - 1];
    let mut frames = binary_ensemble::io::reader::XZAssignmentFrameReader::new(trimmed).unwrap();
    loop {
        match frames.next() {
            Some(Err(e)) => {
                assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof);
                break;
            }
            Some(Ok(_)) => continue,
            None => panic!("expected truncated-frame error"),
        }
    }
}

#[test]
fn xben_encoder_write_ben_file_without_banner_path_roundtrips() {
    let mut payload_only = Vec::new();
    {
        let mut enc = AssignmentWriter::new(&mut payload_only, BenVariant::Standard).unwrap();
        enc.write_assignment(vec![5u16, 5, 7]).unwrap();
        enc.finish().unwrap();
    }
    let payload_only = payload_only[17..].to_vec();

    let mut xz = Vec::new();
    {
        let mt = xz2::stream::MtStreamBuilder::new()
            .threads(1)
            .preset(0)
            .block_size(0)
            .encoder()
            .unwrap();
        let encoder = xz2::write::XzEncoder::new_stream(&mut xz, mt);
        let mut xben =
            binary_ensemble::io::writer::XZAssignmentWriter::new(encoder, BenVariant::Standard)
                .unwrap();
        xben.write_ben_file(BufReader::new(payload_only.as_slice()))
            .unwrap();
    }

    let mut ben = Vec::new();
    decode_xben_to_ben(BufReader::new(xz.as_slice()), &mut ben).unwrap();

    let mut out = Vec::new();
    decode_ben_to_jsonl(ben.as_slice(), &mut out).unwrap();
    assert_eq!(
        out,
        br#"{"assignment":[5,5,7],"sample":1}
"#
    );
}

struct FailAfterN {
    data: Vec<u8>,
    pos: usize,
    fail_at: usize,
}

impl std::io::Read for FailAfterN {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.fail_at {
            return Err(std::io::Error::other("boom"));
        }
        if self.pos >= self.data.len() {
            return Ok(0);
        }
        let n = buf
            .len()
            .min(self.data.len() - self.pos)
            .min(self.fail_at - self.pos);
        buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

#[test]
fn ben_decoder_frame_read_error_paths() {
    let banner = b"STANDARD BEN FILE".to_vec();

    let err = AssignmentReader::new(FailAfterN {
        data: [banner.clone(), vec![3]].concat(),
        pos: 0,
        fail_at: 18,
    })
    .unwrap()
    .next()
    .unwrap()
    .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::Other);

    let err = AssignmentReader::new(FailAfterN {
        data: [banner.clone(), vec![3, 3, 0]].concat(),
        pos: 0,
        fail_at: 20,
    })
    .unwrap()
    .next()
    .unwrap()
    .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::Other);

    let err = AssignmentReader::new(FailAfterN {
        data: [banner.clone(), vec![3, 3, 0, 0, 0, 1]].concat(),
        pos: 0,
        fail_at: 23,
    })
    .unwrap()
    .next()
    .unwrap()
    .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::Other);
}

#[test]
fn ben_decoder_mkv_count_read_error_path() {
    let mut ben = Vec::new();
    encode_jsonl_to_ben(
        BufReader::new(br#"{"assignment":[1,1],"sample":1}"#.as_slice()),
        &mut ben,
        BenVariant::MkvChain,
    )
    .unwrap();
    let truncated = ben[..ben.len() - 1].to_vec();
    let err = AssignmentReader::new(truncated.as_slice())
        .unwrap()
        .next()
        .unwrap()
        .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::UnexpectedEof);
}

#[test]
fn subsample_frame_decoder_propagates_inner_and_decode_errors() {
    let mut inner = SubsampleFrameDecoder::by_indices(
        vec![Err(std::io::Error::other("boom"))].into_iter(),
        vec![1],
    );
    let err = inner.next().unwrap().unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::Other);

    let mut malformed = SubsampleFrameDecoder::by_indices(
        vec![Ok((
            DecodeFrame::XBen(vec![1, 2, 3], BenVariant::Standard),
            1,
        ))]
        .into_iter(),
        vec![1],
    );
    let err = malformed.next().unwrap().unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::UnexpectedEof);
}

fn unique_temp_path(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("binary-ensemble-{name}-{nonce}.tmp"))
}

#[test]
fn decoder_init_error_display_source_and_conversion_paths() {
    let io_error = DecoderInitError::from(std::io::Error::other("boom"));
    assert_eq!(io_error.to_string(), "IO error: boom");
    assert!(io_error.source().is_some());

    let xz_bytes = {
        let mut buf = Vec::new();
        xz_compress(
            BufReader::new(b"hello".as_slice()),
            &mut buf,
            Some(1),
            Some(0),
        )
        .unwrap();
        buf
    };
    let xz_header = xz_bytes[..17].to_vec();
    let invalid = DecoderInitError::InvalidFileFormat(xz_header.clone());
    let msg = invalid.to_string();
    assert!(msg.contains("Compressed header detected"));
    assert!(msg.contains("decode_xben_to_ben"));
    assert!(invalid.source().is_none());

    let generic = DecoderInitError::InvalidFileFormat(b"not a ben header!!".to_vec());
    assert!(generic.to_string().contains("utf8-lossy"));

    let io_err: std::io::Error = DecoderInitError::InvalidFileFormat(xz_header).into();
    assert_eq!(io_err.kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn ben_decoder_and_xben_decoder_count_samples() {
    let jsonl = r#"{"assignment":[1,1],"sample":1}
{"assignment":[1,1],"sample":2}
{"assignment":[2,2],"sample":3}
"#;

    let mut ben = Vec::new();
    encode_jsonl_to_ben(
        BufReader::new(jsonl.as_bytes()),
        &mut ben,
        BenVariant::MkvChain,
    )
    .unwrap();
    assert_eq!(
        AssignmentReader::new(ben.as_slice())
            .unwrap()
            .count_samples()
            .unwrap(),
        3
    );

    let mut xben = Vec::new();
    encode_jsonl_to_xben(
        BufReader::new(jsonl.as_bytes()),
        &mut xben,
        BenVariant::MkvChain,
        Some(1),
        Some(0),
        None,
    )
    .unwrap();
    assert_eq!(
        XZAssignmentReader::new(xben.as_slice())
            .unwrap()
            .count_samples()
            .unwrap(),
        3
    );

    let twodelta_jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[1,2,2,1],"sample":2}
{"assignment":[1,2,2,1],"sample":3}
"#;
    let mut twodelta_xben = Vec::new();
    encode_jsonl_to_xben(
        BufReader::new(twodelta_jsonl.as_bytes()),
        &mut twodelta_xben,
        BenVariant::TwoDelta,
        Some(1),
        Some(0),
        None,
    )
    .unwrap();
    assert_eq!(
        XZAssignmentReader::new(twodelta_xben.as_slice())
            .unwrap()
            .count_samples()
            .unwrap(),
        3
    );
}

#[test]
fn build_frame_iter_and_count_samples_from_file_cover_public_file_api() {
    let jsonl = r#"{"assignment":[1,1],"sample":1}
{"assignment":[2,2],"sample":2}
{"assignment":[2,2],"sample":3}
"#;

    let mut ben = Vec::new();
    encode_jsonl_to_ben(
        BufReader::new(jsonl.as_bytes()),
        &mut ben,
        BenVariant::MkvChain,
    )
    .unwrap();
    let ben_path = unique_temp_path("sample.ben");
    fs::write(&ben_path, &ben).unwrap();

    let mut xben = Vec::new();
    encode_jsonl_to_xben(
        BufReader::new(jsonl.as_bytes()),
        &mut xben,
        BenVariant::MkvChain,
        Some(1),
        Some(0),
        None,
    )
    .unwrap();
    let xben_path = unique_temp_path("sample.xben");
    fs::write(&xben_path, &xben).unwrap();

    let ben_iter = build_frame_iter(&ben_path, "ben").unwrap();
    assert_eq!(collect_frames(ben_iter).unwrap().len(), 2);

    let xben_iter = build_frame_iter(&xben_path, "xben").unwrap();
    assert_eq!(collect_frames(xben_iter).unwrap().len(), 2);

    assert_eq!(count_samples_from_file(&ben_path, "ben").unwrap(), 3);
    assert_eq!(count_samples_from_file(&xben_path, "xben").unwrap(), 3);

    let err = build_frame_iter(&ben_path, "wat").err().unwrap();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);

    fs::remove_file(ben_path).unwrap();
    fs::remove_file(xben_path).unwrap();
}

#[test]
fn ben_decoder_subsample_helpers_work_on_public_api() {
    let jsonl = r#"{"assignment":[1],"sample":1}
{"assignment":[2],"sample":2}
{"assignment":[3],"sample":3}
{"assignment":[4],"sample":4}
"#;

    let mut ben = Vec::new();
    encode_jsonl_to_ben(
        BufReader::new(jsonl.as_bytes()),
        &mut ben,
        BenVariant::MkvChain,
    )
    .unwrap();

    let mut by_indices = AssignmentReader::new(ben.as_slice())
        .unwrap()
        .into_subsample_by_indices(vec![4, 1, 1, 3]);
    let picked = collect_records(&mut by_indices).unwrap();
    assert_eq!(
        picked.into_iter().map(|(a, _)| a[0]).collect::<Vec<u16>>(),
        vec![1, 3, 4]
    );

    let mut by_range = AssignmentReader::new(ben.as_slice())
        .unwrap()
        .into_subsample_by_range(2, 3);
    let picked = collect_records(&mut by_range).unwrap();
    assert_eq!(
        picked.into_iter().map(|(a, _)| a[0]).collect::<Vec<u16>>(),
        vec![2, 3]
    );

    let mut every = AssignmentReader::new(ben.as_slice())
        .unwrap()
        .into_subsample_every(2, 2);
    let picked = collect_records(&mut every).unwrap();
    assert_eq!(
        picked.into_iter().map(|(a, _)| a[0]).collect::<Vec<u16>>(),
        vec![2, 4]
    );
}

#[test]
fn twodelta_roundtrips_and_counts_repeated_frames() {
    let assignments = vec![
        vec![1u16, 1, 2, 2, 3, 3],
        vec![1u16, 1, 2, 2, 3, 3],
        vec![1u16, 2, 2, 1, 3, 3],
        vec![1u16, 2, 2, 1, 3, 3],
        vec![2u16, 2, 1, 1, 3, 3],
    ];

    let mut ben = Vec::new();
    {
        let mut encoder = AssignmentWriter::new(&mut ben, BenVariant::TwoDelta).unwrap();
        for assignment in &assignments {
            encoder.write_assignment(assignment.clone()).unwrap();
        }
        encoder.finish().unwrap();
    }

    let records = collect_records(AssignmentReader::new(ben.as_slice()).unwrap()).unwrap();
    assert_eq!(
        records,
        vec![
            (assignments[0].clone(), 2),
            (assignments[2].clone(), 2),
            (assignments[4].clone(), 1),
        ]
    );

    let mut jsonl = Vec::new();
    decode_ben_to_jsonl(ben.as_slice(), &mut jsonl).unwrap();
    assert_eq!(jsonl, jsonl_from_assignments(&assignments));

    let frames = AssignmentReader::new(ben.as_slice()).unwrap().into_frames();
    assert_eq!(
        collect_frames(frames.map(|res| res.map(|(f, cnt)| (DecodeFrame::Ben(f), cnt))))
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn twodelta_first_frame_carries_repeat_trailer() {
    let first = vec![1u16, 1, 2, 2, 3, 3];
    let second = vec![1u16, 2, 2, 1, 3, 3];

    let mut ben = Vec::new();
    {
        let mut encoder = AssignmentWriter::new(&mut ben, BenVariant::TwoDelta).unwrap();
        encoder.write_assignment(first.clone()).unwrap();
        encoder.write_assignment(first.clone()).unwrap();
        encoder.write_assignment(second).unwrap();
        encoder.finish().unwrap();
    }

    let expected_first = BenEncodeFrame::from_assignment(&first, None);
    assert_eq!(&ben[..17], b"TWODELTA BEN FILE");
    assert_eq!(
        &ben[17..17 + expected_first.as_slice().len()],
        expected_first.as_slice()
    );
    let count_offset = 17 + expected_first.as_slice().len();
    assert_eq!(
        u16::from_be_bytes([ben[count_offset], ben[count_offset + 1]]),
        2
    );
}

#[test]
fn twodelta_rejects_non_pair_transition() {
    let mut ben = Vec::new();
    let mut encoder = AssignmentWriter::new(&mut ben, BenVariant::TwoDelta).unwrap();
    encoder.write_assignment(vec![1u16, 1, 2, 2]).unwrap();
    encoder.write_assignment(vec![1u16, 3, 2, 4]).unwrap();
    let err = encoder.finish().err().unwrap();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn twodelta_write_json_value_rejects_non_pair_transition() {
    let mut ben = Vec::new();
    let mut encoder = AssignmentWriter::new(&mut ben, BenVariant::TwoDelta).unwrap();
    encoder
        .write_json_value(json!({"assignment": [1u16, 1, 2, 2]}))
        .unwrap();
    encoder
        .write_json_value(json!({"assignment": [1u16, 3, 2, 4]}))
        .unwrap();
    let err = encoder.finish().err().unwrap();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn twodelta_supports_frame_iteration_counting_and_sample_extraction() {
    let assignments = vec![
        vec![1u16, 1, 2, 2, 3, 3],
        vec![1u16, 1, 2, 2, 3, 3],
        vec![1u16, 2, 2, 1, 3, 3],
        vec![2u16, 2, 1, 1, 3, 3],
    ];

    let mut ben = Vec::new();
    let jsonl = jsonl_from_assignments(&assignments);
    encode_jsonl_to_ben(
        BufReader::new(jsonl.as_slice()),
        &mut ben,
        BenVariant::TwoDelta,
    )
    .unwrap();

    assert_eq!(
        AssignmentReader::new(ben.as_slice())
            .unwrap()
            .count_samples()
            .unwrap(),
        4
    );

    let frames: Vec<_> = AssignmentReader::new(ben.as_slice())
        .unwrap()
        .into_frames()
        .collect::<std::io::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(frames.len(), 3);
    assert_eq!(frames[0].1, 2);
    assert_eq!(frames[1].1, 1);
    assert_eq!(frames[2].1, 1);

    let picked = extract_assignment_ben(ben.as_slice(), 3).unwrap();
    assert_eq!(picked, assignments[2]);

    let ben_path = unique_temp_path("twodelta_sample.ben");
    fs::write(&ben_path, &ben).unwrap();
    assert_eq!(count_samples_from_file(&ben_path, "ben").unwrap(), 4);
    fs::remove_file(ben_path).unwrap();
}
