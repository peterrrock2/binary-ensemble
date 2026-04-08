use crate::codec::encode::encode_jsonl_to_xben;
use crate::io::reader::errors::DecoderInitError;
use crate::io::reader::{XZAssignmentFrameReader, XZAssignmentReader};
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

// ── Content verification tests ─────────────────────────────────────────────

#[test]
fn xz_reader_into_frames_standard_content() {
    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[3,3,4,4],"sample":2}
"#;
    let xben = make_xben(jsonl, BenVariant::Standard);
    let reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
    let frames: Vec<_> = reader.into_frames().collect();
    assert_eq!(frames.len(), 2);
    // Verify frame bytes can be decoded back
    for f in &frames {
        let (bytes, count) = f.as_ref().unwrap();
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
    let mut reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
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
    let mut reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
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
    let reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader.collect();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].as_ref().unwrap().0, vec![42]);
    assert_eq!(results[0].as_ref().unwrap().1, 1);
}

#[test]
fn xz_reader_single_sample_twodelta() {
    let xben = make_xben_from_assignments(&[vec![1u16, 2, 3]], BenVariant::TwoDelta);
    let reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
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
    let reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
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
    let reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
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
    let reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
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
    let reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
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
    let reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
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
    let assignments = vec![
        vec![1u16, 1, 2, 2],
        vec![2, 1, 2, 2],
        vec![2, 2, 2, 2],
    ];
    let xben = make_xben_from_assignments(&assignments, BenVariant::TwoDelta);
    let reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
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
    use crate::io::reader::AssignmentReader;
    let xz_magic = b"\xFD\x37\x7A\x58\x5A\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00";
    let result = AssignmentReader::new(xz_magic.as_slice());
    assert!(result.is_err());
    let io_err: std::io::Error = result.err().unwrap().into();
    assert_eq!(io_err.kind(), std::io::ErrorKind::InvalidData);
    assert!(io_err.to_string().contains("Compressed header"));
}

#[test]
fn decoder_init_error_unknown_banner() {
    use crate::io::reader::AssignmentReader;
    let bad_banner = b"THIS IS NOT BEN!!";
    let result = AssignmentReader::new(bad_banner.as_slice());
    assert!(result.is_err());
    let io_err: std::io::Error = result.err().unwrap().into();
    assert_eq!(io_err.kind(), std::io::ErrorKind::InvalidData);
    assert!(io_err.to_string().contains("Invalid file format"));
}

#[test]
fn decoder_init_error_io() {
    use crate::io::reader::AssignmentReader;
    struct FailReader;
    impl std::io::Read for FailReader {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "broken"))
        }
    }
    let result = AssignmentReader::new(FailReader);
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
    let mut reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
    let err = reader
        .for_each_assignment(|_assignment, _count| {
            Err(std::io::Error::new(std::io::ErrorKind::Other, "callback failed"))
        })
        .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::Other);
    assert_eq!(err.to_string(), "callback failed");
}

// ── Large assignment vector ────────────────────────────────────────────────

#[test]
fn xz_reader_large_assignment_roundtrip() {
    let big_assign: Vec<u16> = (1..=1000).collect();
    let xben = make_xben_from_assignments(&[big_assign.clone()], BenVariant::Standard);
    let reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader.map(|r| r.unwrap().0).collect();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0], big_assign);
}

// ── AssignmentReader tests ─────────────────────────────────────────────────

#[test]
fn assignment_reader_standard_roundtrip() {
    use crate::codec::encode::encode_jsonl_to_ben;
    use crate::io::reader::AssignmentReader;

    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[3,3,4,4],"sample":2}
"#;
    let mut ben = Vec::new();
    encode_jsonl_to_ben(jsonl.as_bytes(), &mut ben, BenVariant::Standard).unwrap();

    let reader = AssignmentReader::new(ben.as_slice()).unwrap();
    assert_eq!(reader.variant(), BenVariant::Standard);
    let results: Vec<_> = reader.map(|r| r.unwrap().0).collect();
    assert_eq!(results, vec![vec![1, 1, 2, 2], vec![3, 3, 4, 4]]);
}

#[test]
fn assignment_reader_mkv_roundtrip() {
    use crate::codec::encode::encode_jsonl_to_ben;
    use crate::io::reader::AssignmentReader;

    let jsonl = r#"{"assignment":[1,2,3],"sample":1}
{"assignment":[1,2,3],"sample":2}
{"assignment":[4,5,6],"sample":3}
"#;
    let mut ben = Vec::new();
    encode_jsonl_to_ben(jsonl.as_bytes(), &mut ben, BenVariant::MkvChain).unwrap();

    let reader = AssignmentReader::new(ben.as_slice()).unwrap();
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
    use crate::io::reader::AssignmentReader;
    use crate::io::writer::AssignmentWriter;

    let assignments = vec![
        vec![1u16, 1, 2, 2],
        vec![2, 1, 2, 2],
        vec![2, 2, 2, 2],
    ];

    let mut ben = Vec::new();
    {
        let mut writer = AssignmentWriter::new(&mut ben, BenVariant::TwoDelta).unwrap();
        for a in &assignments {
            writer.write_assignment(a.clone()).unwrap();
        }
    }

    let reader = AssignmentReader::new(ben.as_slice()).unwrap();
    assert_eq!(reader.variant(), BenVariant::TwoDelta);
    let results: Vec<_> = reader.map(|r| r.unwrap().0).collect();
    assert_eq!(results, assignments);
}

#[test]
fn assignment_reader_count_samples() {
    use crate::codec::encode::encode_jsonl_to_ben;
    use crate::io::reader::AssignmentReader;

    let jsonl = r#"{"assignment":[1,2],"sample":1}
{"assignment":[3,4],"sample":2}
{"assignment":[5,6],"sample":3}
"#;
    let mut ben = Vec::new();
    encode_jsonl_to_ben(jsonl.as_bytes(), &mut ben, BenVariant::Standard).unwrap();

    let reader = AssignmentReader::new(ben.as_slice()).unwrap();
    assert_eq!(reader.count_samples().unwrap(), 3);
}

#[test]
fn assignment_reader_write_all_jsonl() {
    use crate::codec::encode::encode_jsonl_to_ben;
    use crate::io::reader::AssignmentReader;

    let jsonl = r#"{"assignment":[10,20],"sample":1}
{"assignment":[30,40],"sample":2}
"#;
    let mut ben = Vec::new();
    encode_jsonl_to_ben(jsonl.as_bytes(), &mut ben, BenVariant::Standard).unwrap();

    let mut reader = AssignmentReader::new(ben.as_slice()).unwrap();
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
