use crate::io::reader::XZAssignmentReader;
use crate::io::writer::XZAssignmentWriter;
use crate::BenVariant;
use std::io::Cursor;
use xz2::write::XzEncoder;

fn roundtrip_xben(assignments: &[Vec<u16>], variant: BenVariant) -> Vec<Vec<u16>> {
    let mut xben = Vec::new();
    {
        let encoder = XzEncoder::new(&mut xben, 1);
        let mut writer = XZAssignmentWriter::new(encoder, variant).unwrap();
        for a in assignments {
            writer.write_assignment(a.clone()).unwrap();
        }
    }
    let reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
    reader.map(|r| r.unwrap().0).collect()
}

fn roundtrip_xben_counts(assignments: &[Vec<u16>], variant: BenVariant) -> Vec<(Vec<u16>, u16)> {
    let mut xben = Vec::new();
    {
        let encoder = XzEncoder::new(&mut xben, 1);
        let mut writer = XZAssignmentWriter::new(encoder, variant).unwrap();
        for a in assignments {
            writer.write_assignment(a.clone()).unwrap();
        }
    }
    let reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
    reader.map(|r| r.unwrap()).collect()
}

// ── Standard variant roundtrips ───────────────────────────────────────

#[test]
fn writer_standard_basic_roundtrip() {
    let assignments = vec![vec![1u16, 2, 3], vec![4, 5, 6]];
    assert_eq!(roundtrip_xben(&assignments, BenVariant::Standard), assignments);
}

#[test]
fn writer_standard_single_element_assignments() {
    let assignments = vec![vec![42u16], vec![99]];
    assert_eq!(roundtrip_xben(&assignments, BenVariant::Standard), assignments);
}

// ── MkvChain variant roundtrips ───────────────────────────────────────

#[test]
fn writer_mkv_deduplication() {
    let a = vec![1u16, 2, 3];
    let assignments = vec![a.clone(), a.clone(), a.clone(), vec![4, 5, 6]];
    let results = roundtrip_xben_counts(&assignments, BenVariant::MkvChain);
    let total: usize = results.iter().map(|(_, c)| *c as usize).sum();
    assert_eq!(total, 4);
}

// ── TwoDelta basic roundtrips ─────────────────────────────────────────

#[test]
fn writer_twodelta_basic_roundtrip() {
    let assignments = vec![vec![1u16, 1, 2, 2], vec![2, 1, 2, 2], vec![2, 2, 2, 2]];
    assert_eq!(roundtrip_xben(&assignments, BenVariant::TwoDelta), assignments);
}

#[test]
fn writer_twodelta_anchor_only() {
    let assignments = vec![vec![1u16, 2, 3, 4]];
    assert_eq!(roundtrip_xben(&assignments, BenVariant::TwoDelta), assignments);
}

#[test]
fn writer_twodelta_repeated_anchor() {
    let a = vec![1u16, 2, 1, 2];
    let assignments: Vec<_> = (0..5).map(|_| a.clone()).collect();
    let results = roundtrip_xben_counts(&assignments, BenVariant::TwoDelta);
    let total: usize = results.iter().map(|(_, c)| *c as usize).sum();
    assert_eq!(total, 5);
    for (v, _) in &results {
        assert_eq!(v, &a);
    }
}

#[test]
fn writer_twodelta_repeated_delta() {
    let anchor = vec![1u16, 1, 2, 2];
    let delta = vec![2u16, 1, 2, 2];
    let assignments = vec![
        anchor.clone(),
        delta.clone(),
        delta.clone(),
        delta.clone(),
    ];
    let results = roundtrip_xben_counts(&assignments, BenVariant::TwoDelta);
    let total: usize = results.iter().map(|(_, c)| *c as usize).sum();
    assert_eq!(total, 4);
}

// ── TwoDelta chunk size edge cases ────────────────────────────────────

#[test]
fn writer_twodelta_chunk_size_1() {
    let anchor = vec![1u16, 1, 2, 2];
    let delta = vec![2u16, 2, 1, 1];
    let assignments: Vec<_> = (0..10)
        .map(|i| if i % 2 == 0 { anchor.clone() } else { delta.clone() })
        .collect();

    let mut xben = Vec::new();
    {
        let encoder = XzEncoder::new(&mut xben, 1);
        let mut writer = XZAssignmentWriter::new(encoder, BenVariant::TwoDelta)
            .unwrap()
            .with_chunk_size(1);
        for a in &assignments {
            writer.write_assignment(a.clone()).unwrap();
        }
    }
    let reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader.map(|r| r.unwrap().0).collect();
    assert_eq!(results, assignments);
}

#[test]
fn writer_twodelta_chunk_size_larger_than_stream() {
    let a = vec![1u16, 1, 2, 2];
    let b = vec![2u16, 2, 1, 1];
    let assignments = vec![a.clone(), b.clone(), a.clone()];

    let mut xben = Vec::new();
    {
        let encoder = XzEncoder::new(&mut xben, 1);
        let mut writer = XZAssignmentWriter::new(encoder, BenVariant::TwoDelta)
            .unwrap()
            .with_chunk_size(1_000_000);
        for a in &assignments {
            writer.write_assignment(a.clone()).unwrap();
        }
    }
    let reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader.map(|r| r.unwrap().0).collect();
    assert_eq!(results, assignments);
}

// ── TwoDelta with all-identical assignments (twodelta_repeat_buffered_frame) ──

#[test]
fn writer_twodelta_all_identical_values() {
    let assign = vec![3u16; 8];
    let assignments: Vec<_> = (0..5).map(|_| assign.clone()).collect();
    let results = roundtrip_xben_counts(&assignments, BenVariant::TwoDelta);
    let total: usize = results.iter().map(|(_, c)| *c as usize).sum();
    assert_eq!(total, 5);
    for (v, _) in &results {
        assert_eq!(v, &assign);
    }
}

#[test]
fn writer_twodelta_u16_max_value_in_assignment() {
    let assign = vec![u16::MAX; 4];
    let assignments: Vec<_> = (0..3).map(|_| assign.clone()).collect();
    let results = roundtrip_xben_counts(&assignments, BenVariant::TwoDelta);
    let total: usize = results.iter().map(|(_, c)| *c as usize).sum();
    assert_eq!(total, 3);
    for (v, _) in &results {
        assert_eq!(v, &assign);
    }
}

// ── TwoDelta write_json_value ─────────────────────────────────────────

#[test]
fn writer_twodelta_write_json_value() {
    use serde_json::json;

    let mut xben = Vec::new();
    {
        let encoder = XzEncoder::new(&mut xben, 1);
        let mut writer = XZAssignmentWriter::new(encoder, BenVariant::TwoDelta).unwrap();
        writer
            .write_json_value(json!({"assignment": [1, 2, 1, 2]}))
            .unwrap();
        writer
            .write_json_value(json!({"assignment": [2, 1, 2, 1]}))
            .unwrap();
    }
    let reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader.map(|r| r.unwrap().0).collect();
    assert_eq!(results, vec![vec![1u16, 2, 1, 2], vec![2, 1, 2, 1]]);
}

// ── TwoDelta finish idempotency ───────────────────────────────────────

#[test]
fn writer_finish_is_idempotent() {
    let mut xben = Vec::new();
    {
        let encoder = XzEncoder::new(&mut xben, 1);
        let mut writer = XZAssignmentWriter::new(encoder, BenVariant::TwoDelta).unwrap();
        writer
            .write_assignment(vec![1u16, 2, 3, 4])
            .unwrap();
        writer.finish().unwrap();
        writer.finish().unwrap();
    }
    let reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader.map(|r| r.unwrap().0).collect();
    assert_eq!(results, vec![vec![1u16, 2, 3, 4]]);
}

// ── write_ben_file translation ────────────────────────────────────────

#[test]
fn writer_write_ben_file_standard_roundtrip() {
    use crate::codec::encode::encode_jsonl_to_ben;
    use std::io::BufReader;

    let jsonl = r#"{"assignment":[1,2,3],"sample":1}
{"assignment":[4,5,6],"sample":2}
"#;
    let mut ben = Vec::new();
    encode_jsonl_to_ben(jsonl.as_bytes(), &mut ben, BenVariant::Standard).unwrap();

    let mut xben = Vec::new();
    {
        let encoder = XzEncoder::new(&mut xben, 1);
        let mut writer = XZAssignmentWriter::new(encoder, BenVariant::Standard).unwrap();
        writer.write_ben_file(BufReader::new(ben.as_slice())).unwrap();
        writer.finish().unwrap();
    }

    let reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader.map(|r| r.unwrap().0).collect();
    assert_eq!(results, vec![vec![1u16, 2, 3], vec![4, 5, 6]]);
}

#[test]
fn writer_write_ben_file_mkv_roundtrip() {
    use crate::codec::encode::encode_jsonl_to_ben;
    use std::io::BufReader;

    let jsonl = r#"{"assignment":[1,2,3],"sample":1}
{"assignment":[1,2,3],"sample":2}
{"assignment":[4,5,6],"sample":3}
"#;
    let mut ben = Vec::new();
    encode_jsonl_to_ben(jsonl.as_bytes(), &mut ben, BenVariant::MkvChain).unwrap();

    let mut xben = Vec::new();
    {
        let encoder = XzEncoder::new(&mut xben, 1);
        let mut writer = XZAssignmentWriter::new(encoder, BenVariant::MkvChain).unwrap();
        writer.write_ben_file(BufReader::new(ben.as_slice())).unwrap();
        writer.finish().unwrap();
    }

    let reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader.map(|r| r.unwrap()).collect();
    let total: usize = results.iter().map(|(_, c)| *c as usize).sum();
    assert_eq!(total, 3);
}

#[test]
fn writer_write_ben_file_twodelta_roundtrip() {
    use crate::io::writer::AssignmentWriter;
    use std::io::BufReader;

    let assignments = vec![
        vec![1u16, 2, 1, 2],
        vec![1, 1, 2, 2],
        vec![2, 1, 2, 1],
    ];

    let mut ben = Vec::new();
    {
        let mut w = AssignmentWriter::new(&mut ben, BenVariant::TwoDelta).unwrap();
        for a in &assignments {
            w.write_assignment(a.clone()).unwrap();
        }
    }

    let mut xben = Vec::new();
    {
        let encoder = XzEncoder::new(&mut xben, 1);
        let mut writer = XZAssignmentWriter::new(encoder, BenVariant::TwoDelta).unwrap();
        writer.write_ben_file(BufReader::new(ben.as_slice())).unwrap();
        writer.finish().unwrap();
    }

    let reader = XZAssignmentReader::new(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader.map(|r| r.unwrap().0).collect();
    assert_eq!(results, assignments);
}

#[test]
fn writer_write_ben_file_twodelta_rejects_bannerless() {
    use std::io::BufReader;

    let mut xben = Vec::new();
    let encoder = XzEncoder::new(&mut xben, 1);
    let mut writer = XZAssignmentWriter::new(encoder, BenVariant::TwoDelta).unwrap();
    let no_banner = vec![0u8; 50];
    let err = writer
        .write_ben_file(BufReader::new(no_banner.as_slice()))
        .unwrap_err();
    assert!(
        err.to_string().contains("banner")
            || err.to_string().contains("TwoDelta")
            || err.kind() == std::io::ErrorKind::InvalidData
    );
}

// ── Large-scale stress test ───────────────────────────────────────────

#[test]
fn writer_twodelta_stress_many_unique_deltas() {
    let n = 200;
    let base: Vec<u16> = (0..20).map(|i| if i < 10 { 1 } else { 2 }).collect();
    let flipped: Vec<u16> = (0..20).map(|i| if i < 10 { 2 } else { 1 }).collect();
    let mut assignments = vec![base.clone()];
    for i in 0..n {
        if i % 2 == 0 {
            assignments.push(flipped.clone());
        } else {
            assignments.push(base.clone());
        }
    }

    let results = roundtrip_xben(&assignments, BenVariant::TwoDelta);
    assert_eq!(results, assignments);
}
