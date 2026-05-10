use crate::io::reader::BenStreamReader;
use crate::io::writer::BenStreamWriter;
use crate::BenVariant;
use std::io::Cursor;
use xz2::write::XzEncoder;

/// Build a `BenStreamWriter` over an explicit single-thread XZ encoder so
/// the resulting xben byte stream is deterministic and small.
fn build_xben_writer<'a>(
    out: &'a mut Vec<u8>,
    variant: BenVariant,
    chunk_size: Option<usize>,
) -> BenStreamWriter<&'a mut Vec<u8>> {
    let encoder = XzEncoder::new(out, 1);
    BenStreamWriter::for_xben_with_encoder(encoder, variant, chunk_size).unwrap()
}

fn roundtrip_xben(assignments: &[Vec<u16>], variant: BenVariant) -> Vec<Vec<u16>> {
    let mut xben = Vec::new();
    {
        let mut writer = build_xben_writer(&mut xben, variant, None);
        for a in assignments {
            writer.write_assignment(a.clone()).unwrap();
        }
        writer.finish().unwrap();
    }
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    reader.map(|r| r.unwrap().0).collect()
}

fn roundtrip_xben_counts(assignments: &[Vec<u16>], variant: BenVariant) -> Vec<(Vec<u16>, u16)> {
    let mut xben = Vec::new();
    {
        let mut writer = build_xben_writer(&mut xben, variant, None);
        for a in assignments {
            writer.write_assignment(a.clone()).unwrap();
        }
        writer.finish().unwrap();
    }
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    reader.map(|r| r.unwrap()).collect()
}

// ── Standard variant roundtrips ───────────────────────────────────────

#[test]
fn writer_standard_basic_roundtrip() {
    let assignments = vec![vec![1u16, 2, 3], vec![4, 5, 6]];
    assert_eq!(
        roundtrip_xben(&assignments, BenVariant::Standard),
        assignments
    );
}

#[test]
fn writer_standard_single_element_assignments() {
    let assignments = vec![vec![42u16], vec![99]];
    assert_eq!(
        roundtrip_xben(&assignments, BenVariant::Standard),
        assignments
    );
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
    assert_eq!(
        roundtrip_xben(&assignments, BenVariant::TwoDelta),
        assignments
    );
}

#[test]
fn writer_twodelta_anchor_only() {
    let assignments = vec![vec![1u16, 2, 3, 4]];
    assert_eq!(
        roundtrip_xben(&assignments, BenVariant::TwoDelta),
        assignments
    );
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
    let assignments = vec![anchor.clone(), delta.clone(), delta.clone(), delta.clone()];
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
        .map(|i| {
            if i % 2 == 0 {
                anchor.clone()
            } else {
                delta.clone()
            }
        })
        .collect();

    let mut xben = Vec::new();
    {
        let mut writer = build_xben_writer(&mut xben, BenVariant::TwoDelta, Some(1));
        for a in &assignments {
            writer.write_assignment(a.clone()).unwrap();
        }
        writer.finish().unwrap();
    }
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
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
        let mut writer = build_xben_writer(&mut xben, BenVariant::TwoDelta, Some(1_000_000));
        for a in &assignments {
            writer.write_assignment(a.clone()).unwrap();
        }
        writer.finish().unwrap();
    }
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
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

// ── BEN BenStreamWriter TwoDelta repeat frame ────────────────────────

#[test]
fn ben_writer_twodelta_repeat_frame_via_u16max_overflow() {
    // Assignment with 3 distinct values exercises the `continue` skip path
    // inside `twodelta_repeat_frame` for values outside the picked pair.
    let assign = vec![1u16, 2, 3, 1, 2];
    let n = u16::MAX as usize + 2; // 65537: triggers overflow → repeat frame

    let mut ben = Vec::new();
    {
        let mut w = BenStreamWriter::for_ben(&mut ben, BenVariant::TwoDelta).unwrap();
        for _ in 0..n {
            w.write_assignment(assign.clone()).unwrap();
        }
        w.finish().unwrap();
    }

    let reader = BenStreamReader::from_ben(ben.as_slice()).unwrap();
    let total: usize = reader.map(|r| r.unwrap().1 as usize).sum();
    assert_eq!(total, n);
}

// ── TwoDelta write_json_value ─────────────────────────────────────────

#[test]
fn writer_twodelta_write_json_value() {
    use serde_json::json;

    let mut xben = Vec::new();
    {
        let mut writer = build_xben_writer(&mut xben, BenVariant::TwoDelta, None);
        writer
            .write_json_value(json!({"assignment": [1, 2, 1, 2]}))
            .unwrap();
        writer
            .write_json_value(json!({"assignment": [2, 1, 2, 1]}))
            .unwrap();
        writer.finish().unwrap();
    }
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader.map(|r| r.unwrap().0).collect();
    assert_eq!(results, vec![vec![1u16, 2, 1, 2], vec![2, 1, 2, 1]]);
}

// ── TwoDelta finish idempotency ───────────────────────────────────────

#[test]
fn writer_finish_is_idempotent() {
    let mut xben = Vec::new();
    {
        let mut writer = build_xben_writer(&mut xben, BenVariant::TwoDelta, None);
        writer.write_assignment(vec![1u16, 2, 3, 4]).unwrap();
        writer.finish().unwrap();
        writer.finish().unwrap();
    }
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader.map(|r| r.unwrap().0).collect();
    assert_eq!(results, vec![vec![1u16, 2, 3, 4]]);
}

// ── ingest_ben_stream translation ─────────────────────────────────────

#[test]
fn writer_ingest_ben_stream_standard_roundtrip() {
    use crate::codec::encode::encode_jsonl_to_ben;
    use std::io::BufReader;

    let jsonl = r#"{"assignment":[1,2,3],"sample":1}
{"assignment":[4,5,6],"sample":2}
"#;
    let mut ben = Vec::new();
    encode_jsonl_to_ben(jsonl.as_bytes(), &mut ben, BenVariant::Standard).unwrap();

    let mut xben = Vec::new();
    {
        let mut writer = build_xben_writer(&mut xben, BenVariant::Standard, None);
        writer
            .ingest_ben_stream(BufReader::new(ben.as_slice()))
            .unwrap();
        writer.finish().unwrap();
    }

    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader.map(|r| r.unwrap().0).collect();
    assert_eq!(results, vec![vec![1u16, 2, 3], vec![4, 5, 6]]);
}

#[test]
fn writer_ingest_ben_stream_mkv_roundtrip() {
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
        let mut writer = build_xben_writer(&mut xben, BenVariant::MkvChain, None);
        writer
            .ingest_ben_stream(BufReader::new(ben.as_slice()))
            .unwrap();
        writer.finish().unwrap();
    }

    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader.map(|r| r.unwrap()).collect();
    let total: usize = results.iter().map(|(_, c)| *c as usize).sum();
    assert_eq!(total, 3);
}

#[test]
fn writer_ingest_ben_stream_twodelta_roundtrip() {
    use std::io::BufReader;

    let assignments = vec![vec![1u16, 2, 1, 2], vec![1, 1, 2, 2], vec![2, 1, 2, 1]];

    let mut ben = Vec::new();
    {
        let mut w = BenStreamWriter::for_ben(&mut ben, BenVariant::TwoDelta).unwrap();
        for a in &assignments {
            w.write_assignment(a.clone()).unwrap();
        }
        w.finish().unwrap();
    }

    let mut xben = Vec::new();
    {
        let mut writer = build_xben_writer(&mut xben, BenVariant::TwoDelta, None);
        writer
            .ingest_ben_stream(BufReader::new(ben.as_slice()))
            .unwrap();
        writer.finish().unwrap();
    }

    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader.map(|r| r.unwrap().0).collect();
    assert_eq!(results, assignments);
}

#[test]
fn writer_ingest_ben_stream_twodelta_rejects_bannerless() {
    use std::io::BufReader;

    let mut xben = Vec::new();
    let mut writer = build_xben_writer(&mut xben, BenVariant::TwoDelta, None);
    let no_banner = vec![0u8; 50];
    let err = writer
        .ingest_ben_stream(BufReader::new(no_banner.as_slice()))
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

// ── TwoDelta u16::MAX count overflow paths ───────────────────────────

#[test]
fn writer_twodelta_anchor_count_overflow_u16max() {
    // Use 3 distinct values to exercise the `continue` skip in
    // twodelta_repeat_buffered_frame for values outside the picked pair.
    let assign = vec![1u16, 2, 3, 1, 2];
    let n = u16::MAX as usize + 2; // 65537 — triggers the overflow branch

    let mut xben = Vec::new();
    {
        let mut writer = build_xben_writer(&mut xben, BenVariant::TwoDelta, None);
        for _ in 0..n {
            writer.write_assignment(assign.clone()).unwrap();
        }
        writer.finish().unwrap();
    }
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let total: usize = reader.map(|r| r.unwrap().1 as usize).sum();
    assert_eq!(total, n);
}

#[test]
fn writer_twodelta_delta_count_overflow_u16max() {
    let anchor = vec![1u16, 1, 2, 2];
    let delta = vec![2u16, 1, 2, 2];
    let n_delta = u16::MAX as usize + 1; // 65536 identical deltas

    let mut xben = Vec::new();
    {
        let mut writer = build_xben_writer(&mut xben, BenVariant::TwoDelta, Some(n_delta + 1));
        writer.write_assignment(anchor.clone()).unwrap();
        for _ in 0..n_delta {
            writer.write_assignment(delta.clone()).unwrap();
        }
        writer.finish().unwrap();
    }
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader.map(|r| r.unwrap()).collect();
    let total: usize = results.iter().map(|(_, c)| *c as usize).sum();
    assert_eq!(total, n_delta + 1);
}

// ── TwoDelta translate via ingest_ben_stream with chunk flush ────────

#[test]
fn writer_translate_ben_twodelta_chunk_flush() {
    use std::io::BufReader;

    let a = vec![1u16, 1, 2, 2];
    let b = vec![2u16, 2, 1, 1];
    let assignments: Vec<_> = (0..30)
        .map(|i| if i % 2 == 0 { a.clone() } else { b.clone() })
        .collect();

    let mut ben = Vec::new();
    {
        let mut w = BenStreamWriter::for_ben(&mut ben, BenVariant::TwoDelta).unwrap();
        for a in &assignments {
            w.write_assignment(a.clone()).unwrap();
        }
        w.finish().unwrap();
    }

    let mut xben = Vec::new();
    {
        let mut writer = build_xben_writer(&mut xben, BenVariant::TwoDelta, Some(5));
        writer
            .ingest_ben_stream(BufReader::new(ben.as_slice()))
            .unwrap();
        writer.finish().unwrap();
    }

    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader.map(|r| r.unwrap().0).collect();
    assert_eq!(results, assignments);
}

// ── TwoDelta encoding error propagation ─────────────────────────────

#[test]
fn xz_writer_twodelta_too_many_ids_propagates_on_write() {
    // Writing a third assignment that changes 3 distinct IDs errors at the
    // TwoDelta encode boundary.
    let anchor = vec![1u16, 1, 2, 2];
    let invalid = vec![2u16, 3, 1, 3]; // 3 distinct changing ids
    let mut xben = Vec::new();
    let mut writer = build_xben_writer(&mut xben, BenVariant::TwoDelta, None);
    writer.write_assignment(anchor).unwrap();
    let err = writer.write_assignment(invalid).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
}

// ── MkvChain u16::MAX overflow ───────────────────────────────────────

#[test]
fn writer_mkv_count_overflow_u16max() {
    let assign = vec![1u16, 2, 3];
    let n = u16::MAX as usize + 2; // overflow

    let mut xben = Vec::new();
    {
        let mut writer = build_xben_writer(&mut xben, BenVariant::MkvChain, None);
        for _ in 0..n {
            writer.write_assignment(assign.clone()).unwrap();
        }
        writer.finish().unwrap();
    }
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let total: usize = reader.map(|r| r.unwrap().1 as usize).sum();
    assert_eq!(total, n);
}

// ── Private helper coverage (relocated from sibling source files) ─────

#[test]
fn twodelta_repeat_frame_run_exceeds_u16_max_errors() {
    use super::stream_writer::test_helpers::{twodelta_repeat_buffered_frame, twodelta_repeat_frame};
    use std::io;

    // All-identical-value assignment with 65536 elements: the pair-position
    // run reaches u16::MAX and the encoder must error.
    let assign = vec![1u16; 65536];
    let err = twodelta_repeat_frame(&assign, 1).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    assert!(err.to_string().contains("u16::MAX"));

    // The XBEN buffered variant must error the same way.
    let result = twodelta_repeat_buffered_frame(&assign, 1);
    let err = result.err().expect("expected error");
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    assert!(err.to_string().contains("u16::MAX"));
}

#[test]
fn translate_twodelta_non_eof_read_error_propagates() {
    use std::io::{self, Read};

    // ingest_ben_stream in TwoDelta mode calls translate_ben_twodelta_to_xben.
    // After reading the anchor frame it loops reading delta frames; a
    // non-EOF error on pair_a (first u16 read in the loop) must propagate.
    let mut xben = Vec::new();
    let mut writer = build_xben_writer(&mut xben, BenVariant::TwoDelta, None);

    // Banner (17 bytes) + minimal anchor frame:
    //   max_val_bits=1, max_len_bits=1, n_bytes=0 (no payload), count=1
    let mut input: Vec<u8> = b"TWODELTA BEN FILE".to_vec();
    input.extend_from_slice(&[0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01]);

    struct ErrorAfterEof;
    impl Read for ErrorAfterEof {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "broken"))
        }
    }

    let reader = std::io::BufReader::new(input.as_slice().chain(ErrorAfterEof));
    let err = writer.ingest_ben_stream(reader).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
}

// ── BEN write_frame mixing with write_assignment ────────────────────

#[test]
fn ben_write_frame_then_write_assignment_mixed_standard() {
    let a = vec![1u16, 2, 3];
    let b = vec![4u16, 5, 6];

    let mut ben = Vec::new();
    {
        let mut w = BenStreamWriter::for_ben(&mut ben, BenVariant::Standard).unwrap();
        w.write_assignment(a.clone()).unwrap();
        w.write_assignment(a.clone()).unwrap();
        w.write_frame(b.clone(), 3).unwrap();
        w.write_assignment(a.clone()).unwrap();
        w.finish().unwrap();
    }
    let reader = BenStreamReader::from_ben(ben.as_slice()).unwrap();
    let total: usize = reader.map(|r| r.unwrap().1 as usize).sum();
    assert_eq!(total, 2 + 3 + 1);
}

#[test]
fn ben_write_frame_then_write_assignment_mixed_mkv() {
    let a = vec![1u16, 2, 3];
    let b = vec![4u16, 5, 6];

    let mut ben = Vec::new();
    {
        let mut w = BenStreamWriter::for_ben(&mut ben, BenVariant::MkvChain).unwrap();
        w.write_assignment(a.clone()).unwrap();
        w.write_assignment(a.clone()).unwrap();
        w.write_frame(b.clone(), 3).unwrap();
        w.write_assignment(a.clone()).unwrap();
        w.finish().unwrap();
    }
    let reader = BenStreamReader::from_ben(ben.as_slice()).unwrap();
    let records: Vec<_> = reader.map(|r| r.unwrap()).collect();
    let total: usize = records.iter().map(|(_, c)| *c as usize).sum();
    assert_eq!(total, 6);
    // Expect three counted frames: (a, 2), (b, 3), (a, 1).
    assert_eq!(records.len(), 3);
    assert_eq!(records[0], (a.clone(), 2));
    assert_eq!(records[1], (b.clone(), 3));
    assert_eq!(records[2], (a.clone(), 1));
}

#[test]
fn ben_write_frame_zero_count_is_noop_and_does_not_flush() {
    // write_assignment(a); write_frame(b, 0); write_assignment(a) should
    // act like two adjacent write_assignment(a) calls — no inserted
    // frame boundary.
    let a = vec![1u16, 2, 3];
    let b = vec![4u16, 5, 6];

    let mut ben = Vec::new();
    {
        let mut w = BenStreamWriter::for_ben(&mut ben, BenVariant::MkvChain).unwrap();
        w.write_assignment(a.clone()).unwrap();
        w.write_frame(b.clone(), 0).unwrap();
        w.write_assignment(a.clone()).unwrap();
        w.finish().unwrap();
    }
    let reader = BenStreamReader::from_ben(ben.as_slice()).unwrap();
    let records: Vec<_> = reader.map(|r| r.unwrap()).collect();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0], (a.clone(), 2));
}

#[test]
fn ben_twodelta_first_call_write_frame_emits_anchor_with_count() {
    let a = vec![1u16, 2, 1, 2];
    let mut ben = Vec::new();
    {
        let mut w = BenStreamWriter::for_ben(&mut ben, BenVariant::TwoDelta).unwrap();
        w.write_frame(a.clone(), 3).unwrap();
        w.finish().unwrap();
    }
    let reader = BenStreamReader::from_ben(ben.as_slice()).unwrap();
    let records: Vec<_> = reader.map(|r| r.unwrap()).collect();
    let total: usize = records.iter().map(|(_, c)| *c as usize).sum();
    assert_eq!(total, 3);
    for (v, _) in &records {
        assert_eq!(v, &a);
    }
}

#[test]
fn ben_twodelta_write_frame_updates_previous_assignment_for_next_delta() {
    let a = vec![1u16, 2, 1, 2];
    let b = vec![2u16, 1, 2, 1];
    let mut ben = Vec::new();
    {
        let mut w = BenStreamWriter::for_ben(&mut ben, BenVariant::TwoDelta).unwrap();
        w.write_frame(a.clone(), 3).unwrap();
        w.write_assignment(b.clone()).unwrap();
        w.finish().unwrap();
    }
    // Round-trip must reproduce the inputs, which proves the delta against
    // the emitted anchor was encoded correctly.
    let mut reader = BenStreamReader::from_ben(ben.as_slice()).unwrap();
    let mut samples: Vec<Vec<u16>> = Vec::new();
    reader
        .for_each_assignment(|assignment, count| {
            for _ in 0..count {
                samples.push(assignment.to_vec());
            }
            Ok(true)
        })
        .unwrap();
    assert_eq!(samples.len(), 4);
    for v in &samples[..3] {
        assert_eq!(v, &a);
    }
    assert_eq!(&samples[3], &b);
}

#[test]
fn write_frame_on_xben_returns_invalid_input() {
    let mut xben = Vec::new();
    let mut writer = build_xben_writer(&mut xben, BenVariant::MkvChain, None);
    let err = writer.write_frame(vec![1u16, 2, 3], 1).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

// ── finish + body-state guards ───────────────────────────────────────

#[test]
fn xben_finish_emits_complete_stream_before_drop() {
    let mut xben = Vec::new();
    let mut writer = build_xben_writer(&mut xben, BenVariant::MkvChain, None);
    writer.write_assignment(vec![1u16, 2, 3]).unwrap();
    writer.finish().unwrap();
    // Repeated finish after success returns Ok.
    writer.finish().unwrap();
    drop(writer);

    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader.map(|r| r.unwrap().0).collect();
    assert_eq!(results, vec![vec![1u16, 2, 3]]);
}

#[test]
fn write_methods_after_finish_return_invalid_input() {
    let mut ben = Vec::new();
    let mut w = BenStreamWriter::for_ben(&mut ben, BenVariant::MkvChain).unwrap();
    w.write_assignment(vec![1u16, 2, 3]).unwrap();
    w.finish().unwrap();
    let err = w.write_assignment(vec![4u16, 5, 6]).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    let err = w.write_frame(vec![4u16, 5, 6], 1).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    let err = w
        .write_json_value(serde_json::json!({"assignment": [4, 5, 6]}))
        .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

#[test]
fn write_frame_after_finish_with_zero_count_still_returns_invalid_input() {
    // Pin guard ordering: finished/wrong-mode checks happen before the
    // zero-count no-op.
    let mut ben = Vec::new();
    let mut w = BenStreamWriter::for_ben(&mut ben, BenVariant::MkvChain).unwrap();
    w.finish().unwrap();
    let err = w.write_frame(vec![1u16, 2, 3], 0).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

#[test]
fn write_frame_zero_count_on_xben_returns_invalid_input() {
    // Guard ordering: wrong-mode check happens before zero-count no-op.
    let mut xben = Vec::new();
    let mut w = build_xben_writer(&mut xben, BenVariant::MkvChain, None);
    let err = w.write_frame(vec![1u16, 2, 3], 0).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

#[test]
fn ingest_ben_stream_lifecycle_terminal_for_sample_writes() {
    use crate::codec::encode::encode_jsonl_to_ben;
    use std::io::BufReader;

    let jsonl = r#"{"assignment":[1,2,3],"sample":1}
"#;
    let mut ben = Vec::new();
    encode_jsonl_to_ben(jsonl.as_bytes(), &mut ben, BenVariant::Standard).unwrap();

    let mut xben = Vec::new();
    let mut writer = build_xben_writer(&mut xben, BenVariant::Standard, None);
    writer
        .ingest_ben_stream(BufReader::new(ben.as_slice()))
        .unwrap();

    // Subsequent sample writes must be rejected; finish() still succeeds.
    let err = writer.write_assignment(vec![1u16, 2, 3]).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    let err = writer
        .write_json_value(serde_json::json!({"assignment": [1, 2, 3]}))
        .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    let err = writer
        .ingest_ben_stream(BufReader::new(b"".as_slice()))
        .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);

    writer.finish().unwrap();
}

#[test]
fn ingest_ben_stream_rejects_non_fresh_writer() {
    use std::io::BufReader;

    let mut xben = Vec::new();
    let mut writer = build_xben_writer(&mut xben, BenVariant::Standard, None);
    writer.write_assignment(vec![1u16, 2, 3]).unwrap();
    let err = writer
        .ingest_ben_stream(BufReader::new(b"".as_slice()))
        .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

#[test]
fn ingest_ben_stream_rejects_ben_mode_writer() {
    use std::io::BufReader;

    let mut ben = Vec::new();
    let mut w = BenStreamWriter::for_ben(&mut ben, BenVariant::Standard).unwrap();
    let err = w
        .ingest_ben_stream(BufReader::new(b"".as_slice()))
        .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

// ── Fail-hard gate: poisoning on encode pipeline error ───────────────

#[test]
fn ben_writer_failed_state_after_underlying_writer_error() {
    // The banner write happens during construction; constructor failure
    // bypasses WriterState entirely. To exercise the post-construction
    // poisoning path we wrap a buffer that accepts only the 17 banner
    // bytes and errors on subsequent writes.
    struct FailAfterN {
        buf: Vec<u8>,
        n: usize,
    }
    impl std::io::Write for FailAfterN {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            if self.buf.len() + b.len() > self.n {
                return Err(std::io::Error::other("boom"));
            }
            self.buf.extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let mut w = BenStreamWriter::for_ben(
        FailAfterN {
            buf: Vec::new(),
            n: 17,
        },
        BenVariant::MkvChain,
    )
    .unwrap();
    // First call buffers the assignment as pending; no IO yet.
    w.write_assignment(vec![1u16, 2, 3]).unwrap();
    // Second call with a different assignment triggers a flush, which
    // must fail and poison the writer.
    let err = w.write_assignment(vec![4u16, 5, 6]).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::Other);
    let err = w.write_assignment(vec![1u16, 2, 3]).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    let err = w.write_frame(vec![1u16, 2, 3], 1).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    let err = w.finish().unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}
