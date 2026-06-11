use crate::io::reader::BenStreamReader;
use crate::io::writer::BenStreamWriter;
use crate::BenVariant;
use std::io::Cursor;
use xz2::write::XzEncoder;

/// Build a `BenStreamWriter` over an explicit single-thread XZ encoder so the resulting xben byte
/// stream is deterministic and small.
fn build_xben_writer(
    out: &mut Vec<u8>,
    variant: BenVariant,
    chunk_size: Option<usize>,
) -> BenStreamWriter<&mut Vec<u8>> {
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

// ── Zero/one-sample edges ─────────────────────────────────────────────

/// Round-trip an `assignments` list (possibly empty) through a BEN writer and reader for the
/// given variant, asserting the decoded sequence equals the input. Used by the zero/one-sample
/// matrix tests below.
fn assert_ben_round_trip(assignments: &[Vec<u16>], variant: BenVariant) {
    let mut ben = Vec::new();
    {
        let mut writer = BenStreamWriter::for_ben(&mut ben, variant).unwrap();
        for a in assignments {
            writer.write_assignment(a.clone()).unwrap();
        }
        writer.finish().unwrap();
    }
    let reader = BenStreamReader::from_ben(Cursor::new(&ben)).unwrap();
    let decoded: Vec<Vec<u16>> = reader
        .silent(true)
        .flat_map(|r| {
            let (a, c) = r.unwrap();
            std::iter::repeat_n(a, c as usize)
        })
        .collect();
    assert_eq!(
        decoded, assignments,
        "BEN round-trip failed for {variant:?}"
    );
}

/// XBEN counterpart of [`assert_ben_round_trip`].
fn assert_xben_round_trip(assignments: &[Vec<u16>], variant: BenVariant) {
    let mut xben = Vec::new();
    {
        let mut writer = build_xben_writer(&mut xben, variant, None);
        for a in assignments {
            writer.write_assignment(a.clone()).unwrap();
        }
        writer.finish().unwrap();
    }
    let reader = BenStreamReader::from_xben(Cursor::new(&xben)).unwrap();
    let decoded: Vec<Vec<u16>> = reader
        .silent(true)
        .flat_map(|r| {
            let (a, c) = r.unwrap();
            std::iter::repeat_n(a, c as usize)
        })
        .collect();
    assert_eq!(
        decoded, assignments,
        "XBEN round-trip failed for {variant:?}"
    );
}

/// Zero-sample (banner-only) BEN streams round-trip for every variant. Constructed by opening
/// the writer and immediately finishing it without any `write_assignment` calls. Catches stream
/// readers that assume at least one frame follows the banner.
#[test]
fn writer_ben_zero_sample_round_trip_per_variant() {
    for variant in [
        BenVariant::Standard,
        BenVariant::MkvChain,
        BenVariant::TwoDelta,
    ] {
        assert_ben_round_trip(&[], variant);
    }
}

/// Zero-sample XBEN streams. XBEN adds an outer xz frame around the BEN content, so this also
/// covers any reader path that expects at least one BEN frame inside the compressed payload.
#[test]
fn writer_xben_zero_sample_round_trip_per_variant() {
    for variant in [
        BenVariant::Standard,
        BenVariant::MkvChain,
        BenVariant::TwoDelta,
    ] {
        assert_xben_round_trip(&[], variant);
    }
}

/// One-sample BEN streams. Each fixture contains a single first frame; for TwoDelta this is the
/// MkvChain-shaped anchor frame (no delta frames follow, since there's no second sample).
#[test]
fn writer_ben_one_sample_round_trip_per_variant() {
    let assignment = vec![1u16, 1, 2, 2];
    for variant in [
        BenVariant::Standard,
        BenVariant::MkvChain,
        BenVariant::TwoDelta,
    ] {
        assert_ben_round_trip(std::slice::from_ref(&assignment), variant);
    }
}

/// One-sample XBEN streams. Mirrors the BEN matrix above but through the xz-compressed wire
/// format. For TwoDelta this exercises the XBEN columnar-chunk path when only an anchor exists
/// and no chunk has accumulated.
#[test]
fn writer_xben_one_sample_round_trip_per_variant() {
    let assignment = vec![1u16, 1, 2, 2];
    for variant in [
        BenVariant::Standard,
        BenVariant::MkvChain,
        BenVariant::TwoDelta,
    ] {
        assert_xben_round_trip(std::slice::from_ref(&assignment), variant);
    }
}

#[test]
fn writer_twodelta_chunk_boundary_off_by_one_grid() {
    // Off-by-one bugs in the chunked TwoDelta path hide exactly at the boundaries between full
    // chunks and partial trailing chunks: a flush that runs one short of the chunk boundary, a
    // flush that exactly fills it, and a flush that spills one past. Sweep both the first chunk
    // (samples = chunk - 1, chunk, chunk + 1) and the second chunk (samples = 2*chunk - 1,
    // 2*chunk, 2*chunk + 1) for every plausible chunk size, including the default 10_000.
    //
    // Each test generates assignments that strictly alternate between an anchor pattern and a
    // delta pattern so the writer is forced through both the anchor-frame and delta-frame paths;
    // a writer that miscounts chunk boundaries would either drop the final partial chunk, write
    // a stale anchor for the next chunk, or scramble the delta chain.
    let anchor = vec![1u16, 1, 2, 2];
    let delta = vec![2u16, 2, 1, 1];

    for &chunk_size in &[2usize, 7, 64, 10_000] {
        for &n_samples in &[
            chunk_size.saturating_sub(1),
            chunk_size,
            chunk_size + 1,
            2 * chunk_size - 1,
            2 * chunk_size,
            2 * chunk_size + 1,
        ] {
            if n_samples == 0 {
                continue;
            }
            let assignments: Vec<Vec<u16>> = (0..n_samples)
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
                let mut writer =
                    build_xben_writer(&mut xben, BenVariant::TwoDelta, Some(chunk_size));
                for a in &assignments {
                    writer.write_assignment(a.clone()).unwrap();
                }
                writer.finish().unwrap();
            }

            let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
            let decoded: Vec<Vec<u16>> = reader
                .flat_map(|r| {
                    let (a, count) = r.unwrap();
                    std::iter::repeat_n(a, count as usize)
                })
                .collect();
            assert_eq!(
                decoded, assignments,
                "TwoDelta chunk-boundary round-trip failed for chunk_size={chunk_size}, \
                 n_samples={n_samples}"
            );
        }
    }
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
    // Assignment with 3 distinct values exercises the `continue` skip path inside
    // `twodelta_repeat_frame` for values outside the picked pair.
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
    // Use 3 distinct values to exercise the `continue` skip in twodelta_repeat_buffered_frame for
    // values outside the picked pair.
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

// ── TwoDelta >2-district fallback ───────────────────────────────────

#[test]
fn xz_writer_twodelta_too_many_ids_falls_back_to_snapshot() {
    // A transition that changes 3 distinct ids is no longer an error: it emits a full snapshot
    // frame and still round-trips.
    let anchor = vec![1u16, 1, 2, 2];
    let multi = vec![2u16, 3, 1, 3]; // 3 distinct changing ids
    let mut xben = Vec::new();
    {
        let mut writer = build_xben_writer(&mut xben, BenVariant::TwoDelta, None);
        writer.write_assignment(anchor.clone()).unwrap();
        writer.write_assignment(multi.clone()).unwrap();
        writer.finish().unwrap();
    }
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let results: Vec<_> = reader.map(|r| r.unwrap().0).collect();
    assert_eq!(results, vec![anchor, multi]);
}

#[test]
fn xz_writer_twodelta_mixed_snapshot_delta_direct_roundtrip() {
    // The direct XBEN writer (not the BEN→XBEN translate path) must emit a mid-stream full frame
    // for a >2-district transition and rebase later deltas onto it.
    let assignments = vec![
        vec![1u16, 1, 2, 2], // anchor (full)
        vec![1u16, 2, 1, 2], // delta
        vec![3u16, 3, 1, 2], // 3 ids → mid-stream full
        vec![3u16, 3, 2, 1], // delta from the snapshot
    ];
    assert_eq!(
        roundtrip_xben(&assignments, BenVariant::TwoDelta),
        assignments
    );
}

#[test]
fn xz_writer_twodelta_new_district_falls_back_to_snapshot_direct() {
    // A 2-id transition that introduces a district absent from the previous assignment has no mask
    // to delta against, so the direct XBEN writer must emit a snapshot; a later 2-swap among now
    // present ids deltas normally.
    let assignments = vec![
        vec![1u16, 1, 1, 1], // anchor
        vec![1u16, 1, 2, 2], // introduces district 2 → snapshot
        vec![1u16, 2, 1, 2], // delta (both ids present)
    ];
    assert_eq!(
        roundtrip_xben(&assignments, BenVariant::TwoDelta),
        assignments
    );
}

#[test]
fn xz_writer_twodelta_delta_snapshot_repeat_delta_direct() {
    // delta → snapshot → repeat → delta: the repeat increments the deferred full frame's count,
    // and the following delta rebases onto the flushed snapshot.
    let s = vec![3u16, 3, 1, 2];
    let assignments = vec![
        vec![1u16, 1, 2, 2], // anchor
        vec![1u16, 2, 1, 2], // delta
        s.clone(),           // snapshot
        s.clone(),           // repeat of snapshot
        vec![3u16, 3, 2, 1], // delta from the snapshot
    ];
    assert_eq!(
        roundtrip_xben_counts(&assignments, BenVariant::TwoDelta),
        vec![
            (vec![1u16, 1, 2, 2], 1),
            (vec![1u16, 2, 1, 2], 1),
            (s.clone(), 2),
            (vec![3u16, 3, 2, 1], 1),
        ]
    );
}

#[test]
fn xz_writer_twodelta_pending_full_count_overflow_u16max() {
    // A snapshot repeated past u16::MAX flushes the full frame (count == u16::MAX) and emits the
    // overflow repeat as a delta in the following chunk, so the total still round-trips.
    let anchor = vec![1u16, 1, 2, 2];
    let delta = vec![1u16, 2, 1, 2]; // 2-swap from anchor
    let snapshot = vec![3u16, 3, 3, 3]; // 3 ids change → snapshot
    let repeats = u16::MAX as usize + 1; // one past the count ceiling

    let mut xben = Vec::new();
    {
        let mut writer = build_xben_writer(&mut xben, BenVariant::TwoDelta, None);
        writer.write_assignment(anchor.clone()).unwrap();
        writer.write_assignment(delta.clone()).unwrap();
        for _ in 0..repeats {
            writer.write_assignment(snapshot.clone()).unwrap();
        }
        writer.finish().unwrap();
    }
    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let total: usize = reader.map(|r| r.unwrap().1 as usize).sum();
    assert_eq!(total, 2 + repeats);
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
    use super::stream_writer::test_helpers::{
        twodelta_repeat_buffered_frame, twodelta_repeat_frame,
    };
    use std::io;

    // All-identical-value assignment with 65536 elements: the pair-position run reaches u16::MAX
    // and the encoder must error.
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

// ── TwoDelta long-run snapshot fallback ──────────────────────────────
//
// A pair-projected run longer than u16::MAX cannot be expressed in a delta-shaped frame (splitting
// it would require zero-length runs, which readers reject as corruption). The writers fall back to
// snapshot/full frames, whose RLE splits long runs natively. One test per fallback site.

/// Smallest assignment whose single-district body exceeds the u16::MAX run limit when projected
/// onto a repeat/delta pair.
fn long_run_assignment() -> Vec<u16> {
    vec![1u16; u16::MAX as usize + 1]
}

/// Drain a plain-BEN TwoDelta stream, asserting every sample equals `expected` and returning the
/// expanded sample total.
fn drain_ben_expecting(ben: &[u8], expected: &[u16]) -> usize {
    let mut total = 0usize;
    BenStreamReader::from_ben(ben)
        .unwrap()
        .silent(true)
        .for_each_assignment(|a, count| {
            assert_eq!(a, expected, "decoded assignment diverged");
            total += count as usize;
            Ok(true)
        })
        .unwrap();
    total
}

#[test]
fn ben_twodelta_long_run_repeat_falls_back_to_snapshot() {
    let a = long_run_assignment();
    let mut ben = Vec::new();
    {
        let mut writer = BenStreamWriter::for_ben(&mut ben, BenVariant::TwoDelta).unwrap();
        writer.write_frame(a.clone(), 3).unwrap();
        // Repeat of the previous frame: unrepresentable as a repeat delta → snapshot fallback.
        writer.write_frame(a.clone(), 4).unwrap();
        writer.finish().unwrap();
    }
    assert_eq!(drain_ben_expecting(&ben, &a), 7);
}

#[test]
fn ben_twodelta_long_run_delta_falls_back_to_snapshot() {
    // A → B is a clean 2-swap at position 0, but the delta's pair covers every position and B's
    // leading run exceeds u16::MAX → snapshot fallback.
    let mut a = vec![1u16; u16::MAX as usize + 2];
    *a.last_mut().unwrap() = 2;
    let mut b = a.clone();
    b[0] = 2;

    let mut ben = Vec::new();
    {
        let mut writer = BenStreamWriter::for_ben(&mut ben, BenVariant::TwoDelta).unwrap();
        writer.write_frame(a.clone(), 1).unwrap();
        writer.write_frame(b.clone(), 2).unwrap();
        writer.finish().unwrap();
    }

    let mut seen = Vec::new();
    BenStreamReader::from_ben(ben.as_slice())
        .unwrap()
        .silent(true)
        .for_each_assignment(|assignment, count| {
            seen.push((assignment.to_vec(), count));
            Ok(true)
        })
        .unwrap();
    assert_eq!(seen, vec![(a, 1), (b, 2)]);
}

#[test]
fn xben_twodelta_long_run_delta_falls_back_to_full_frame() {
    // Same construction as the plain-BEN delta test, through the XBEN columnar writer.
    let mut a = vec![1u16; u16::MAX as usize + 2];
    *a.last_mut().unwrap() = 2;
    let mut b = a.clone();
    b[0] = 2;

    assert_eq!(
        roundtrip_xben_counts(&[a.clone(), b.clone()], BenVariant::TwoDelta),
        vec![(a, 1), (b, 1)]
    );
}

#[test]
fn xben_twodelta_long_run_repeat_after_chunk_flush_falls_back_to_full_frame() {
    // chunk_size = 1 forces a chunk flush after the A→B delta, so the next repeat of B reaches
    // the classify-Repeat arm with an empty chunk. B's repeat pair (1, 4) projects onto a run
    // longer than u16::MAX → pending-full fallback. The A→B delta itself stays representable
    // because its pair (3, 4) covers only the two tail positions.
    let mut a = vec![1u16; u16::MAX as usize + 3];
    a[u16::MAX as usize + 1] = 3;
    a[u16::MAX as usize + 2] = 4;
    let mut b = a.clone();
    b[u16::MAX as usize + 1] = 4;

    let mut xben = Vec::new();
    {
        let mut writer = build_xben_writer(&mut xben, BenVariant::TwoDelta, Some(1));
        writer.write_assignment(a.clone()).unwrap();
        writer.write_assignment(b.clone()).unwrap();
        writer.write_assignment(b.clone()).unwrap();
        writer.finish().unwrap();
    }

    let reader = BenStreamReader::from_xben(Cursor::new(xben)).unwrap();
    let decoded: Vec<(Vec<u16>, u16)> = reader.map(|r| r.unwrap()).collect();
    let total: usize = decoded.iter().map(|&(_, c)| c as usize).sum();
    assert_eq!(total, 3);
    assert_eq!(decoded[0].0, a);
    for (assignment, _) in &decoded[1..] {
        assert_eq!(assignment, &b);
    }
}

#[test]
fn xben_twodelta_long_run_repeat_saturation_falls_back_to_full_frame() {
    // u16::MAX identical samples saturate the pending full frame's count; the next repeat cannot
    // be a delta-shaped frame (single-district body → pair-projected run beyond u16::MAX), so the
    // writer re-buffers it as a fresh full frame and keeps merging later repeats into it.
    let a = long_run_assignment();
    let n = u16::MAX as usize + 2;

    let mut xben = Vec::new();
    {
        let mut writer = build_xben_writer(&mut xben, BenVariant::TwoDelta, None);
        for _ in 0..n {
            writer.write_assignment(a.clone()).unwrap();
        }
        writer.finish().unwrap();
    }

    let mut total = 0usize;
    BenStreamReader::from_xben(Cursor::new(xben))
        .unwrap()
        .silent(true)
        .for_each_assignment(|assignment, count| {
            assert_eq!(assignment, a.as_slice());
            total += count as usize;
            Ok(true)
        })
        .unwrap();
    assert_eq!(total, n);
}

#[test]
fn xben_twodelta_long_run_chunk_repeat_saturation_falls_back_to_full_frame() {
    // An A→B delta seeds the chunk, u16::MAX repeats of B saturate that delta's count, and the
    // next repeat trips the chunk-saturation path: B's repeat pair projects onto a run beyond
    // u16::MAX → pending-full fallback.
    let mut a = vec![1u16; u16::MAX as usize + 3];
    a[u16::MAX as usize + 1] = 3;
    a[u16::MAX as usize + 2] = 4;
    let mut b = a.clone();
    b[u16::MAX as usize + 1] = 4;
    let n_b = u16::MAX as usize + 2;

    let mut xben = Vec::new();
    {
        let mut writer = build_xben_writer(&mut xben, BenVariant::TwoDelta, None);
        writer.write_assignment(a.clone()).unwrap();
        for _ in 0..n_b {
            writer.write_assignment(b.clone()).unwrap();
        }
        writer.finish().unwrap();
    }

    let mut a_total = 0usize;
    let mut b_total = 0usize;
    BenStreamReader::from_xben(Cursor::new(xben))
        .unwrap()
        .silent(true)
        .for_each_assignment(|assignment, count| {
            if assignment == a.as_slice() {
                a_total += count as usize;
            } else {
                assert_eq!(assignment, b.as_slice());
                b_total += count as usize;
            }
            Ok(true)
        })
        .unwrap();
    assert_eq!((a_total, b_total), (1, n_b));
}

#[test]
fn translate_twodelta_non_eof_read_error_propagates() {
    use std::io::{self, Read};

    // ingest_ben_stream in TwoDelta mode calls translate_ben_twodelta_to_xben. After consuming a
    // complete snapshot frame it loops reading the next frame's tag byte; a non-EOF error there
    // must propagate.
    let mut xben = Vec::new();
    let mut writer = build_xben_writer(&mut xben, BenVariant::TwoDelta, None);

    // Banner (17 bytes) + a complete snapshot frame:
    //   snapshot tag, then max_val_bits=1, max_len_bits=1, n_bytes=0 (no payload), count=1.
    let mut input: Vec<u8> = b"TWODELTA BEN FILE".to_vec();
    input.extend_from_slice(&[0x00, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01]);

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
    // write_assignment(a); write_frame(b, 0); write_assignment(a) should act like two adjacent
    // write_assignment(a) calls — no inserted frame boundary.
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
    // Round-trip must reproduce the inputs, which proves the delta against the emitted anchor was
    // encoded correctly.
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
    // Pin guard ordering: finished/wrong-mode checks happen before the zero-count no-op.
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
    // The banner write happens during construction; constructor failure bypasses WriterState
    // entirely. To exercise the post-construction poisoning path we wrap a buffer that accepts only
    // the 17 banner bytes and errors on subsequent writes.
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
    // Second call with a different assignment triggers a flush, which must fail and poison the
    // writer.
    let err = w.write_assignment(vec![4u16, 5, 6]).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::Other);
    let err = w.write_assignment(vec![1u16, 2, 3]).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    let err = w.write_frame(vec![1u16, 2, 3], 1).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    let err = w.finish().unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

// ── stream_writer/mod.rs coverage ─────────────────────────────────────

use crate::io::reader::BenWireFormat;
use crate::io::writer::XzEncodeOptions;

#[test]
fn for_xben_top_level_constructor_round_trips_per_variant() {
    // The internal codec plumbing builds XBEN writers through `for_xben_with_encoder` with a
    // pre-built XzEncoder; the public `for_xben` constructor (which takes XzEncodeOptions and
    // builds the encoder internally) is the path external callers use. Exercise it directly so
    // the encoder-construction branch isn't only covered indirectly.
    let assignment = vec![1u16, 1, 2, 2];
    for variant in [
        BenVariant::Standard,
        BenVariant::MkvChain,
        BenVariant::TwoDelta,
    ] {
        let mut buf = Vec::new();
        {
            let opts = XzEncodeOptions::new()
                .with_n_threads(1)
                .with_compression_level(1);
            let mut writer = BenStreamWriter::for_xben(&mut buf, variant, opts).unwrap();
            writer.write_assignment(assignment.clone()).unwrap();
            writer.finish().unwrap();
        }
        let decoded: Vec<Vec<u16>> = BenStreamReader::from_xben(Cursor::new(&buf))
            .unwrap()
            .silent(true)
            .flat_map(|r| {
                let (a, c) = r.unwrap();
                std::iter::repeat_n(a, c as usize)
            })
            .collect();
        assert_eq!(decoded, vec![assignment.clone()], "variant={variant:?}");
    }
}

#[test]
fn writer_variant_and_wire_format_accessors_reflect_construction() {
    // The variant() and wire_format() accessors are zero-cost getters but easy to regress —
    // a future refactor that adds a third inner variant must keep these in sync. Pin both.
    for variant in [
        BenVariant::Standard,
        BenVariant::MkvChain,
        BenVariant::TwoDelta,
    ] {
        let mut buf = Vec::new();
        let ben_writer = BenStreamWriter::for_ben(&mut buf, variant).unwrap();
        assert_eq!(ben_writer.variant(), variant);
        assert_eq!(ben_writer.wire_format(), BenWireFormat::Ben);
        drop(ben_writer); // BEN writer drop is a no-op-flush.

        let mut buf = Vec::new();
        let xben_writer = build_xben_writer(&mut buf, variant, None);
        assert_eq!(xben_writer.variant(), variant);
        assert_eq!(xben_writer.wire_format(), BenWireFormat::XBen);
    }
}

#[test]
fn finish_into_inner_returns_underlying_buffer_for_ben_open_state() {
    // `finish_into_inner` from the Open state must flush pending state and hand back the inner
    // writer. Pins the BEN-Open branch (lines 303-307).
    let buf = Vec::new();
    let mut writer = BenStreamWriter::for_ben(buf, BenVariant::Standard).unwrap();
    writer.write_assignment(vec![1u16, 2]).unwrap();
    let inner = writer.finish_into_inner().unwrap();
    // Inner must contain at least the banner; concrete bytes are pinned by other tests.
    assert!(inner.starts_with(b"STANDARD BEN FILE"));
}

#[test]
fn finish_into_inner_returns_underlying_buffer_for_xben_open_state() {
    // Pins the XBEN-Open branch (lines 309-315): finish the xz encoder and return the inner
    // buffer.
    let buf = Vec::new();
    let encoder = XzEncoder::new(buf, 1);
    let mut writer =
        BenStreamWriter::for_xben_with_encoder(encoder, BenVariant::Standard, None).unwrap();
    writer.write_assignment(vec![1u16, 2]).unwrap();
    let inner = writer.finish_into_inner().unwrap();
    // The inner buffer should be a complete xz stream (decompresses to the BEN stream). We
    // don't pin exact bytes; just confirm the writer handed back a non-empty buffer.
    assert!(!inner.is_empty());
}

#[test]
fn finish_into_inner_from_complete_state_returns_buffer_without_double_flush() {
    // After `finish()` succeeds the writer is Complete; `finish_into_inner` must accept this
    // state and return the inner writer without trying to flush again.
    let buf = Vec::new();
    let mut writer = BenStreamWriter::for_ben(buf, BenVariant::Standard).unwrap();
    writer.write_assignment(vec![1u16, 2]).unwrap();
    writer.finish().unwrap();
    let inner = writer.finish_into_inner().unwrap();
    assert!(inner.starts_with(b"STANDARD BEN FILE"));
}

#[test]
fn write_json_value_with_malformed_assignment_field_does_not_poison() {
    // The JSON-parse step in write_json_value is preflight: a malformed input must error but
    // leave the writer in Open so subsequent valid writes still work. This pins the contract
    // that JSON validation happens before any stateful encode work.
    use serde_json::json;
    let mut buf = Vec::new();
    {
        let mut writer = BenStreamWriter::for_ben(&mut buf, BenVariant::Standard).unwrap();
        // Missing the "assignment" field -> rejected by parse_json_assignment, NOT a stateful
        // write -> writer stays Open.
        let bad = json!({"sample": 1});
        let err = writer.write_json_value(bad).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        // Writer must still accept a valid sample after the preflight rejection.
        writer
            .write_json_value(json!({"assignment": [1, 2], "sample": 1}))
            .unwrap();
        writer.finish().unwrap();
    }
    assert!(buf.starts_with(b"STANDARD BEN FILE"));
}
