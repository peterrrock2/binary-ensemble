use crate::codec::encode::encode_jsonl_to_xben;
use crate::io::reader::{XZAssignmentReader, XZAssignmentFrameReader};
use crate::io::writer::XZAssignmentWriter;
use crate::BenVariant;
use std::io::Cursor;
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
    )
    .unwrap();
    xben
}

/// Build a minimal XBEN stream using XZAssignmentWriter directly.
fn make_xben_from_assignments(assignments: &[Vec<u16>], variant: BenVariant) -> Vec<u8> {
    let mut xben = Vec::new();
    {
        let encoder = XzEncoder::new(&mut xben, 1);
        let mut writer = XZAssignmentWriter::new(encoder, variant).unwrap();
        for a in assignments {
            writer.write_assignment(a.clone()).unwrap();
        }
    }
    xben
}

// ── XZAssignmentReader ──────────────────────────────────────────────────────

#[test]
fn xz_reader_standard_iterator() {
    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[2,2,1,1],"sample":2}
"#;
    let xben = make_xben(jsonl, BenVariant::Standard);
    let reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
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
    let reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
    assert_eq!(reader.variant(), BenVariant::MkvChain);
    let results: Vec<_> = reader.collect();
    // MkvChain collapses identical consecutive assignments
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].as_ref().unwrap().1, 2); // count=2
    assert_eq!(results[1].as_ref().unwrap().1, 1); // count=1
}

#[test]
fn xz_reader_twodelta_iterator() {
    let assignments = vec![
        vec![1u16, 1, 2, 2],
        vec![2, 1, 2, 2],
        vec![2, 2, 2, 2],
    ];
    let xben = make_xben_from_assignments(&assignments, BenVariant::TwoDelta);
    let reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
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
    let reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
    assert_eq!(reader.count_samples().unwrap(), 3);
}

#[test]
fn xz_reader_count_samples_mkv() {
    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[1,1,2,2],"sample":2}
{"assignment":[2,2,1,1],"sample":3}
"#;
    let xben = make_xben(jsonl, BenVariant::MkvChain);
    let reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
    assert_eq!(reader.count_samples().unwrap(), 3);
}

#[test]
fn xz_reader_silent_suppresses_output() {
    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
"#;
    let xben = make_xben(jsonl, BenVariant::Standard);
    let reader = XZAssignmentReader::new(Cursor::new(xben))
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
    let mut reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
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
    let mut reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
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
    let mut reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
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
    let mut reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
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
    let reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
    let frames: Vec<_> = reader.into_frames().collect();
    assert_eq!(frames.len(), 2);
    for f in &frames {
        let (bytes, count) = f.as_ref().unwrap();
        assert!(!bytes.is_empty());
        assert_eq!(*count, 1);
    }
}

#[test]
fn xz_reader_into_frames_twodelta() {
    let assignments = vec![
        vec![1u16, 1, 2, 2],
        vec![2, 1, 2, 2],
    ];
    let xben = make_xben_from_assignments(&assignments, BenVariant::TwoDelta);
    let reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
    let frames: Vec<_> = reader.into_frames().collect();
    assert_eq!(frames.len(), 2);
}

#[test]
fn xz_frame_reader_new() {
    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
"#;
    let xben = make_xben(jsonl, BenVariant::Standard);
    let reader = XZAssignmentFrameReader::new(Cursor::new(xben)).unwrap();
    let frames: Vec<_> = reader.collect();
    assert_eq!(frames.len(), 1);
}

#[test]
fn xz_reader_new_rejects_invalid_data() {
    let garbage = vec![0u8; 100];
    let result = XZAssignmentReader::new(Cursor::new(garbage));
    assert!(result.is_err());
}

// ── XZAssignmentReader subsample ────────────────────────────────────────────

#[test]
fn xz_reader_subsample_by_indices() {
    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[2,2,1,1],"sample":2}
{"assignment":[3,3,3,3],"sample":3}
"#;
    let xben = make_xben(jsonl, BenVariant::Standard);
    let reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
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
    let reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
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
    let reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader
        .into_subsample_every(2, 1) // samples 1, 3
        .map(|r| r.unwrap().0)
        .collect();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0], vec![1, 1, 2, 2]);
    assert_eq!(results[1], vec![3, 3, 3, 3]);
}

// ── XZAssignmentReader for_each_assignment with silent ──────────────────────

#[test]
fn xz_reader_for_each_assignment_silent() {
    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[2,2,1,1],"sample":2}
"#;
    let xben = make_xben(jsonl, BenVariant::Standard);
    let mut reader = XZAssignmentReader::new(Cursor::new(xben))
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

// ── XZAssignmentReader TwoDelta write_all_jsonl ─────────────────────────────

#[test]
fn xz_reader_write_all_jsonl_twodelta() {
    let assignments = vec![
        vec![1u16, 1, 2, 2],
        vec![2, 1, 2, 2],
        vec![2, 2, 2, 2],
    ];
    let xben = make_xben_from_assignments(&assignments, BenVariant::TwoDelta);
    let mut reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
    let mut output = Vec::new();
    reader.write_all_jsonl(&mut output).unwrap();
    let output_str = String::from_utf8(output).unwrap();
    let lines: Vec<&str> = output_str.trim().split('\n').collect();
    assert_eq!(lines.len(), 3);
}

// ── XZAssignmentReader TwoDelta count_samples ───────────────────────────────

#[test]
fn xz_reader_count_samples_twodelta() {
    let assignments = vec![
        vec![1u16, 1, 2, 2],
        vec![2, 1, 2, 2],
    ];
    let xben = make_xben_from_assignments(&assignments, BenVariant::TwoDelta);
    let reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
    assert_eq!(reader.count_samples().unwrap(), 2);
}
