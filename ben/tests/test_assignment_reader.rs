//! Rigorous tests for `BenStreamReader` with the MkvChain and TwoDelta BEN variants.
//!
//! Standard-variant tests already exist in `test_coverage.rs`. This file adds equivalent depth for
//! the two more complex variants. The helpers intentionally mirror those in `test_coverage.rs` so
//! that the two suites are easy to compare.

use binary_ensemble::codec::decode::decode_ben_to_jsonl;
use binary_ensemble::codec::encode::encode_jsonl_to_ben;
use binary_ensemble::format::banners::{MKVCHAIN_BEN_BANNER, TWODELTA_BEN_BANNER};
use binary_ensemble::io::reader::{BenStreamFrameReader, BenStreamReader};
use binary_ensemble::io::writer::BenStreamWriter;
use binary_ensemble::BenVariant;

use std::io::{self, Cursor};

mod common;
use common::jsonl_from_assignments;

// ────────────────────────────────────────────────────────────────────────────── Shared helpers
// ──────────────────────────────────────────────────────────────────────────────

fn encode_ben(assignments: &[Vec<u16>], variant: BenVariant) -> Vec<u8> {
    let jsonl = jsonl_from_assignments(assignments);
    let mut ben = Vec::new();
    encode_jsonl_to_ben(jsonl.as_slice(), &mut ben, variant).unwrap();
    ben
}

/// Expand all repetitions by calling `for_each_assignment`.
fn expand_assignments(ben: &[u8]) -> Vec<Vec<u16>> {
    let mut decoder = BenStreamReader::from_ben(ben).unwrap().silent(true);
    let mut out = Vec::new();
    decoder
        .for_each_assignment(|a, count| {
            for _ in 0..count {
                out.push(a.to_vec());
            }
            Ok(true)
        })
        .unwrap();
    out
}

// ────────────────────────────────────────────────────────────────────────────── MkvChain variant
// ──────────────────────────────────────────────────────────────────────────────

mod mkvchain {
    use super::*;

    // ─── banner and initialisation ────────────────────────────────────────────

    #[test]
    fn banner_is_correct() {
        let ben = encode_ben(&[vec![1u16, 2, 3]], BenVariant::MkvChain);
        assert!(ben.starts_with(MKVCHAIN_BEN_BANNER));
    }

    #[test]
    fn variant_accessor_returns_mkvchain() {
        let ben = encode_ben(&[vec![1u16, 2]], BenVariant::MkvChain);
        let decoder = BenStreamReader::from_ben(ben.as_slice()).unwrap();
        assert_eq!(decoder.variant(), BenVariant::MkvChain);
    }

    #[test]
    fn empty_payload_yields_nothing() {
        let ben = MKVCHAIN_BEN_BANNER.to_vec();
        let decoder = BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .silent(true);
        let frames: Vec<_> = decoder.collect::<io::Result<Vec<_>>>().unwrap();
        assert!(frames.is_empty());
    }

    // ─── iterator / round-trips ───────────────────────────────────────────────

    #[test]
    fn single_assignment_round_trip() {
        let assignment = vec![3u16, 3, 1, 2, 2, 1];
        let ben = encode_ben(std::slice::from_ref(&assignment), BenVariant::MkvChain);

        let mut decoder = BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .silent(true);
        let (decoded, count) = decoder.next().unwrap().unwrap();
        assert_eq!(count, 1);
        assert_eq!(decoded, assignment);
        assert!(decoder.next().is_none());
    }

    #[test]
    fn multiple_distinct_assignments_each_have_count_one() {
        let assignments = vec![vec![1u16, 2, 3], vec![3u16, 2, 1], vec![2u16, 1, 3]];
        let ben = encode_ben(&assignments, BenVariant::MkvChain);

        let mut decoder = BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .silent(true);
        for expected in &assignments {
            let (decoded, count) = decoder.next().unwrap().unwrap();
            assert_eq!(count, 1, "distinct assignment should have count=1");
            assert_eq!(&decoded, expected);
        }
        assert!(decoder.next().is_none());
    }

    #[test]
    fn identical_run_compressed_into_single_frame_with_correct_count() {
        // 5 identical assignments → one frame with count = 5.
        let assignment = vec![2u16, 2, 1, 1];
        let assignments = vec![assignment.clone(); 5];
        let ben = encode_ben(&assignments, BenVariant::MkvChain);

        let mut decoder = BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .silent(true);
        let (decoded, count) = decoder.next().unwrap().unwrap();
        assert_eq!(count, 5, "expected compressed count=5, got {count}");
        assert_eq!(decoded, assignment);
        assert!(decoder.next().is_none());
    }

    #[test]
    fn mixed_runs_yield_correct_frame_counts() {
        // [A×3, B×2, C×1] → three frames with counts [3, 2, 1].
        let a = vec![1u16, 1, 1];
        let b = vec![2u16, 2, 2];
        let c = vec![3u16, 3, 3];
        let assignments = [
            a.clone(),
            a.clone(),
            a.clone(),
            b.clone(),
            b.clone(),
            c.clone(),
        ];
        let ben = encode_ben(&assignments, BenVariant::MkvChain);

        let mut decoder = BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .silent(true);

        let (d1, c1) = decoder.next().unwrap().unwrap();
        assert_eq!(c1, 3);
        assert_eq!(d1, a);

        let (d2, c2) = decoder.next().unwrap().unwrap();
        assert_eq!(c2, 2);
        assert_eq!(d2, b);

        let (d3, c3) = decoder.next().unwrap().unwrap();
        assert_eq!(c3, 1);
        assert_eq!(d3, c);

        assert!(decoder.next().is_none());
    }

    #[test]
    fn alternating_assignments_each_have_count_one() {
        // A,B,A,B,A — no adjacent pair is identical, so every frame has count=1.
        let a = vec![1u16, 2, 3];
        let b = vec![3u16, 2, 1];
        let assignments: Vec<Vec<u16>> = (0..5)
            .map(|i| if i % 2 == 0 { a.clone() } else { b.clone() })
            .collect();
        let ben = encode_ben(&assignments, BenVariant::MkvChain);

        let records: Vec<_> = BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .silent(true)
            .collect::<io::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(records.len(), 5);
        for (_, count) in &records {
            assert_eq!(*count, 1, "alternating frames should each have count=1");
        }
    }

    #[test]
    fn iterator_values_match_original_assignments() {
        let assignments: Vec<Vec<u16>> = (0u16..8).map(|i| vec![i, i + 1]).collect();
        let ben = encode_ben(&assignments, BenVariant::MkvChain);

        let decoded: Vec<Vec<u16>> = BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .silent(true)
            .map(|r| r.unwrap().0)
            .collect();
        assert_eq!(decoded, assignments);
    }

    // ─── count_samples ────────────────────────────────────────────────────────

    #[test]
    fn count_samples_with_no_repetitions() {
        let assignments = vec![vec![1u16, 2], vec![3u16, 4], vec![5u16, 6]];
        let ben = encode_ben(&assignments, BenVariant::MkvChain);
        assert_eq!(
            BenStreamReader::from_ben(ben.as_slice())
                .unwrap()
                .count_samples()
                .unwrap(),
            3
        );
    }

    #[test]
    fn count_samples_expands_repetitions() {
        // 3×A + 2×B = 5 total samples from 2 frames.
        let a = vec![1u16, 0];
        let b = vec![0u16, 1];
        let assignments: Vec<_> = (0..3)
            .map(|_| a.clone())
            .chain((0..2).map(|_| b.clone()))
            .collect();
        let ben = encode_ben(&assignments, BenVariant::MkvChain);
        assert_eq!(
            BenStreamReader::from_ben(ben.as_slice())
                .unwrap()
                .count_samples()
                .unwrap(),
            5
        );
    }

    #[test]
    fn count_samples_empty_stream() {
        let ben = MKVCHAIN_BEN_BANNER.to_vec();
        assert_eq!(
            BenStreamReader::from_ben(ben.as_slice())
                .unwrap()
                .count_samples()
                .unwrap(),
            0
        );
    }

    // ─── write_all_jsonl ──────────────────────────────────────────────────────

    #[test]
    fn write_all_jsonl_expands_repetitions() {
        // A single frame with count=3 must produce 3 separate JSONL lines.
        let assignment = vec![5u16, 5, 5];
        let ben = encode_ben(&vec![assignment.clone(); 3], BenVariant::MkvChain);

        let mut out = Vec::new();
        BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .write_all_jsonl(&mut out)
            .unwrap();
        let s = String::from_utf8(out).unwrap();

        assert_eq!(s.lines().count(), 3, "expected 3 JSONL lines for 3 samples");
        for line in s.lines() {
            assert!(line.contains("\"assignment\":[5,5,5]"), "bad line: {line}");
        }
    }

    #[test]
    fn write_all_jsonl_sample_numbers_are_sequential() {
        // Sample numbers must be 1, 2, 3 even when originating from one compressed frame.
        let assignment = vec![1u16, 2, 3];
        let ben = encode_ben(&vec![assignment; 3], BenVariant::MkvChain);

        let mut out = Vec::new();
        BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .write_all_jsonl(&mut out)
            .unwrap();
        let s = String::from_utf8(out).unwrap();

        let parsed: Vec<serde_json::Value> = s
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        for (i, v) in parsed.iter().enumerate() {
            assert_eq!(
                v["sample"],
                i as u64 + 1,
                "sample number mismatch at position {i}"
            );
        }
    }

    #[test]
    fn write_all_jsonl_mixed_runs_are_correct() {
        let a = vec![10u16, 20];
        let b = vec![30u16, 40];
        // A, A, B → 3 lines, first two are [10,20], third is [30,40].
        let ben = encode_ben(&[a.clone(), a.clone(), b.clone()], BenVariant::MkvChain);

        let mut out = Vec::new();
        BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .write_all_jsonl(&mut out)
            .unwrap();
        let s = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = s.lines().collect();

        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("[10,20]"), "line 0: {}", lines[0]);
        assert!(lines[1].contains("[10,20]"), "line 1: {}", lines[1]);
        assert!(lines[2].contains("[30,40]"), "line 2: {}", lines[2]);
    }

    #[test]
    fn write_all_jsonl_matches_codec_decode() {
        let assignments: Vec<Vec<u16>> = vec![
            vec![1u16, 2, 1, 2],
            vec![1u16, 2, 1, 2],
            vec![2u16, 1, 2, 1],
        ];
        let ben = encode_ben(&assignments, BenVariant::MkvChain);

        let mut via_reader = Vec::new();
        BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .write_all_jsonl(&mut via_reader)
            .unwrap();

        let mut via_codec = Vec::new();
        decode_ben_to_jsonl(ben.as_slice(), &mut via_codec).unwrap();

        assert_eq!(via_reader, via_codec);
    }

    // ─── for_each_assignment ─────────────────────────────────────────────────

    #[test]
    fn for_each_receives_correct_count() {
        let assignment = vec![7u16, 8, 9];
        let ben = encode_ben(&vec![assignment.clone(); 4], BenVariant::MkvChain);

        let mut seen_count = 0u16;
        BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .silent(true)
            .for_each_assignment(|a, count| {
                assert_eq!(a, assignment.as_slice());
                seen_count = count;
                Ok(true)
            })
            .unwrap();
        assert_eq!(seen_count, 4);
    }

    #[test]
    fn for_each_mixed_runs_delivers_correct_pairs() {
        let a = vec![1u16, 1];
        let b = vec![2u16, 2];
        let c = vec![3u16, 3];
        let assignments = [
            a.clone(),
            a.clone(),
            a.clone(),
            b.clone(),
            b.clone(),
            c.clone(),
        ];
        let ben = encode_ben(&assignments, BenVariant::MkvChain);

        let mut frames: Vec<(Vec<u16>, u16)> = Vec::new();
        BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .silent(true)
            .for_each_assignment(|assignment, count| {
                frames.push((assignment.to_vec(), count));
                Ok(true)
            })
            .unwrap();

        assert_eq!(frames, vec![(a, 3), (b, 2), (c, 1)]);
    }

    #[test]
    fn for_each_early_stop_terminates_after_first_frame() {
        let a = vec![1u16, 1];
        let b = vec![2u16, 2];
        let c = vec![3u16, 3];
        let assignments = [a.clone(), a.clone(), b.clone(), c.clone()];
        let ben = encode_ben(&assignments, BenVariant::MkvChain);

        let mut seen = Vec::new();
        BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .silent(true)
            .for_each_assignment(|assignment, _count| {
                seen.push(assignment.to_vec());
                Ok(false) // stop immediately after first frame
            })
            .unwrap();
        assert_eq!(seen, vec![a]);
    }

    // ─── into_frames (BenStreamFrameReader) ──────────────────────────────────

    #[test]
    fn frame_reader_yields_count_in_tuple() {
        // A run of 3 identical assignments → one frame tuple (frame, 3).
        let assignment = vec![5u16, 6, 7];
        let ben = encode_ben(&vec![assignment; 3], BenVariant::MkvChain);

        let frames: Vec<_> = BenStreamFrameReader::from_ben(Cursor::new(ben))
            .unwrap()
            .collect::<io::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(frames.len(), 1, "expected one compressed frame");
        assert_eq!(frames[0].1, 3, "count should be 3");
    }

    #[test]
    fn frame_reader_mixed_runs_yield_correct_counts() {
        let a = vec![1u16, 0];
        let b = vec![0u16, 1];
        // A×2, B×1 → 2 frames with counts [2, 1].
        let ben = encode_ben(&[a.clone(), a.clone(), b.clone()], BenVariant::MkvChain);

        let frames: Vec<_> = BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .silent(true)
            .into_frames()
            .collect::<io::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].1, 2, "first frame count");
        assert_eq!(frames[1].1, 1, "second frame count");
    }

    #[test]
    fn frame_reader_bytes_decode_back_to_original_assignment() {
        let assignment = vec![3u16, 3, 1, 2];
        let ben = encode_ben(std::slice::from_ref(&assignment), BenVariant::MkvChain);

        let (frame, _count) = BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .silent(true)
            .into_frames()
            .next()
            .unwrap()
            .unwrap();

        let decoded = frame.expand_self_contained().unwrap();
        assert_eq!(decoded, assignment);
    }

    // ─── subsampling ──────────────────────────────────────────────────────────
    //
    // SubsampleFrameDecoder operates at the frame level: it returns one (assignment, count) tuple
    // per frame that contains any selected indices, where count is the number of selected indices
    // in that frame.

    #[test]
    fn subsample_by_indices_locates_correct_sample_in_run() {
        // A×5, B×5; index 3 is in the A run, index 6 is the first B.
        let a = vec![1u16; 4];
        let b = vec![2u16; 4];
        let assignments: Vec<_> = (0..5)
            .map(|_| a.clone())
            .chain((0..5).map(|_| b.clone()))
            .collect();
        let ben = encode_ben(&assignments, BenVariant::MkvChain);

        let selected: Vec<(Vec<u16>, u16)> = BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .silent(true)
            .into_subsample_by_indices(vec![3usize, 6])
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(selected, vec![(a, 1), (b, 1)]);
    }

    #[test]
    fn subsample_by_indices_multiple_in_same_frame_returns_count() {
        // A×5; indices 2 and 4 both fall in the single A frame → one result with count=2.
        let a = vec![1u16; 4];
        let ben = encode_ben(&vec![a.clone(); 5], BenVariant::MkvChain);

        let selected: Vec<(Vec<u16>, u16)> = BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .silent(true)
            .into_subsample_by_indices(vec![2usize, 4])
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(
            selected.len(),
            1,
            "two indices in same frame → one result tuple"
        );
        assert_eq!(selected[0].0, a);
        assert_eq!(selected[0].1, 2, "count should be 2");
    }

    #[test]
    fn subsample_by_range_spans_repeated_frames() {
        // A×3, B×3; range [2, 5] → A contributes samples 2,3 (count=2) and B contributes 4,5
        // (count=2).
        let a = vec![10u16; 3];
        let b = vec![20u16; 3];
        let assignments: Vec<_> = (0..3)
            .map(|_| a.clone())
            .chain((0..3).map(|_| b.clone()))
            .collect();
        let ben = encode_ben(&assignments, BenVariant::MkvChain);

        let selected: Vec<(Vec<u16>, u16)> = BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .silent(true)
            .into_subsample_by_range(2, 5)
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(selected.len(), 2, "two frames contribute to range [2,5]");
        assert_eq!(selected[0], (a, 2)); // samples 2,3 from A
        assert_eq!(selected[1], (b, 2)); // samples 4,5 from B
    }

    #[test]
    fn subsample_every_within_single_run() {
        // A×6; every 2nd from offset 1 → indices 1,3,5 all in the A frame → count=3.
        let a = vec![99u16; 2];
        let ben = encode_ben(&vec![a.clone(); 6], BenVariant::MkvChain);

        let selected: Vec<(Vec<u16>, u16)> = BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .silent(true)
            .into_subsample_every(2, 1)
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(
            selected.len(),
            1,
            "all selected indices in one frame → one result"
        );
        assert_eq!(selected[0].0, a);
        assert_eq!(selected[0].1, 3, "indices 1,3,5 selected → count=3");
    }

    #[test]
    fn subsample_every_across_two_runs() {
        // A×4, B×4; every 2nd from offset 2 → indices 2,4,6,8 → 2 from A, 2 from B.
        let a = vec![10u16; 2];
        let b = vec![20u16; 2];
        let assignments: Vec<_> = (0..4)
            .map(|_| a.clone())
            .chain((0..4).map(|_| b.clone()))
            .collect();
        let ben = encode_ben(&assignments, BenVariant::MkvChain);

        let selected: Vec<(Vec<u16>, u16)> = BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .silent(true)
            .into_subsample_every(2, 2)
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(selected, vec![(a, 2), (b, 2)]);
    }

    // ─── error paths ─────────────────────────────────────────────────────────

    #[test]
    fn truncated_count_field_errors_on_next() {
        // Drop the last byte of the MkvChain count (u16 BE) from the stream.
        let assignment = vec![1u16, 1];
        let ben = encode_ben(&[assignment], BenVariant::MkvChain);
        let truncated = &ben[..ben.len() - 1];
        let err = BenStreamReader::from_ben(truncated)
            .unwrap()
            .next()
            .unwrap()
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn truncated_mid_payload_errors_on_next() {
        let assignment = vec![1u16, 2, 3, 4, 5];
        let ben = encode_ben(&[assignment], BenVariant::MkvChain);
        let truncated = &ben[..ben.len() - 5];
        let err = BenStreamReader::from_ben(truncated)
            .unwrap()
            .next()
            .unwrap()
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn count_samples_propagates_truncation_error() {
        let assignment = vec![1u16, 2];
        let ben = encode_ben(&[assignment], BenVariant::MkvChain);
        let truncated = &ben[..ben.len() - 1];
        let err = BenStreamReader::from_ben(truncated)
            .unwrap()
            .count_samples()
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn write_all_jsonl_propagates_truncation_error() {
        let assignment = vec![1u16, 2];
        let ben = encode_ben(&[assignment], BenVariant::MkvChain);
        let truncated = &ben[..ben.len() - 1];
        let err = BenStreamReader::from_ben(truncated)
            .unwrap()
            .write_all_jsonl(io::sink())
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }
}

// ────────────────────────────────────────────────────────────────────────────── TwoDelta variant
// ──────────────────────────────────────────────────────────────────────────────

mod twodelta {
    use super::*;

    /// Encode via `BenStreamWriter` so we control the exact frame layout.
    fn encode_twodelta(assignments: &[Vec<u16>]) -> Vec<u8> {
        let mut ben = Vec::new();
        {
            let mut writer = BenStreamWriter::for_ben(&mut ben, BenVariant::TwoDelta).unwrap();
            for a in assignments {
                writer.write_assignment(a.clone()).unwrap();
            }
            writer.finish().unwrap();
        }
        ben
    }

    // ─── banner and initialisation ────────────────────────────────────────────

    #[test]
    fn banner_is_correct() {
        let ben = encode_twodelta(&[vec![1u16, 2, 3]]);
        assert!(ben.starts_with(TWODELTA_BEN_BANNER));
    }

    #[test]
    fn variant_accessor_returns_twodelta() {
        let ben = encode_twodelta(&[vec![1u16, 2]]);
        assert_eq!(
            BenStreamReader::from_ben(ben.as_slice()).unwrap().variant(),
            BenVariant::TwoDelta
        );
    }

    // ─── round-trips ──────────────────────────────────────────────────────────

    #[test]
    fn single_anchor_frame_round_trip() {
        // A stream with only one assignment contains just the anchor frame.
        let assignment = vec![1u16, 1, 2, 2, 3, 3];
        let ben = encode_twodelta(std::slice::from_ref(&assignment));
        assert_eq!(expand_assignments(&ben), vec![assignment]);
    }

    #[test]
    fn anchor_then_single_delta_round_trip() {
        let anchor = vec![1u16, 1, 2, 2];
        let next = vec![2u16, 2, 1, 1]; // all 1s↔2s swapped
        let input = vec![anchor.clone(), next.clone()];
        let ben = encode_twodelta(&input);
        assert_eq!(expand_assignments(&ben), input);
    }

    #[test]
    fn multiple_deltas_round_trip() {
        // a→b→a→b: two alternating pair-swap assignments.
        let a = vec![1u16, 1, 2, 2, 3, 3];
        let b = vec![2u16, 2, 1, 1, 3, 3]; // 1↔2, 3s unchanged
        let input = vec![a.clone(), b.clone(), a.clone(), b.clone()];
        let ben = encode_twodelta(&input);
        assert_eq!(expand_assignments(&ben), input);
    }

    #[test]
    fn delta_values_are_applied_correctly() {
        // Explicit value check: the decoder must correctly update the previous assignment when it
        // applies the delta.
        //   anchor: [1, 2, 1, 2, 1]
        //   next:   [2, 1, 2, 1, 2]  (every element swaps 1↔2)
        let anchor = vec![1u16, 2, 1, 2, 1];
        let next = vec![2u16, 1, 2, 1, 2];
        let ben = encode_twodelta(&[anchor, next.clone()]);

        let mut decoder = BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .silent(true);
        let _ = decoder.next().unwrap().unwrap(); // skip anchor
        let (decoded_next, _) = decoder.next().unwrap().unwrap();
        assert_eq!(decoded_next, next);
    }

    #[test]
    fn partial_swap_delta_is_correct() {
        // Only some positions change: [1,2,3,1] → [2,2,3,2] (1s → 2s, 2s stay).
        let anchor = vec![1u16, 2, 3, 1];
        let next = vec![2u16, 2, 3, 2];
        let input = vec![anchor, next];
        let ben = encode_twodelta(&input);
        assert_eq!(expand_assignments(&ben), input);
    }

    #[test]
    fn long_delta_chain_round_trip() {
        // A longer chain: a, b, a, b, a, b (6 assignments, 3 a→b and 2 b→a deltas).
        let a = vec![1u16, 1, 2, 2, 1, 2];
        let b = vec![2u16, 2, 1, 1, 2, 1]; // 1↔2 everywhere
        let input: Vec<Vec<u16>> = (0..6)
            .map(|i| if i % 2 == 0 { a.clone() } else { b.clone() })
            .collect();
        let ben = encode_twodelta(&input);
        assert_eq!(expand_assignments(&ben), input);
    }

    // ─── repetition counts ────────────────────────────────────────────────────

    #[test]
    fn anchor_repetition_count_is_correct() {
        // Three identical anchor assignments → one frame with count=3.
        let anchor = vec![1u16, 1, 2, 2];
        let ben = encode_twodelta(&vec![anchor.clone(); 3]);

        let mut decoder = BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .silent(true);
        let (decoded, count) = decoder.next().unwrap().unwrap();
        assert_eq!(count, 3, "anchor count should be 3");
        assert_eq!(decoded, anchor);
        assert!(decoder.next().is_none());
    }

    #[test]
    fn delta_repetition_count_is_correct() {
        // a, b, b, b → anchor(a, 1), delta(a→b, 3).
        let a = vec![1u16, 1, 2, 2];
        let b = vec![2u16, 2, 1, 1];
        let assignments: Vec<_> = std::iter::once(a.clone())
            .chain((0..3).map(|_| b.clone()))
            .collect();
        let ben = encode_twodelta(&assignments);

        let mut decoder = BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .silent(true);

        let (d_anchor, c_anchor) = decoder.next().unwrap().unwrap();
        assert_eq!(c_anchor, 1, "anchor count");
        assert_eq!(d_anchor, a);

        let (d_delta, c_delta) = decoder.next().unwrap().unwrap();
        assert_eq!(c_delta, 3, "delta count should be 3");
        assert_eq!(d_delta, b);

        assert!(decoder.next().is_none());
    }

    #[test]
    fn anchor_and_delta_repetitions_round_trip() {
        // a×2, b×3 → anchor(2), delta(3). Expanding must give 5 correct assignments.
        let a = vec![1u16, 1, 2, 2];
        let b = vec![2u16, 2, 1, 1];
        let assignments: Vec<_> = (0..2)
            .map(|_| a.clone())
            .chain((0..3).map(|_| b.clone()))
            .collect();
        let ben = encode_twodelta(&assignments);
        assert_eq!(expand_assignments(&ben), assignments);
    }

    #[test]
    fn interleaved_repetitions_round_trip() {
        // a, b, b, a, a, a, b → anchor(a,1), delta(a→b,2), delta(b→a,3), delta(a→b,1).
        let a = vec![1u16, 1, 2, 2];
        let b = vec![2u16, 2, 1, 1];
        let assignments = vec![
            a.clone(),
            b.clone(),
            b.clone(),
            a.clone(),
            a.clone(),
            a.clone(),
            b.clone(),
        ];
        let ben = encode_twodelta(&assignments);
        assert_eq!(expand_assignments(&ben), assignments);
    }

    // ─── count_samples ────────────────────────────────────────────────────────

    #[test]
    fn count_samples_single_anchor() {
        let ben = encode_twodelta(&[vec![1u16, 2, 3]]);
        assert_eq!(
            BenStreamReader::from_ben(ben.as_slice())
                .unwrap()
                .count_samples()
                .unwrap(),
            1
        );
    }

    #[test]
    fn count_samples_anchor_plus_two_deltas() {
        let a = vec![1u16, 1, 2, 2];
        let b = vec![2u16, 2, 1, 1];
        let assignments = vec![a.clone(), b.clone(), a.clone()];
        let ben = encode_twodelta(&assignments);
        assert_eq!(
            BenStreamReader::from_ben(ben.as_slice())
                .unwrap()
                .count_samples()
                .unwrap(),
            3
        );
    }

    #[test]
    fn count_samples_expands_repetitions() {
        // a×2, b×3 → 5 total.
        let a = vec![1u16, 1, 2, 2];
        let b = vec![2u16, 2, 1, 1];
        let assignments: Vec<_> = (0..2)
            .map(|_| a.clone())
            .chain((0..3).map(|_| b.clone()))
            .collect();
        let ben = encode_twodelta(&assignments);
        assert_eq!(
            BenStreamReader::from_ben(ben.as_slice())
                .unwrap()
                .count_samples()
                .unwrap(),
            5
        );
    }

    // ─── write_all_jsonl ──────────────────────────────────────────────────────

    #[test]
    fn write_all_jsonl_single_anchor() {
        let assignment = vec![1u16, 2, 3];
        let ben = encode_twodelta(std::slice::from_ref(&assignment));

        let mut out = Vec::new();
        BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .write_all_jsonl(&mut out)
            .unwrap();
        let s = String::from_utf8(out).unwrap();

        assert_eq!(s.lines().count(), 1);
        assert!(s.contains("[1,2,3]"));
    }

    #[test]
    fn write_all_jsonl_expands_all_repetitions() {
        // a×2, b×3 → 5 lines with correct content.
        let a = vec![1u16, 1, 2, 2];
        let b = vec![2u16, 2, 1, 1];
        let assignments: Vec<_> = (0..2)
            .map(|_| a.clone())
            .chain((0..3).map(|_| b.clone()))
            .collect();
        let ben = encode_twodelta(&assignments);

        let mut out = Vec::new();
        BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .write_all_jsonl(&mut out)
            .unwrap();
        let s = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = s.lines().collect();

        assert_eq!(lines.len(), 5);
        assert!(lines[0].contains("[1,1,2,2]"), "line 0: {}", lines[0]);
        assert!(lines[1].contains("[1,1,2,2]"), "line 1: {}", lines[1]);
        assert!(lines[2].contains("[2,2,1,1]"), "line 2: {}", lines[2]);
        assert!(lines[3].contains("[2,2,1,1]"), "line 3: {}", lines[3]);
        assert!(lines[4].contains("[2,2,1,1]"), "line 4: {}", lines[4]);
    }

    #[test]
    fn write_all_jsonl_sample_numbers_are_sequential() {
        let a = vec![1u16, 1, 2, 2];
        let b = vec![2u16, 2, 1, 1];
        let assignments: Vec<_> = (0..2)
            .map(|_| a.clone())
            .chain((0..3).map(|_| b.clone()))
            .collect();
        let ben = encode_twodelta(&assignments);

        let mut out = Vec::new();
        BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .write_all_jsonl(&mut out)
            .unwrap();
        let s = String::from_utf8(out).unwrap();

        let parsed: Vec<serde_json::Value> = s
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        for (i, v) in parsed.iter().enumerate() {
            assert_eq!(v["sample"], i as u64 + 1, "sample number at position {i}");
        }
    }

    #[test]
    fn write_all_jsonl_matches_codec_decode() {
        let a = vec![1u16, 2, 1, 2];
        let b = vec![2u16, 1, 2, 1];
        let ben = encode_twodelta(&[a.clone(), b.clone(), a.clone()]);

        let mut via_reader = Vec::new();
        BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .write_all_jsonl(&mut via_reader)
            .unwrap();

        let mut via_codec = Vec::new();
        decode_ben_to_jsonl(ben.as_slice(), &mut via_codec).unwrap();

        assert_eq!(via_reader, via_codec);
    }

    // ─── for_each_assignment ─────────────────────────────────────────────────

    #[test]
    fn for_each_receives_anchor_count() {
        let anchor = vec![1u16, 1, 2, 2];
        let ben = encode_twodelta(&vec![anchor.clone(); 4]);

        let mut seen: Vec<(Vec<u16>, u16)> = Vec::new();
        BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .silent(true)
            .for_each_assignment(|a, count| {
                seen.push((a.to_vec(), count));
                Ok(true)
            })
            .unwrap();

        assert_eq!(seen, vec![(anchor, 4)]);
    }

    #[test]
    fn for_each_receives_anchor_and_delta_counts() {
        // a×2, b×3 → callback invoked twice: (a, 2) then (b, 3).
        let a = vec![1u16, 1, 2, 2];
        let b = vec![2u16, 2, 1, 1];
        let assignments: Vec<_> = (0..2)
            .map(|_| a.clone())
            .chain((0..3).map(|_| b.clone()))
            .collect();
        let ben = encode_twodelta(&assignments);

        let mut frames: Vec<(Vec<u16>, u16)> = Vec::new();
        BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .silent(true)
            .for_each_assignment(|assignment, count| {
                frames.push((assignment.to_vec(), count));
                Ok(true)
            })
            .unwrap();

        assert_eq!(frames, vec![(a, 2), (b, 3)]);
    }

    #[test]
    fn for_each_early_stop() {
        // Three distinct frames; stopping after the second delivers exactly 2.
        let a = vec![1u16, 2, 1, 2];
        let b = vec![2u16, 1, 2, 1];
        let c = vec![1u16, 1, 2, 2];
        let ben = encode_twodelta(&[a.clone(), b.clone(), c.clone()]);

        let mut seen = Vec::new();
        BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .silent(true)
            .for_each_assignment(|assignment, _count| {
                seen.push(assignment.to_vec());
                Ok(seen.len() < 2)
            })
            .unwrap();

        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0], a);
        assert_eq!(seen[1], b);
    }

    // ─── into_frames (BenStreamFrameReader) ──────────────────────────────────

    #[test]
    fn into_frames_count_is_preserved_through_re_encoding() {
        // a×2, b×3 → 2 re-encoded frames with counts [2, 3].
        let a = vec![1u16, 1, 2, 2];
        let b = vec![2u16, 2, 1, 1];
        let assignments: Vec<_> = (0..2)
            .map(|_| a.clone())
            .chain((0..3).map(|_| b.clone()))
            .collect();
        let ben = encode_twodelta(&assignments);

        let frames: Vec<_> = BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .silent(true)
            .into_frames()
            .collect::<io::Result<Vec<_>>>()
            .unwrap();

        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].1, 2, "anchor frame count");
        assert_eq!(frames[1].1, 3, "delta frame count");
    }

    #[test]
    fn into_frames_decodes_to_correct_assignments() {
        // Each re-encoded frame must decode back to the materialized assignment.
        let a = vec![1u16, 1, 2, 2];
        let b = vec![2u16, 2, 1, 1];
        let c = vec![1u16, 2, 1, 2];
        let input = vec![a.clone(), b.clone(), c.clone()];
        let ben = encode_twodelta(&input);

        let frames: Vec<_> = BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .silent(true)
            .into_frames()
            .collect::<io::Result<Vec<_>>>()
            .unwrap();

        assert_eq!(frames.len(), 3);
        for (i, (frame, _count)) in frames.iter().enumerate() {
            let decoded = frame.expand_self_contained().unwrap();
            assert_eq!(decoded, input[i], "frame {i} decoded incorrectly");
        }
    }

    #[test]
    fn into_frames_from_anchor_only_has_single_frame_with_count_one() {
        let assignment = vec![1u16, 2, 3];
        let ben = encode_twodelta(&[assignment]);
        let frames: Vec<_> = BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .silent(true)
            .into_frames()
            .collect::<io::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].1, 1);
    }

    #[test]
    fn into_frames_length_matches_unique_assignment_count() {
        // a, b, a, b, a → 5 distinct frames (no run-length compression).
        let a = vec![1u16, 1, 2, 2];
        let b = vec![2u16, 2, 1, 1];
        let input: Vec<_> = (0..5)
            .map(|i| if i % 2 == 0 { a.clone() } else { b.clone() })
            .collect();
        let ben = encode_twodelta(&input);

        let frames: Vec<_> = BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .silent(true)
            .into_frames()
            .collect::<io::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(frames.len(), 5, "expected one frame per unique transition");
    }

    // ─── subsampling ──────────────────────────────────────────────────────────

    #[test]
    fn subsample_by_indices_distinct_frames() {
        // Five distinct assignments; select 1-based indices 1, 3, 5.
        let a = vec![1u16, 1, 2, 2];
        let b = vec![2u16, 2, 1, 1];
        let c = vec![1u16, 2, 1, 2];
        let input = vec![a.clone(), b.clone(), c.clone(), a.clone(), b.clone()];
        let ben = encode_twodelta(&input);

        let selected: Vec<Vec<u16>> = BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .silent(true)
            .into_subsample_by_indices(vec![1usize, 3, 5])
            .map(|r| r.unwrap().0)
            .collect();

        assert_eq!(selected, vec![a.clone(), c.clone(), b.clone()]);
    }

    #[test]
    fn subsample_by_range_distinct_frames() {
        let a = vec![1u16, 1, 2, 2];
        let b = vec![2u16, 2, 1, 1];
        let c = vec![1u16, 2, 1, 2];
        let input = vec![a.clone(), b.clone(), c.clone(), a.clone(), b.clone()];
        let ben = encode_twodelta(&input);

        // Range [2, 4] → 3 assignments: b, c, a.
        let selected: Vec<Vec<u16>> = BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .silent(true)
            .into_subsample_by_range(2, 4)
            .map(|r| r.unwrap().0)
            .collect();

        assert_eq!(selected, vec![b.clone(), c.clone(), a.clone()]);
    }

    #[test]
    fn subsample_every_distinct_frames() {
        // 6 cycling assignments: a,b,c,a,b,c. Every 3 from offset 1 → indices 1,4 → a,a.
        let a = vec![1u16, 1, 2, 2];
        let b = vec![2u16, 2, 1, 1];
        let c = vec![1u16, 2, 1, 2];
        let input = vec![
            a.clone(),
            b.clone(),
            c.clone(),
            a.clone(),
            b.clone(),
            c.clone(),
        ];
        let ben = encode_twodelta(&input);

        let selected: Vec<Vec<u16>> = BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .silent(true)
            .into_subsample_every(3, 1)
            .map(|r| r.unwrap().0)
            .collect();

        assert_eq!(selected, vec![a.clone(), a.clone()]);
    }

    #[test]
    fn subsample_by_indices_across_repeated_frames() {
        // a×3, b×3 → 6 samples from 2 frames. Indices 1 and 3 fall in the anchor (a) frame →
        // (a, 2). Index 4 is the first b → (b, 1).
        let a = vec![1u16, 1, 2, 2];
        let b = vec![2u16, 2, 1, 1];
        let assignments: Vec<_> = (0..3)
            .map(|_| a.clone())
            .chain((0..3).map(|_| b.clone()))
            .collect();
        let ben = encode_twodelta(&assignments);

        let selected: Vec<(Vec<u16>, u16)> = BenStreamReader::from_ben(ben.as_slice())
            .unwrap()
            .silent(true)
            .into_subsample_by_indices(vec![1usize, 3, 4])
            .map(|r| r.unwrap())
            .collect();

        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0], (a, 2)); // indices 1,3 in anchor frame
        assert_eq!(selected[1], (b, 1)); // index 4 in delta frame
    }

    // ─── error paths ─────────────────────────────────────────────────────────

    #[test]
    fn truncated_anchor_errors_on_next() {
        let assignment = vec![1u16, 2, 3];
        let ben = encode_twodelta(&[assignment]);
        let truncated = &ben[..ben.len() - 1];
        let err = BenStreamReader::from_ben(truncated)
            .unwrap()
            .next()
            .unwrap()
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn truncated_delta_errors_on_next() {
        let a = vec![1u16, 1, 2, 2];
        let b = vec![2u16, 2, 1, 1];
        let ben = encode_twodelta(&[a.clone(), b.clone()]);
        let truncated = &ben[..ben.len() - 1];

        let mut decoder = BenStreamReader::from_ben(truncated).unwrap().silent(true);
        let _ = decoder.next().unwrap().unwrap(); // anchor succeeds
        let err = decoder.next().unwrap().unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn count_samples_propagates_truncation_error() {
        let a = vec![1u16, 1, 2, 2];
        let b = vec![2u16, 2, 1, 1];
        let ben = encode_twodelta(&[a, b]);
        let truncated = &ben[..ben.len() - 1];
        let err = BenStreamReader::from_ben(truncated)
            .unwrap()
            .count_samples()
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn write_all_jsonl_propagates_truncation_error() {
        let a = vec![1u16, 1, 2, 2];
        let b = vec![2u16, 2, 1, 1];
        let ben = encode_twodelta(&[a, b]);
        let truncated = &ben[..ben.len() - 1];
        let err = BenStreamReader::from_ben(truncated)
            .unwrap()
            .write_all_jsonl(io::sink())
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }
}
