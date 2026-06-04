use crate::codec::encode::encode_jsonl_to_xben;
use crate::io::reader::errors::DecoderInitError;
use crate::io::reader::subsample::{DecodeFrame, Selection, SubsampleFrameDecoder};
use crate::io::reader::{BenStreamFrameReader, BenStreamReader, BenWireFormat};
use crate::io::writer::BenStreamWriter;
use crate::BenVariant;
use std::io::{self, Cursor, Write};
use xz2::write::XzEncoder;

/// Build a minimal XBEN stream from JSONL input for testing.
fn make_xben(jsonl: &str, variant: BenVariant) -> Vec<u8> {
    let mut xben = Vec::new();
    encode_jsonl_to_xben(
        jsonl.as_bytes(),
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

/// Build a minimal XBEN stream using BenStreamWriter directly.
fn make_xben_from_assignments(assignments: &[Vec<u16>], variant: BenVariant) -> Vec<u8> {
    let mut xben = Vec::new();
    {
        let encoder = XzEncoder::new(&mut xben, 1);
        let mut writer = BenStreamWriter::for_xben_with_encoder(encoder, variant, None).unwrap();
        for a in assignments {
            writer.write_assignment(a.clone()).unwrap();
        }
    }
    xben
}

// ── BenStreamReader ──────────────────────────────────────────────────────

#[test]
fn xz_reader_standard_iterator() {
    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[2,2,1,1],"sample":2}
"#;
    let xben = make_xben(jsonl, BenVariant::Standard);
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    assert_eq!(reader.variant(), BenVariant::Standard);
    let results: Vec<_> = reader.collect();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].as_ref().unwrap().0, vec![1, 1, 2, 2]);
    assert_eq!(results[0].as_ref().unwrap().1, 1);
    assert_eq!(results[1].as_ref().unwrap().0, vec![2, 2, 1, 1]);
}

#[test]
fn xz_reader_mkv_iterator() {
    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[1,1,2,2],"sample":2}
{"assignment":[2,2,1,1],"sample":3}
"#;
    let xben = make_xben(jsonl, BenVariant::MkvChain);
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    assert_eq!(reader.variant(), BenVariant::MkvChain);
    let results: Vec<_> = reader.collect();
    // MkvChain collapses identical consecutive assignments
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].as_ref().unwrap().1, 2); // count=2
    assert_eq!(results[1].as_ref().unwrap().1, 1); // count=1
}

#[test]
fn xz_reader_twodelta_iterator() {
    let assignments = vec![vec![1u16, 1, 2, 2], vec![2, 1, 2, 2], vec![2, 2, 2, 2]];
    let xben = make_xben_from_assignments(&assignments, BenVariant::TwoDelta);
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    assert_eq!(reader.variant(), BenVariant::TwoDelta);
    let results: Vec<_> = reader.map(|r| r.unwrap().0).collect();
    assert_eq!(results, assignments);
}

#[test]
fn xz_reader_count_samples_standard() {
    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[2,2,1,1],"sample":2}
{"assignment":[1,2,1,2],"sample":3}
"#;
    let xben = make_xben(jsonl, BenVariant::Standard);
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    assert_eq!(reader.count_samples().unwrap(), 3);
}

#[test]
fn xz_reader_count_samples_mkv() {
    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[1,1,2,2],"sample":2}
{"assignment":[2,2,1,1],"sample":3}
"#;
    let xben = make_xben(jsonl, BenVariant::MkvChain);
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    assert_eq!(reader.count_samples().unwrap(), 3);
}

#[test]
fn xz_reader_silent_suppresses_output() {
    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
"#;
    let xben = make_xben(jsonl, BenVariant::Standard);
    let reader = BenStreamReader::from_xben(Cursor::new(xben))
        .unwrap()
        .silent(true);
    let results: Vec<_> = reader.collect();
    assert_eq!(results.len(), 1);
}

#[test]
fn xz_reader_for_each_assignment() {
    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[2,2,1,1],"sample":2}
"#;
    let xben = make_xben(jsonl, BenVariant::Standard);
    let mut reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let mut collected = Vec::new();
    reader
        .for_each_assignment(|assignment, count| {
            collected.push((assignment.to_vec(), count));
            Ok(true)
        })
        .unwrap();
    assert_eq!(collected.len(), 2);
    assert_eq!(collected[0].0, vec![1, 1, 2, 2]);
    assert_eq!(collected[1].0, vec![2, 2, 1, 1]);
}

#[test]
fn xz_reader_for_each_assignment_early_stop() {
    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[2,2,1,1],"sample":2}
{"assignment":[3,3,3,3],"sample":3}
"#;
    let xben = make_xben(jsonl, BenVariant::Standard);
    let mut reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let mut collected = Vec::new();
    reader
        .for_each_assignment(|assignment, _count| {
            collected.push(assignment.to_vec());
            Ok(false) // stop after first
        })
        .unwrap();
    assert_eq!(collected.len(), 1);
}

#[test]
fn xz_reader_write_all_jsonl() {
    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[2,2,1,1],"sample":2}
"#;
    let xben = make_xben(jsonl, BenVariant::Standard);
    let mut reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let mut output = Vec::new();
    reader.write_all_jsonl(&mut output).unwrap();
    let output_str = String::from_utf8(output).unwrap();
    let lines: Vec<&str> = output_str.trim().split('\n').collect();
    assert_eq!(lines.len(), 2);
    let v1: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(v1["assignment"], serde_json::json!([1, 1, 2, 2]));
    assert_eq!(v1["sample"], 1);
}

#[test]
fn xz_reader_write_all_jsonl_mkv_expands_counts() {
    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[1,1,2,2],"sample":2}
{"assignment":[2,2,1,1],"sample":3}
"#;
    let xben = make_xben(jsonl, BenVariant::MkvChain);
    let mut reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let mut output = Vec::new();
    reader.write_all_jsonl(&mut output).unwrap();
    let output_str = String::from_utf8(output).unwrap();
    let lines: Vec<&str> = output_str.trim().split('\n').collect();
    assert_eq!(lines.len(), 3); // expanded from count=2 + count=1
}

#[test]
fn xz_reader_into_frames_standard() {
    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[2,2,1,1],"sample":2}
"#;
    let xben = make_xben(jsonl, BenVariant::Standard);
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let frames: Vec<_> = reader.into_frames().collect();
    assert_eq!(frames.len(), 2);
    for f in &frames {
        let (frame, count) = f.as_ref().unwrap();
        let bytes = match frame {
            DecodeFrame::XBen(b, _) => b,
            DecodeFrame::Ben(_) => panic!("xben frame iterator yielded BEN arm"),
        };
        assert!(!bytes.is_empty());
        assert_eq!(*count, 1);
    }
}

#[test]
fn xz_reader_into_frames_twodelta() {
    let assignments = vec![vec![1u16, 1, 2, 2], vec![2, 1, 2, 2]];
    let xben = make_xben_from_assignments(&assignments, BenVariant::TwoDelta);
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let frames: Vec<_> = reader.into_frames().collect();
    assert_eq!(frames.len(), 2);
}

/// Build a plain-BEN stream using BenStreamWriter directly.
fn make_ben_from_assignments(assignments: &[Vec<u16>], variant: BenVariant) -> Vec<u8> {
    let mut ben = Vec::new();
    {
        let mut writer = BenStreamWriter::for_ben(&mut ben, variant).unwrap();
        for a in assignments {
            writer.write_assignment(a.clone()).unwrap();
        }
    }
    ben
}

/// A TwoDelta sequence that exercises every framing path: anchor (full), a 2-swap delta, a
/// multi-district transition (mid-stream snapshot), a repeat of that snapshot, and a delta rebased
/// onto it.
fn mixed_twodelta_assignments() -> Vec<Vec<u16>> {
    vec![
        vec![1u16, 1, 2, 2],
        vec![1u16, 2, 1, 2],
        vec![3u16, 3, 1, 2],
        vec![3u16, 3, 1, 2],
        vec![3u16, 3, 2, 1],
    ]
}

/// Drive a raw-frame iterator to completion, expanding each self-contained frame `count` times.
fn expand_raw_frames<R: io::Read>(frames: BenStreamFrameReader<R>) -> Vec<Vec<u16>> {
    let mut out = Vec::new();
    for item in frames {
        let (frame, count) = item.unwrap();
        let assignment = frame.expand_self_contained().unwrap();
        for _ in 0..count {
            out.push(assignment.clone());
        }
    }
    out
}

#[test]
fn raw_frame_surface_roundtrips_mixed_twodelta_ben() {
    // The subsample/raw-frame surface materializes each TwoDelta frame and re-encodes it as a
    // self-contained Standard frame. A mixed snapshot/delta stream must round-trip across it.
    let assignments = mixed_twodelta_assignments();
    let ben = make_ben_from_assignments(&assignments, BenVariant::TwoDelta);
    let frames = BenStreamReader::from_ben(Cursor::new(ben))
        .unwrap()
        .into_frames();
    assert_eq!(expand_raw_frames(frames), assignments);
}

#[test]
fn raw_frame_surface_roundtrips_mixed_twodelta_xben() {
    let assignments = mixed_twodelta_assignments();
    let xben = make_xben_from_assignments(&assignments, BenVariant::TwoDelta);
    let frames = BenStreamReader::from_xben(Cursor::new(xben))
        .unwrap()
        .into_frames();
    assert_eq!(expand_raw_frames(frames), assignments);
}

#[test]
fn subsample_mixed_twodelta_ben_selects_correct_samples() {
    // Subsampling rides on the raw-frame surface; selecting across the mid-stream snapshot must
    // still rebase the later delta correctly.
    let assignments = mixed_twodelta_assignments();
    let ben = make_ben_from_assignments(&assignments, BenVariant::TwoDelta);
    let results: Vec<_> = BenStreamReader::from_ben(Cursor::new(ben))
        .unwrap()
        .into_subsample_by_indices(vec![1, 3, 5])
        .map(|r| r.unwrap().0)
        .collect();
    assert_eq!(
        results,
        vec![
            assignments[0].clone(),
            assignments[2].clone(),
            assignments[4].clone()
        ]
    );
}

#[test]
fn subsample_mixed_twodelta_xben_selects_correct_samples() {
    let assignments = mixed_twodelta_assignments();
    let xben = make_xben_from_assignments(&assignments, BenVariant::TwoDelta);
    let results: Vec<_> = BenStreamReader::from_xben(Cursor::new(xben))
        .unwrap()
        .into_subsample_by_indices(vec![1, 3, 5])
        .map(|r| r.unwrap().0)
        .collect();
    assert_eq!(
        results,
        vec![
            assignments[0].clone(),
            assignments[2].clone(),
            assignments[4].clone()
        ]
    );
}

#[test]
fn xz_frame_reader_new() {
    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
"#;
    let xben = make_xben(jsonl, BenVariant::Standard);
    let reader = BenStreamFrameReader::from_xben(Cursor::new(xben)).unwrap();
    let frames: Vec<_> = reader.collect();
    assert_eq!(frames.len(), 1);
}

#[test]
fn xz_reader_new_rejects_invalid_data() {
    let garbage = vec![0u8; 100];
    let result = BenStreamReader::from_xben(Cursor::new(garbage));
    assert!(result.is_err());
}

// ── BenStreamReader subsample ────────────────────────────────────────────

#[test]
fn xz_reader_subsample_by_indices() {
    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[2,2,1,1],"sample":2}
{"assignment":[3,3,3,3],"sample":3}
"#;
    let xben = make_xben(jsonl, BenVariant::Standard);
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader
        .into_subsample_by_indices(vec![1, 3])
        .map(|r| r.unwrap().0)
        .collect();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0], vec![1, 1, 2, 2]);
    assert_eq!(results[1], vec![3, 3, 3, 3]);
}

#[test]
fn xz_reader_subsample_by_range() {
    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[2,2,1,1],"sample":2}
{"assignment":[3,3,3,3],"sample":3}
"#;
    let xben = make_xben(jsonl, BenVariant::Standard);
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader
        .into_subsample_by_range(2, 3)
        .map(|r| r.unwrap().0)
        .collect();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0], vec![2, 2, 1, 1]);
    assert_eq!(results[1], vec![3, 3, 3, 3]);
}

#[test]
fn xz_reader_subsample_every() {
    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[2,2,1,1],"sample":2}
{"assignment":[3,3,3,3],"sample":3}
{"assignment":[4,4,4,4],"sample":4}
"#;
    let xben = make_xben(jsonl, BenVariant::Standard);
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader
        .into_subsample_every(2, 1) // samples 1, 3
        .map(|r| r.unwrap().0)
        .collect();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0], vec![1, 1, 2, 2]);
    assert_eq!(results[1], vec![3, 3, 3, 3]);
}

// ── BenStreamReader for_each_assignment with silent ──────────────────────

#[test]
fn xz_reader_for_each_assignment_silent() {
    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[2,2,1,1],"sample":2}
"#;
    let xben = make_xben(jsonl, BenVariant::Standard);
    let mut reader = BenStreamReader::from_xben(Cursor::new(xben))
        .unwrap()
        .silent(true);
    let mut count = 0usize;
    reader
        .for_each_assignment(|_assignment, _cnt| {
            count += 1;
            Ok(true)
        })
        .unwrap();
    assert_eq!(count, 2);
}

// ── BenStreamReader TwoDelta write_all_jsonl ─────────────────────────────

#[test]
fn xz_reader_write_all_jsonl_twodelta() {
    let assignments = vec![vec![1u16, 1, 2, 2], vec![2, 1, 2, 2], vec![2, 2, 2, 2]];
    let xben = make_xben_from_assignments(&assignments, BenVariant::TwoDelta);
    let mut reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let mut output = Vec::new();
    reader.write_all_jsonl(&mut output).unwrap();
    let output_str = String::from_utf8(output).unwrap();
    let lines: Vec<&str> = output_str.trim().split('\n').collect();
    assert_eq!(lines.len(), 3);
}

// ── BenStreamReader TwoDelta count_samples ───────────────────────────────

#[test]
fn xz_reader_count_samples_twodelta() {
    let assignments = vec![vec![1u16, 1, 2, 2], vec![2, 1, 2, 2]];
    let xben = make_xben_from_assignments(&assignments, BenVariant::TwoDelta);
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    assert_eq!(reader.count_samples().unwrap(), 2);
}

// ── Content verification tests ─────────────────────────────────────────────

#[test]
fn xz_reader_into_frames_standard_content() {
    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[3,3,4,4],"sample":2}
"#;
    let xben = make_xben(jsonl, BenVariant::Standard);
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let frames: Vec<_> = reader.into_frames().collect();
    assert_eq!(frames.len(), 2);
    // Verify frame bytes can be decoded back
    for f in &frames {
        let (frame, count) = f.as_ref().unwrap();
        let bytes = match frame {
            DecodeFrame::XBen(b, _) => b,
            DecodeFrame::Ben(_) => panic!("xben frame iterator yielded BEN arm"),
        };
        assert!(!bytes.is_empty());
        assert_eq!(*count, 1);
    }
}

#[test]
fn xz_reader_write_all_jsonl_standard_content_verified() {
    let jsonl = r#"{"assignment":[5,6,7],"sample":1}
{"assignment":[8,9,10],"sample":2}
"#;
    let xben = make_xben(jsonl, BenVariant::Standard);
    let mut reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let mut output = Vec::new();
    reader.write_all_jsonl(&mut output).unwrap();
    let output_str = String::from_utf8(output).unwrap();
    let lines: Vec<&str> = output_str.trim().split('\n').collect();
    assert_eq!(lines.len(), 2);
    let v1: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(v1["assignment"], serde_json::json!([5, 6, 7]));
    assert_eq!(v1["sample"], 1);
    let v2: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(v2["assignment"], serde_json::json!([8, 9, 10]));
    assert_eq!(v2["sample"], 2);
}

#[test]
fn xz_reader_write_all_jsonl_mkv_content_verified() {
    let jsonl = r#"{"assignment":[1,2,3],"sample":1}
{"assignment":[1,2,3],"sample":2}
{"assignment":[4,5,6],"sample":3}
"#;
    let xben = make_xben(jsonl, BenVariant::MkvChain);
    let mut reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let mut output = Vec::new();
    reader.write_all_jsonl(&mut output).unwrap();
    let output_str = String::from_utf8(output).unwrap();
    let lines: Vec<&str> = output_str.trim().split('\n').collect();
    assert_eq!(lines.len(), 3);
    let v1: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(v1["assignment"], serde_json::json!([1, 2, 3]));
    let v2: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(v2["assignment"], serde_json::json!([1, 2, 3]));
    let v3: serde_json::Value = serde_json::from_str(lines[2]).unwrap();
    assert_eq!(v3["assignment"], serde_json::json!([4, 5, 6]));
}

// ── Single sample streams ──────────────────────────────────────────────────

#[test]
fn xz_reader_single_sample_standard() {
    let jsonl = r#"{"assignment":[42],"sample":1}
"#;
    let xben = make_xben(jsonl, BenVariant::Standard);
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader.collect();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].as_ref().unwrap().0, vec![42]);
    assert_eq!(results[0].as_ref().unwrap().1, 1);
}

#[test]
fn xz_reader_single_sample_twodelta() {
    let xben = make_xben_from_assignments(&[vec![1u16, 2, 3]], BenVariant::TwoDelta);
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader.map(|r| r.unwrap().0).collect();
    assert_eq!(results, vec![vec![1, 2, 3]]);
}

// ── Subsample edge cases ────────────────────────────────────────────────────

#[test]
fn xz_reader_subsample_by_indices_deduplicates_and_sorts() {
    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[2,2,1,1],"sample":2}
{"assignment":[3,3,3,3],"sample":3}
"#;
    let xben = make_xben(jsonl, BenVariant::Standard);
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    // Pass unsorted duplicates: [3,1,3,1] → sorted+deduped [1,3]
    let results: Vec<_> = reader
        .into_subsample_by_indices(vec![3, 1, 3, 1])
        .map(|r| r.unwrap().0)
        .collect();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0], vec![1, 1, 2, 2]);
    assert_eq!(results[1], vec![3, 3, 3, 3]);
}

#[test]
fn xz_reader_subsample_by_indices_beyond_stream() {
    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[2,2,1,1],"sample":2}
"#;
    let xben = make_xben(jsonl, BenVariant::Standard);
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    // Index 5 is beyond the stream (only 2 samples)
    let results: Vec<_> = reader
        .into_subsample_by_indices(vec![5])
        .map(|r| r.unwrap().0)
        .collect();
    assert_eq!(results.len(), 0);
}

#[test]
fn xz_reader_subsample_by_range_single_element() {
    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[2,2,1,1],"sample":2}
{"assignment":[3,3,3,3],"sample":3}
"#;
    let xben = make_xben(jsonl, BenVariant::Standard);
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader
        .into_subsample_by_range(2, 2) // only sample 2
        .map(|r| r.unwrap().0)
        .collect();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0], vec![2, 2, 1, 1]);
}

#[test]
fn xz_reader_subsample_every_offset_beyond_stream() {
    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[2,2,1,1],"sample":2}
"#;
    let xben = make_xben(jsonl, BenVariant::Standard);
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    // Offset 10 is beyond the stream
    let results: Vec<_> = reader
        .into_subsample_every(1, 10)
        .map(|r| r.unwrap().0)
        .collect();
    assert_eq!(results.len(), 0);
}

#[test]
fn xz_reader_subsample_mkv_with_count_gt_1() {
    let jsonl = r#"{"assignment":[1,2,3],"sample":1}
{"assignment":[1,2,3],"sample":2}
{"assignment":[1,2,3],"sample":3}
{"assignment":[4,5,6],"sample":4}
"#;
    let xben = make_xben(jsonl, BenVariant::MkvChain);
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    // Select sample 2 (middle of the count=3 frame) and sample 4
    let results: Vec<_> = reader
        .into_subsample_by_indices(vec![2, 4])
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].0, vec![1, 2, 3]);
    assert_eq!(results[1].0, vec![4, 5, 6]);
}

#[test]
fn xz_reader_subsample_twodelta() {
    let assignments = vec![vec![1u16, 1, 2, 2], vec![2, 1, 2, 2], vec![2, 2, 2, 2]];
    let xben = make_xben_from_assignments(&assignments, BenVariant::TwoDelta);
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader
        .into_subsample_by_indices(vec![1, 3])
        .map(|r| r.unwrap().0)
        .collect();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0], vec![1, 1, 2, 2]);
    assert_eq!(results[1], vec![2, 2, 2, 2]);
}

// ── DecoderInitError tests ─────────────────────────────────────────────────

#[test]
fn decoder_init_error_xz_header_detected() {
    // Feed XZ-compressed data to a reader that expects uncompressed BEN
    use crate::io::reader::BenStreamReader;
    let xz_magic = b"\xFD\x37\x7A\x58\x5A\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00";
    let result = BenStreamReader::from_ben(xz_magic.as_slice());
    assert!(result.is_err());
    let io_err: std::io::Error = result.err().unwrap().into();
    assert_eq!(io_err.kind(), std::io::ErrorKind::InvalidData);
    assert!(io_err.to_string().contains("Compressed header"));
}

#[test]
fn decoder_init_error_unknown_banner() {
    use crate::io::reader::BenStreamReader;
    let bad_banner = b"THIS IS NOT BEN!!";
    let result = BenStreamReader::from_ben(bad_banner.as_slice());
    assert!(result.is_err());
    let io_err: std::io::Error = result.err().unwrap().into();
    assert_eq!(io_err.kind(), std::io::ErrorKind::InvalidData);
    assert!(io_err.to_string().contains("Invalid file format"));
}

#[test]
fn decoder_init_error_io() {
    use crate::io::reader::BenStreamReader;
    struct FailReader;
    impl std::io::Read for FailReader {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "broken",
            ))
        }
    }
    let result = BenStreamReader::from_ben(FailReader);
    assert!(result.is_err());
    let io_err: std::io::Error = result.err().unwrap().into();
    assert_eq!(io_err.kind(), std::io::ErrorKind::BrokenPipe);
}

#[test]
fn decoder_init_error_unknown_mode() {
    let err = DecoderInitError::UnknownMode {
        mode: "foo".to_string(),
    };
    let io_err: std::io::Error = err.into();
    assert_eq!(io_err.kind(), std::io::ErrorKind::InvalidInput);
    assert!(io_err.to_string().contains("foo"));
}

// ── for_each_assignment edge cases ─────────────────────────────────────────

#[test]
fn xz_reader_for_each_assignment_callback_error_propagates() {
    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[2,2,1,1],"sample":2}
"#;
    let xben = make_xben(jsonl, BenVariant::Standard);
    let mut reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let err = reader
        .for_each_assignment(|_assignment, _count| Err(std::io::Error::other("callback failed")))
        .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::Other);
    assert_eq!(err.to_string(), "callback failed");
}

// ── Large assignment vector ────────────────────────────────────────────────

#[test]
fn xz_reader_large_assignment_roundtrip() {
    let big_assign: Vec<u16> = (1..=1000).collect();
    let xben = make_xben_from_assignments(std::slice::from_ref(&big_assign), BenVariant::Standard);
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader.map(|r| r.unwrap().0).collect();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0], big_assign);
}

// ── SubsampleFrameDecoder stress tests ────────────────────────────────

#[test]
fn subsample_every_start_beyond_hi_returns_zero() {
    let assignments = vec![vec![1u16, 2, 3], vec![4, 5, 6]];
    let xben = make_xben_from_assignments(&assignments, BenVariant::Standard);
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader
        .into_subsample_every(1, 100)
        .map(|r| r.unwrap().0)
        .collect();
    assert_eq!(results.len(), 0);
}

#[test]
fn subsample_range_non_overlapping_returns_empty() {
    let assignments = vec![vec![1u16, 2], vec![3, 4], vec![5, 6]];
    let xben = make_xben_from_assignments(&assignments, BenVariant::Standard);
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader
        .into_subsample_by_range(10, 20)
        .map(|r| r.unwrap().0)
        .collect();
    assert_eq!(results.len(), 0);
}

#[test]
fn subsample_indices_mixed_before_and_after() {
    let assignments: Vec<Vec<u16>> = (1..=5).map(|i| vec![i; 3]).collect();
    let xben = make_xben_from_assignments(&assignments, BenVariant::Standard);
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader
        .into_subsample_by_indices(vec![2, 4, 100])
        .map(|r| r.unwrap().0)
        .collect();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0], vec![2, 2, 2]);
    assert_eq!(results[1], vec![4, 4, 4]);
}

#[test]
fn subsample_every_step_larger_than_stream() {
    let assignments = vec![vec![1u16, 2], vec![3, 4]];
    let xben = make_xben_from_assignments(&assignments, BenVariant::Standard);
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader
        .into_subsample_every(100, 1)
        .map(|r| r.unwrap().0)
        .collect();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0], vec![1, 2]);
}

#[test]
fn subsample_indices_empty_yields_nothing() {
    let assignments = vec![vec![1u16, 2]];
    let xben = make_xben_from_assignments(&assignments, BenVariant::Standard);
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader
        .into_subsample_by_indices(Vec::<usize>::new())
        .map(|r| r.unwrap().0)
        .collect();
    assert_eq!(results.len(), 0);
}

#[test]
fn subsample_twodelta_by_range() {
    let assignments = vec![vec![1u16, 1, 2, 2], vec![2, 1, 2, 2], vec![2, 2, 2, 2]];
    let xben = make_xben_from_assignments(&assignments, BenVariant::TwoDelta);
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader
        .into_subsample_by_range(2, 3)
        .map(|r| r.unwrap().0)
        .collect();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0], vec![2, 1, 2, 2]);
    assert_eq!(results[1], vec![2, 2, 2, 2]);
}

#[test]
fn subsample_twodelta_every() {
    let assignments = vec![
        vec![1u16, 1, 2, 2],
        vec![2, 1, 1, 2],
        vec![1, 2, 1, 2],
        vec![2, 1, 2, 1],
    ];
    let xben = make_xben_from_assignments(&assignments, BenVariant::TwoDelta);
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader
        .into_subsample_every(2, 1)
        .map(|r| r.unwrap().0)
        .collect();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0], vec![1, 1, 2, 2]);
    assert_eq!(results[1], vec![1, 2, 1, 2]);
}

// ── XBEN TwoDelta writer stress tests (roundtrip via reader) ──────────

#[test]
fn xz_twodelta_many_identical_assignments_roundtrip() {
    let assign = vec![1u16, 2, 1, 2];
    let assignments: Vec<_> = (0..100).map(|_| assign.clone()).collect();
    let xben = make_xben_from_assignments(&assignments, BenVariant::TwoDelta);
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader.map(|r| r.unwrap()).collect();
    let total_samples: usize = results.iter().map(|(_, c)| *c as usize).sum();
    assert_eq!(total_samples, 100);
    for (a, _) in &results {
        assert_eq!(a, &assign);
    }
}

#[test]
fn xz_twodelta_all_identical_single_value_roundtrip() {
    let assign = vec![5u16; 10];
    let assignments: Vec<_> = (0..10).map(|_| assign.clone()).collect();
    let xben = make_xben_from_assignments(&assignments, BenVariant::TwoDelta);
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader.map(|r| r.unwrap()).collect();
    let total: usize = results.iter().map(|(_, c)| *c as usize).sum();
    assert_eq!(total, 10);
    for (a, _) in &results {
        assert_eq!(a, &assign);
    }
}

#[test]
fn xz_twodelta_alternating_assignments_roundtrip() {
    let a = vec![1u16, 1, 2, 2];
    let b = vec![2u16, 2, 1, 1];
    let assignments: Vec<_> = (0..50)
        .map(|i| if i % 2 == 0 { a.clone() } else { b.clone() })
        .collect();
    let xben = make_xben_from_assignments(&assignments, BenVariant::TwoDelta);
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader.map(|r| r.unwrap().0).collect();
    assert_eq!(results.len(), 50);
    for (i, r) in results.iter().enumerate() {
        if i % 2 == 0 {
            assert_eq!(r, &a);
        } else {
            assert_eq!(r, &b);
        }
    }
}

#[test]
fn xz_twodelta_large_assignment_roundtrip() {
    let n = 500;
    let a1: Vec<u16> = (0..n).map(|i| if i < n / 2 { 1 } else { 2 }).collect();
    let a2: Vec<u16> = (0..n).map(|i| if i < n / 2 { 2 } else { 1 }).collect();
    let assignments = vec![a1.clone(), a2.clone(), a1.clone()];
    let xben = make_xben_from_assignments(&assignments, BenVariant::TwoDelta);
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader.map(|r| r.unwrap().0).collect();
    assert_eq!(results, assignments);
}

#[test]
fn xz_twodelta_chunk_boundary_roundtrip() {
    use crate::io::writer::BenStreamWriter;
    use xz2::write::XzEncoder;

    let anchor = vec![1u16, 2, 1, 2];
    let delta = vec![2u16, 1, 2, 1];

    let mut xben = Vec::new();
    {
        let encoder = XzEncoder::new(&mut xben, 1);
        let mut writer =
            BenStreamWriter::for_xben_with_encoder(encoder, BenVariant::TwoDelta, Some(3)).unwrap();
        writer.write_assignment(anchor.clone()).unwrap();
        for _ in 0..10 {
            writer.write_assignment(delta.clone()).unwrap();
            writer.write_assignment(anchor.clone()).unwrap();
        }
        writer.finish().unwrap();
    }

    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader.map(|r| r.unwrap().0).collect();
    assert_eq!(results.len(), 21);
    assert_eq!(results[0], anchor);
    for (i, sample) in results.iter().enumerate().skip(1) {
        if i % 2 == 1 {
            assert_eq!(*sample, delta);
        } else {
            assert_eq!(*sample, anchor);
        }
    }
}

#[test]
fn xz_twodelta_repeated_delta_in_chunk_roundtrip() {
    use crate::io::writer::BenStreamWriter;
    use xz2::write::XzEncoder;

    let anchor = vec![1u16, 1, 2, 2];
    let delta = vec![2u16, 1, 2, 2];

    let mut xben = Vec::new();
    {
        let encoder = XzEncoder::new(&mut xben, 1);
        let mut writer =
            BenStreamWriter::for_xben_with_encoder(encoder, BenVariant::TwoDelta, Some(100))
                .unwrap();
        writer.write_assignment(anchor.clone()).unwrap();
        writer.write_assignment(delta.clone()).unwrap();
        writer.write_assignment(delta.clone()).unwrap();
        writer.write_assignment(delta.clone()).unwrap();
        writer.finish().unwrap();
    }

    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader.map(|r| r.unwrap()).collect();
    let total: usize = results.iter().map(|(_, c)| *c as usize).sum();
    assert_eq!(total, 4);
    assert_eq!(results[0].0, anchor);
    for r in &results[1..] {
        assert_eq!(r.0, delta);
    }
}

// ── translate_ben_twodelta_to_xben ────────────────────────────────────

#[test]
fn translate_ben_twodelta_to_xben_roundtrip() {
    use crate::codec::decode::decode_xben_to_jsonl;
    use crate::codec::encode::encode_ben_to_xben;
    use crate::io::writer::BenStreamWriter;
    use std::io::BufReader;

    let a0 = vec![1u16, 2, 1, 2];
    let a1 = vec![1u16, 1, 2, 2];
    let a2 = vec![2u16, 1, 2, 1];
    let assignments = vec![a0.clone(), a1.clone(), a2.clone()];

    let mut ben = Vec::new();
    {
        let mut w = BenStreamWriter::for_ben(&mut ben, BenVariant::TwoDelta).unwrap();
        for a in &assignments {
            w.write_assignment(a.clone()).unwrap();
        }
    }

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

    let output_str = String::from_utf8(jsonl).unwrap();
    let lines: Vec<&str> = output_str.trim().split('\n').collect();
    assert_eq!(lines.len(), 3);

    for (i, (line, expected)) in lines.iter().zip(assignments.iter()).enumerate() {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        let assign: Vec<u16> = v["assignment"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_u64().unwrap() as u16)
            .collect();
        assert_eq!(&assign, expected, "mismatch at sample {}", i + 1);
    }
}

#[test]
fn translate_ben_twodelta_to_xben_with_repetitions() {
    use crate::codec::encode::encode_ben_to_xben;
    use crate::io::writer::BenStreamWriter;
    use std::io::BufReader;

    let anchor = vec![1u16, 2, 1, 2];
    let delta = vec![2u16, 1, 2, 1];
    let assignments = vec![
        anchor.clone(),
        anchor.clone(),
        anchor.clone(),
        delta.clone(),
        delta.clone(),
    ];

    let mut ben = Vec::new();
    {
        let mut w = BenStreamWriter::for_ben(&mut ben, BenVariant::TwoDelta).unwrap();
        for a in &assignments {
            w.write_assignment(a.clone()).unwrap();
        }
    }

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

    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader.map(|r| r.unwrap()).collect();
    let total: usize = results.iter().map(|(_, c)| *c as usize).sum();
    assert_eq!(total, 5);
}

#[test]
fn translate_ben_twodelta_to_xben_many_deltas() {
    use crate::codec::encode::encode_ben_to_xben;
    use crate::io::writer::BenStreamWriter;
    use std::io::BufReader;

    let a = vec![1u16, 1, 2, 2];
    let b = vec![2u16, 2, 1, 1];
    let assignments: Vec<_> = (0..20)
        .map(|i| if i % 2 == 0 { a.clone() } else { b.clone() })
        .collect();

    let mut ben = Vec::new();
    {
        let mut w = BenStreamWriter::for_ben(&mut ben, BenVariant::TwoDelta).unwrap();
        for a in &assignments {
            w.write_assignment(a.clone()).unwrap();
        }
    }

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

    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader.map(|r| r.unwrap().0).collect();
    assert_eq!(results, assignments);
}

// ── count_samples_from_frame_iter ─────────────────────────────────────

#[test]
fn count_samples_from_frame_iter_basic() {
    use crate::codec::encode::encode_jsonl_to_ben;
    use crate::io::reader::subsample::{
        build_frame_iter_from_reader, count_samples_from_frame_iter,
    };

    let jsonl = r#"{"assignment":[1,2],"sample":1}
{"assignment":[3,4],"sample":2}
{"assignment":[5,6],"sample":3}
"#;
    let mut ben = Vec::new();
    encode_jsonl_to_ben(jsonl.as_bytes(), &mut ben, BenVariant::Standard).unwrap();

    let iter = build_frame_iter_from_reader(Cursor::new(ben), BenWireFormat::Ben).unwrap();
    assert_eq!(count_samples_from_frame_iter(iter).unwrap(), 3);
}

#[test]
fn count_samples_from_frame_iter_xben() {
    use crate::io::reader::subsample::{
        build_frame_iter_from_reader, count_samples_from_frame_iter,
    };

    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[2,2,1,1],"sample":2}
"#;
    let xben = make_xben(jsonl, BenVariant::Standard);
    let iter = build_frame_iter_from_reader(Cursor::new(xben), BenWireFormat::XBen).unwrap();
    assert_eq!(count_samples_from_frame_iter(iter).unwrap(), 2);
}

#[test]
fn count_samples_from_frame_iter_mkv() {
    use crate::codec::encode::encode_jsonl_to_ben;
    use crate::io::reader::subsample::{
        build_frame_iter_from_reader, count_samples_from_frame_iter,
    };

    let jsonl = r#"{"assignment":[1,2],"sample":1}
{"assignment":[1,2],"sample":2}
{"assignment":[3,4],"sample":3}
"#;
    let mut ben = Vec::new();
    encode_jsonl_to_ben(jsonl.as_bytes(), &mut ben, BenVariant::MkvChain).unwrap();

    let iter = build_frame_iter_from_reader(Cursor::new(ben), BenWireFormat::Ben).unwrap();
    assert_eq!(count_samples_from_frame_iter(iter).unwrap(), 3);
}

// ── BenStreamReader tests ─────────────────────────────────────────────────

#[test]
fn assignment_reader_standard_roundtrip() {
    use crate::codec::encode::encode_jsonl_to_ben;
    use crate::io::reader::BenStreamReader;

    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[3,3,4,4],"sample":2}
"#;
    let mut ben = Vec::new();
    encode_jsonl_to_ben(jsonl.as_bytes(), &mut ben, BenVariant::Standard).unwrap();

    let reader = BenStreamReader::from_ben(ben.as_slice()).unwrap();
    assert_eq!(reader.variant(), BenVariant::Standard);
    let results: Vec<_> = reader.map(|r| r.unwrap().0).collect();
    assert_eq!(results, vec![vec![1, 1, 2, 2], vec![3, 3, 4, 4]]);
}

#[test]
fn assignment_reader_mkv_roundtrip() {
    use crate::codec::encode::encode_jsonl_to_ben;
    use crate::io::reader::BenStreamReader;

    let jsonl = r#"{"assignment":[1,2,3],"sample":1}
{"assignment":[1,2,3],"sample":2}
{"assignment":[4,5,6],"sample":3}
"#;
    let mut ben = Vec::new();
    encode_jsonl_to_ben(jsonl.as_bytes(), &mut ben, BenVariant::MkvChain).unwrap();

    let reader = BenStreamReader::from_ben(ben.as_slice()).unwrap();
    assert_eq!(reader.variant(), BenVariant::MkvChain);
    let results: Vec<_> = reader.map(|r| r.unwrap()).collect();
    // MkvChain collapses: first frame count=2, second count=1
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].0, vec![1, 2, 3]);
    assert_eq!(results[0].1, 2);
    assert_eq!(results[1].0, vec![4, 5, 6]);
    assert_eq!(results[1].1, 1);
}

#[test]
fn assignment_reader_twodelta_roundtrip() {
    use crate::io::reader::BenStreamReader;
    use crate::io::writer::BenStreamWriter;

    let assignments = vec![vec![1u16, 1, 2, 2], vec![2, 1, 2, 2], vec![2, 2, 2, 2]];

    let mut ben = Vec::new();
    {
        let mut writer = BenStreamWriter::for_ben(&mut ben, BenVariant::TwoDelta).unwrap();
        for a in &assignments {
            writer.write_assignment(a.clone()).unwrap();
        }
    }

    let reader = BenStreamReader::from_ben(ben.as_slice()).unwrap();
    assert_eq!(reader.variant(), BenVariant::TwoDelta);
    let results: Vec<_> = reader.map(|r| r.unwrap().0).collect();
    assert_eq!(results, assignments);
}

#[test]
fn assignment_reader_count_samples() {
    use crate::codec::encode::encode_jsonl_to_ben;
    use crate::io::reader::BenStreamReader;

    let jsonl = r#"{"assignment":[1,2],"sample":1}
{"assignment":[3,4],"sample":2}
{"assignment":[5,6],"sample":3}
"#;
    let mut ben = Vec::new();
    encode_jsonl_to_ben(jsonl.as_bytes(), &mut ben, BenVariant::Standard).unwrap();

    let reader = BenStreamReader::from_ben(ben.as_slice()).unwrap();
    assert_eq!(reader.count_samples().unwrap(), 3);
}

#[test]
fn assignment_reader_write_all_jsonl() {
    use crate::codec::encode::encode_jsonl_to_ben;
    use crate::io::reader::BenStreamReader;

    let jsonl = r#"{"assignment":[10,20],"sample":1}
{"assignment":[30,40],"sample":2}
"#;
    let mut ben = Vec::new();
    encode_jsonl_to_ben(jsonl.as_bytes(), &mut ben, BenVariant::Standard).unwrap();

    let mut reader = BenStreamReader::from_ben(ben.as_slice()).unwrap();
    let mut output = Vec::new();
    reader.write_all_jsonl(&mut output).unwrap();
    let output_str = String::from_utf8(output).unwrap();
    let lines: Vec<&str> = output_str.trim().split('\n').collect();
    assert_eq!(lines.len(), 2);
    let v1: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(v1["assignment"], serde_json::json!([10, 20]));
    let v2: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(v2["assignment"], serde_json::json!([30, 40]));
}

// ── Zero-count frame errors in BenStreamReader ──────────────────────────

#[test]
fn xz_reader_standard_zero_count_frame_errors() {
    use xz2::write::XzEncoder;

    let mut xben = Vec::new();
    {
        let mut encoder = XzEncoder::new(&mut xben, 1);
        // Write banner
        encoder.write_all(b"STANDARD BEN FILE").unwrap();
        // Write a ben32 frame: one RLE pair (value=1, count=3) + zero terminator
        let frame: &[u8] = &[
            0, 1, 0, 3, // (value=1, count=3)
            0, 0, 0, 0, // zero terminator
        ];
        encoder.write_all(frame).unwrap();
        encoder.finish().unwrap();
    }

    // Manually patch: for Standard, there's no count field after the terminator. Zero-count only
    // fires for MkvChain where the count is explicit. So test MkvChain zero-count instead.
    let mut xben_mkv = Vec::new();
    {
        let mut encoder = XzEncoder::new(&mut xben_mkv, 1);
        encoder.write_all(b"MKVCHAIN BEN FILE").unwrap();
        let frame: &[u8] = &[
            0, 1, 0, 3, // (value=1, count=3)
            0, 0, 0, 0, // zero terminator
            0, 0, // count = 0  <-- triggers zero_count_frame_error
        ];
        encoder.write_all(frame).unwrap();
        encoder.finish().unwrap();
    }

    let reader = BenStreamReader::from_xben(Cursor::new(xben_mkv)).unwrap();
    let err = reader.into_iter().next().unwrap().unwrap_err();
    assert!(err.to_string().contains("zero"));
}

#[test]
fn xz_reader_twodelta_unknown_frame_tag_errors() {
    use xz2::write::XzEncoder;

    let mut xben = Vec::new();
    {
        let mut encoder = XzEncoder::new(&mut xben, 1);
        encoder.write_all(b"TWODELTA BEN FILE").unwrap();
        // Write a byte with unknown tag (0xFF)
        encoder.write_all(&[0xFF]).unwrap();
        encoder.finish().unwrap();
    }

    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let err = reader.into_iter().next().unwrap().unwrap_err();
    assert!(err.to_string().contains("0xff") || err.to_string().contains("unknown"));
}

#[test]
fn xz_reader_truncated_stream_errors() {
    use xz2::write::XzEncoder;

    let mut xben = Vec::new();
    {
        let mut encoder = XzEncoder::new(&mut xben, 1);
        encoder.write_all(b"STANDARD BEN FILE").unwrap();
        // Write a partial ben32 frame (no zero terminator)
        encoder.write_all(&[0, 1, 0, 3]).unwrap();
        encoder.finish().unwrap();
    }

    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let err = reader.into_iter().next().unwrap().unwrap_err();
    assert!(err.to_string().contains("truncated") || err.to_string().contains("Truncated"));
}

// ── Subsample Every branch: first > hi ─────────────────────────────────────

#[test]
fn subsample_every_first_past_hi() {
    // 4 samples, step=10, offset=5: first selected = 5, but only 4 samples exist → the `first > hi`
    // branch fires for every frame.
    let jsonl = concat!(
        "{\"assignment\":[1,2],\"sample\":1}\n",
        "{\"assignment\":[3,4],\"sample\":2}\n",
        "{\"assignment\":[5,6],\"sample\":3}\n",
        "{\"assignment\":[7,8],\"sample\":4}\n",
    );
    let xben = make_xben(jsonl, BenVariant::Standard);
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let sub = reader.into_subsample_every(10, 5);
    let results: Vec<_> = sub.map(|r| r.unwrap()).collect();
    assert!(results.is_empty());
}

// ── MkvChain extract with count>1 mid-block sample ─────────────────────────

#[test]
fn extract_assignment_ben_mkv_mid_block() {
    use crate::codec::encode::encode_jsonl_to_ben;
    use crate::ops::extract::extract_assignment_ben;

    let jsonl = concat!(
        "{\"assignment\":[1,2,3],\"sample\":1}\n",
        "{\"assignment\":[1,2,3],\"sample\":2}\n",
        "{\"assignment\":[1,2,3],\"sample\":3}\n",
        "{\"assignment\":[4,5,6],\"sample\":4}\n",
    );

    let mut ben = Vec::new();
    encode_jsonl_to_ben(
        jsonl.as_bytes(),
        std::io::BufWriter::new(&mut ben),
        BenVariant::MkvChain,
    )
    .unwrap();

    // Sample 2 is in the middle of the first MkvChain block (count=3)
    let result = extract_assignment_ben(ben.as_slice(), 2).unwrap();
    assert_eq!(result, vec![1, 2, 3]);
}

#[test]
fn xz_reader_twodelta_full_frame_zero_count_errors() {
    use xz2::write::XzEncoder;

    let mut xben = Vec::new();
    {
        let mut encoder = XzEncoder::new(&mut xben, 1);
        encoder.write_all(b"TWODELTA BEN FILE").unwrap();

        // Full frame with count=0
        encoder.write_all(&[0u8]).unwrap(); // tag=0
        encoder.write_all(&1u32.to_be_bytes()).unwrap(); // 1 run
        encoder.write_all(&1u16.to_be_bytes()).unwrap(); // value=1
        encoder.write_all(&2u16.to_be_bytes()).unwrap(); // len=2
        encoder.write_all(&0u16.to_be_bytes()).unwrap(); // count=0

        encoder.finish().unwrap();
    }

    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let err = reader.into_iter().next().unwrap().unwrap_err();
    assert!(err.to_string().contains("zero"));
}

#[test]
fn xz_reader_twodelta_chunk_zero_count_errors() {
    use xz2::write::XzEncoder;

    let mut xben = Vec::new();
    {
        let mut encoder = XzEncoder::new(&mut xben, 1);
        encoder.write_all(b"TWODELTA BEN FILE").unwrap();

        // Full frame (tag=0): anchor [1,2]
        encoder.write_all(&[0u8]).unwrap();
        encoder.write_all(&2u32.to_be_bytes()).unwrap();
        encoder.write_all(&1u16.to_be_bytes()).unwrap();
        encoder.write_all(&1u16.to_be_bytes()).unwrap();
        encoder.write_all(&2u16.to_be_bytes()).unwrap();
        encoder.write_all(&1u16.to_be_bytes()).unwrap();
        encoder.write_all(&1u16.to_be_bytes()).unwrap(); // count=1

        // Chunk (tag=2) with 1 frame, count=0
        encoder.write_all(&[2u8]).unwrap(); // tag=2
        encoder.write_all(&1u32.to_be_bytes()).unwrap(); // n_frames=1
                                                         // Pair channel: (2,1)
        encoder.write_all(&2u16.to_be_bytes()).unwrap();
        encoder.write_all(&1u16.to_be_bytes()).unwrap();
        // Count channel: 0
        encoder.write_all(&0u16.to_be_bytes()).unwrap();
        // Run-count channel: 1 run
        encoder.write_all(&1u32.to_be_bytes()).unwrap();
        // Run-length data: 2
        encoder.write_all(&2u16.to_be_bytes()).unwrap();

        encoder.finish().unwrap();
    }

    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader.collect();
    assert_eq!(results.len(), 2); // anchor + chunk frame
    assert!(results[0].is_ok());
    assert!(results[1]
        .as_ref()
        .unwrap_err()
        .to_string()
        .contains("zero"));
}

// ── Subsample with indices that skip past frame boundaries ──────────

#[test]
fn subsample_indices_skip_past_lo() {
    // MkvChain stream where first frame has count=5 but we only want indices [7,8]. This forces the
    // Indices selection to skip past `lo` (line 160-161 in subsample.rs).
    let jsonl = concat!(
        "{\"assignment\":[1,2,3],\"sample\":1}\n",
        "{\"assignment\":[1,2,3],\"sample\":2}\n",
        "{\"assignment\":[1,2,3],\"sample\":3}\n",
        "{\"assignment\":[1,2,3],\"sample\":4}\n",
        "{\"assignment\":[1,2,3],\"sample\":5}\n",
        "{\"assignment\":[4,5,6],\"sample\":6}\n",
        "{\"assignment\":[4,5,6],\"sample\":7}\n",
        "{\"assignment\":[4,5,6],\"sample\":8}\n",
    );
    let xben = make_xben(jsonl, BenVariant::MkvChain);
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader
        .into_subsample_by_indices(vec![7, 8])
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(results.len(), 1); // one frame covering both
    assert_eq!(results[0].0, vec![4, 5, 6]);
    assert_eq!(results[0].1, 2);
}

// ── Subsample indices with zero (below 1-based lo) ──────────────────

#[test]
fn subsample_indices_with_zero_skips_past_lo() {
    let assignments = vec![vec![1u16, 2], vec![3, 4], vec![5, 6]];
    let xben = make_xben_from_assignments(&assignments, BenVariant::Standard);
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    // Index 0 is below the 1-based lo boundary, exercises the `next < lo` skip.
    let results: Vec<_> = reader
        .into_subsample_by_indices(vec![0, 2])
        .map(|r| r.unwrap().0)
        .collect();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0], vec![3, 4]);
}

// ── BenStreamFrameReader for MkvChain zero-count ─────────────────

#[test]
fn xz_frame_reader_mkv_zero_count_errors() {
    use xz2::write::XzEncoder;

    let mut xben = Vec::new();
    {
        let mut encoder = XzEncoder::new(&mut xben, 1);
        encoder.write_all(b"MKVCHAIN BEN FILE").unwrap();
        let frame: &[u8] = &[
            0, 1, 0, 3, // (value=1, count=3)
            0, 0, 0, 0, // zero terminator
            0, 0, // count = 0
        ];
        encoder.write_all(frame).unwrap();
        encoder.finish().unwrap();
    }

    let reader = BenStreamFrameReader::from_xben(Cursor::new(xben)).unwrap();
    let err = reader.into_iter().next().unwrap().unwrap_err();
    assert!(err.to_string().contains("zero"));
}

// ── BenStreamReader TwoDelta truncated stream ─────────────────────

#[test]
fn xz_reader_twodelta_truncated_stream_errors() {
    use xz2::write::XzEncoder;

    let mut xben = Vec::new();
    {
        let mut encoder = XzEncoder::new(&mut xben, 1);
        encoder.write_all(b"TWODELTA BEN FILE").unwrap();
        // Write a full tag + partial run count (not enough bytes for a complete frame)
        encoder.write_all(&[0u8, 0, 0]).unwrap();
        encoder.finish().unwrap();
    }

    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let err = reader.into_iter().next().unwrap().unwrap_err();
    assert!(
        err.to_string().contains("truncated") || err.to_string().contains("Truncated"),
        "got: {}",
        err
    );
}

// ── Legacy TwoDelta delta without anchor (NoAnchorFrame) ────────────

#[test]
fn xz_reader_twodelta_tag1_rejected_as_unknown() {
    use xz2::write::XzEncoder;

    let mut xben = Vec::new();
    {
        let mut encoder = XzEncoder::new(&mut xben, 1);
        encoder.write_all(b"TWODELTA BEN FILE").unwrap();

        // Full frame (tag=0) anchor so the stream is valid up to this point.
        encoder.write_all(&[0u8]).unwrap();
        encoder.write_all(&1u32.to_be_bytes()).unwrap(); // 1 run
        encoder.write_all(&1u16.to_be_bytes()).unwrap(); // value=1
        encoder.write_all(&2u16.to_be_bytes()).unwrap(); // len=2
        encoder.write_all(&1u16.to_be_bytes()).unwrap(); // count=1

        // Tag 1 (removed legacy delta) should now be rejected as unknown.
        encoder.write_all(&[1u8]).unwrap();
        // Enough trailing bytes so the reader can attempt to parse.
        encoder.write_all(&[0u8; 20]).unwrap();

        encoder.finish().unwrap();
    }

    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let mut iter = reader.into_iter();
    let _first = iter.next().unwrap().unwrap(); // consume the valid full frame
    let err = iter.next().unwrap().unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("unknown") || err.to_string().contains("tag"),
        "expected unknown-tag error, got: {}",
        err
    );
}

// ── Chunk delta without anchor (NoAnchorFrame via chunk queue) ───────

#[test]
fn xz_reader_twodelta_chunk_delta_without_anchor_errors() {
    use xz2::write::XzEncoder;

    let mut xben = Vec::new();
    {
        let mut encoder = XzEncoder::new(&mut xben, 1);
        encoder.write_all(b"TWODELTA BEN FILE").unwrap();

        // Write a chunk (tag=2) with 1 delta frame but no preceding full frame.
        encoder.write_all(&[2u8]).unwrap(); // tag=2
        encoder.write_all(&1u32.to_be_bytes()).unwrap(); // n_frames=1
        encoder.write_all(&1u16.to_be_bytes()).unwrap(); // pair.0=1
        encoder.write_all(&2u16.to_be_bytes()).unwrap(); // pair.1=2
        encoder.write_all(&1u16.to_be_bytes()).unwrap(); // count=1
        encoder.write_all(&1u32.to_be_bytes()).unwrap(); // 1 run
        encoder.write_all(&2u16.to_be_bytes()).unwrap(); // rl=2

        encoder.finish().unwrap();
    }

    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let err = reader.into_iter().next().unwrap().unwrap_err();
    assert!(
        err.to_string().contains("full-assignment") || err.to_string().contains("anchor"),
        "got: {}",
        err
    );
}

// ── for_each_assignment with stream error ────────────────────────────

#[test]
fn xz_reader_for_each_assignment_stream_error() {
    use xz2::write::XzEncoder;

    // Create a valid TwoDelta stream that ends with truncated data
    let mut xben = Vec::new();
    {
        let mut encoder = XzEncoder::new(&mut xben, 1);
        encoder.write_all(b"TWODELTA BEN FILE").unwrap();

        // Valid full frame
        encoder.write_all(&[0u8]).unwrap();
        encoder.write_all(&1u32.to_be_bytes()).unwrap();
        encoder.write_all(&1u16.to_be_bytes()).unwrap();
        encoder.write_all(&2u16.to_be_bytes()).unwrap();
        encoder.write_all(&1u16.to_be_bytes()).unwrap(); // count=1

        // Truncated second frame
        encoder.write_all(&[0u8, 0]).unwrap();
        encoder.finish().unwrap();
    }

    let mut reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let mut count = 0usize;
    let result = reader.for_each_assignment(|_assignment, _cnt| {
        count += 1;
        Ok(true)
    });
    // Should get the first assignment but error on the truncated second frame
    assert!(count >= 1);
    assert!(result.is_err());
}

// ── BenStreamFrameReader truncated TwoDelta ──────────────────────

#[test]
fn xz_frame_reader_twodelta_truncated_errors() {
    use xz2::write::XzEncoder;

    let mut xben = Vec::new();
    {
        let mut encoder = XzEncoder::new(&mut xben, 1);
        encoder.write_all(b"STANDARD BEN FILE").unwrap();
        // Partial ben32 frame — no zero terminator, triggers truncated error
        encoder.write_all(&[0, 1, 0, 3]).unwrap();
        encoder.finish().unwrap();
    }

    let reader = BenStreamFrameReader::from_xben(Cursor::new(xben)).unwrap();
    let err = reader.into_iter().next().unwrap().unwrap_err();
    assert!(
        err.to_string().contains("truncated") || err.to_string().contains("Truncated"),
        "got: {}",
        err
    );
}

// ── Standard/MkvChain frame decode error ─────────────────────────────

#[test]
fn xz_reader_standard_corrupt_frame_errors() {
    use xz2::write::XzEncoder;

    // Write a valid-looking ben32 frame structure but with corrupted content that
    // decode_xben_frame_to_assignment can't parse
    let mut xben = Vec::new();
    {
        let mut encoder = XzEncoder::new(&mut xben, 1);
        encoder.write_all(b"STANDARD BEN FILE").unwrap();
        // Write 4 bytes followed by zero terminator — the frame decodes to a single run (value=255,
        // count=255). This should actually be valid. Instead, write a completely empty frame
        // (just the zero terminator).
        encoder.write_all(&[0, 0, 0, 0]).unwrap(); // just zero terminator (no runs)
        encoder.finish().unwrap();
    }

    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader.collect();
    // An empty frame (no RLE pairs before terminator) yields an empty assignment
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].as_ref().unwrap().0, Vec::<u16>::new());
}

// ── SubsampleFrameDecoder: zero-count frame error ───────────────────

#[test]
fn subsample_decoder_zero_count_frame_errors() {
    // A frame iterator that yields a frame with count=0 should produce an InvalidData error from
    // SubsampleFrameDecoder::next().
    let frame = DecodeFrame::XBen(
        vec![0, 1, 0, 2, 0, 0, 0, 0], // valid ben32: [1,2] + zero terminator
        BenVariant::Standard,
    );
    let items: Vec<io::Result<(DecodeFrame, u16)>> = vec![Ok((frame, 0))];
    let mut decoder =
        SubsampleFrameDecoder::new(items.into_iter(), Selection::Range { start: 1, end: 10 });
    let err = decoder.next().unwrap().unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert!(err.to_string().contains("zero"), "got: {}", err);
}

// ── BenStreamFrameReader: TwoDelta into_frames ───────────────────

#[test]
fn xz_frame_reader_twodelta_into_frames() {
    // Verify that into_frames() works for TwoDelta streams. The frame reader takes the TwoDelta
    // short-circuit path (re-encoding decoded assignments back to ben32).
    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[2,1,2,2],"sample":2}
"#;
    let xben = make_xben(jsonl, BenVariant::TwoDelta);
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let frames: Vec<_> = reader.into_frames().map(|r| r.unwrap()).collect();
    assert_eq!(frames.len(), 2);
    // Each frame is (ben32_bytes, count); counts should be 1
    assert_eq!(frames[0].1, 1);
    assert_eq!(frames[1].1, 1);
}

// ── BenStreamReader: count_samples helper ────────────────────────

#[test]
fn xz_reader_count_samples() {
    let jsonl = r#"{"assignment":[1,2,3],"sample":1}
{"assignment":[4,5,6],"sample":2}
{"assignment":[7,8,9],"sample":3}
"#;
    let xben = make_xben(jsonl, BenVariant::Standard);
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    assert_eq!(reader.count_samples().unwrap(), 3);
}

// ── BenStreamReader: write_all_jsonl ─────────────────────────────

#[test]
fn xz_reader_write_all_jsonl_standard_roundtrip() {
    let jsonl_in = r#"{"assignment":[1,2,3],"sample":1}
{"assignment":[4,5,6],"sample":2}
"#;
    let xben = make_xben(jsonl_in, BenVariant::Standard);
    let mut reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let mut output = Vec::new();
    reader.write_all_jsonl(&mut output).unwrap();
    let text = String::from_utf8(output).unwrap();
    assert_eq!(text.lines().count(), 2);
    assert!(text.contains("\"assignment\":[1,2,3]"));
    assert!(text.contains("\"assignment\":[4,5,6]"));
}

// ── BenStreamReader: TwoDelta error propagation in RawBenFrameIter ──────────

#[test]
fn raw_frame_iter_propagates_twodelta_decode_error() {
    use crate::io::reader::BenStreamReader;
    use crate::io::writer::BenStreamWriter;

    // Build a minimal TwoDelta BEN file with two samples.
    let mut ben: Vec<u8> = Vec::new();
    {
        let mut writer = BenStreamWriter::for_ben(&mut ben, BenVariant::TwoDelta).unwrap();
        writer.write_assignment(vec![1u16, 1, 2, 2]).unwrap();
        writer.write_assignment(vec![2u16, 1, 2, 1]).unwrap();
    }

    // Locate the TwoDelta delta frame start by parsing the anchor (snapshot/MkvChain) frame header:
    // banner(17) + snapshot_tag(1) + max_val_bits(1) + max_len_bits(1) + n_bytes(4 BE) +
    // payload(n_bytes) + count(2) = anchor_end (which points at the next frame's tag byte).
    let banner_len = 17usize;
    let anchor_start = banner_len + 1; // skip the snapshot tag
    let n_bytes =
        u32::from_be_bytes(ben[anchor_start + 2..anchor_start + 6].try_into().unwrap()) as usize;
    let anchor_end = anchor_start + 6 + n_bytes + 2;

    // The delta frame: delta_tag(1) + pair_a(2) + pair_b(2) + max_len_bits(1) + ... Set
    // max_len_bits to 0, which triggers InvalidData during decoding.
    ben[anchor_end + 5] = 0;

    let reader = BenStreamReader::from_ben(Cursor::new(ben)).unwrap();
    let mut iter = reader.into_frames();
    iter.next().unwrap().unwrap(); // anchor frame OK
    let err = iter.next().unwrap().unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

// ── BenStreamReader: zero-count frame errors ────────────────────────────────

/// Build a minimal MkvChain BEN stream whose first frame has count == 0.
fn make_mkvchain_zero_count_frame() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"MKVCHAIN BEN FILE"); // 17-byte banner
    bytes.push(1u8); // max_val_bit_count
    bytes.push(1u8); // max_len_bit_count
    bytes.extend_from_slice(&1u32.to_be_bytes()); // n_bytes = 1
    bytes.push(0xFFu8); // 1 payload byte
    bytes.extend_from_slice(&0u16.to_be_bytes()); // count = 0
    bytes
}

#[test]
fn assignment_reader_count_samples_rejects_zero_count_frame() {
    use crate::io::reader::BenStreamReader;
    let data = make_mkvchain_zero_count_frame();
    let reader = BenStreamReader::from_ben(Cursor::new(data)).unwrap();
    let err = reader.count_samples().unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn assignment_reader_for_each_rejects_zero_count_frame() {
    use crate::io::reader::BenStreamReader;
    let data = make_mkvchain_zero_count_frame();
    let mut reader = BenStreamReader::from_ben(Cursor::new(data)).unwrap();
    let err = reader.for_each_assignment(|_, _| Ok(true)).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn raw_frame_iter_rejects_zero_count_mkv_frame() {
    use crate::io::reader::BenStreamReader;
    let data = make_mkvchain_zero_count_frame();
    let reader = BenStreamReader::from_ben(Cursor::new(data)).unwrap();
    let err = reader
        .into_frames()
        .next()
        .expect("should yield one item")
        .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}
