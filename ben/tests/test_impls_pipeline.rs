#![allow(clippy::needless_collect)]

use ben::decode::{
    decode_ben_line, decode_ben_to_jsonl, decode_xben_to_ben, decode_xben_to_jsonl, xz_decompress,
    BenDecoder, DecoderInitError, XBenDecoder,
};
use ben::encode::{
    encode_ben_to_xben, encode_ben_vec_from_rle, encode_jsonl_to_ben, encode_jsonl_to_xben,
    xz_compress, BenEncoder,
};
use ben::BenVariant;

use proptest::prelude::*;
use serde_json::json;
use std::io::{BufReader, Cursor, Write};

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

// Random (small) thread count and compression level for MT encoder.
fn strat_threads_levels() -> impl Strategy<Value = (u32, u32)> {
    (1u32..=4, 0u32..=9)
}

// ---------- Tests ----------

proptest! {
    // JSONL -> BEN(Standard) -> JSONL round-trip via BenEncoder/BenDecoder entry points.
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

    // Iterator surface: XBenDecoder -> records matches direct JSONL
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
        ).unwrap();

        let mut dec = XBenDecoder::new(xben.as_slice()).unwrap();
        let recs = collect_records(&mut dec).unwrap();

        let iter_jsonl = jsonl_from_records(&recs, 0);

        // Also decode via the library jsonl_decode_xben and compare.
        let mut direct = Vec::new();
        decode_xben_to_jsonl(BufReader::new(xben.as_slice()), &mut direct).unwrap();

        prop_assert_eq!(iter_jsonl, direct);
    }

    // Iterator surface: BenDecoder over BEN produced by BenEncoder.
    #[test]
    fn fuzz_bendecoder_iterator_matches_jsonl(seq in strat_assignment_seq()) {
        let jsonl = jsonl_from_assignments(&seq);

        // Build BEN(Standard)
        let mut ben = Vec::new();
        encode_jsonl_to_ben(BufReader::new(jsonl.as_slice()), &mut ben, BenVariant::Standard).unwrap();

        // Iterate BenDecoder
        let mut dec = BenDecoder::new(ben.as_slice()).unwrap();
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
        ).unwrap();

        // Choose some indices to keep (1-based). We derive from seq length.
        let n = seq.len().max(1);
        let mut want: Vec<usize> = (1..=n).step_by(3).collect(); // 1,4,7,…
        if want.is_empty() { want.push(1); }

        let xb = XBenDecoder::new(xben.as_slice()).unwrap();
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
        ).unwrap();

        let n = seq.len();
        let mut truth: Vec<Vec<u16>> = Vec::new();
        for i in 1..=n {
            if i >= offset && (i - offset) % step == 0 {
                truth.push(seq[i-1].clone());
            }
        }

        let xb = XBenDecoder::new(xben.as_slice()).unwrap();
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
        ).unwrap();

        let n = seq.len();
        let s = start.min(n.max(1));
        let e = (s + len).min(n);

        let truth: Vec<Vec<u16>> = (s..=e).map(|i| seq[i-1].clone()).collect();

        let xb = XBenDecoder::new(xben.as_slice()).unwrap();
        let mut sub = xb.into_subsample_by_range(s, e);
        let recs = collect_records(&mut sub).unwrap();

        let mut picked: Vec<Vec<u16>> = Vec::new();
        for (a, c) in recs {
            for _ in 0..c { picked.push(a.clone()); }
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

    let err = BenDecoder::new(Cursor::new(bogus))
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

    let err = XBenDecoder::new(xz.as_slice())
        .err()
        .expect("expeced InvalidFileFormat error");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
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
    )
    .unwrap();

    // Keep every 1 starting at offset=2 -> only second sample.
    let xb = XBenDecoder::new(xben.as_slice()).unwrap();
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
        let mut enc = BenEncoder::new(&mut ben_vec, BenVariant::MkvChain);
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
    let err = BenDecoder::new(xz.as_slice())
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
    let err = XBenDecoder::new(wrong.as_slice())
        .err()
        .expect("expected invalid data");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
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
    )
    .unwrap();

    // Trim the last byte to force partial frame after decompress
    let trimmed = &xz[..xz.len() - 1];
    // Iterating should surface UnexpectedEof (partial frame)
    let mut it = XBenDecoder::new(trimmed).unwrap();
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
    let ben = encode_ben_vec_from_rle(rle.clone());
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
    )
    .unwrap();
    let xb = XBenDecoder::new(xz.as_slice()).unwrap();

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
    // Build a BEN(Standard)
    let jsonl = r#"{"assignment":[1,1],"sample":1}
{"assignment":[2,2],"sample":2}
"#;
    let mut ben = Vec::new();
    encode_jsonl_to_ben(
        BufReader::new(jsonl.as_bytes()),
        &mut ben,
        BenVariant::Standard,
    )
    .unwrap();

    // Now convert BEN -> XBEN
    let mut xz = Vec::new();
    encode_ben_to_xben(BufReader::new(ben.as_slice()), &mut xz, Some(1), Some(0))
        .expect("ben->xben failed");

    // Decode back
    let mut ben_back = Vec::new();
    decode_xben_to_ben(BufReader::new(xz.as_slice()), &mut ben_back).unwrap();

    // Then to JSONL
    let mut jsonl_back = Vec::new();
    decode_ben_to_jsonl(ben_back.as_slice(), &mut jsonl_back).unwrap();
    assert_eq!(jsonl_back, jsonl.as_bytes());
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
    )
    .unwrap();
    let mut ben = Vec::new();
    decode_xben_to_ben(BufReader::new(xz.as_slice()), &mut ben).unwrap();
    let mut out = Vec::new();
    decode_ben_to_jsonl(ben.as_slice(), &mut out).unwrap();
    assert_eq!(out, jsonl.as_bytes());
}
