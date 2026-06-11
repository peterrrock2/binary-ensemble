//! Boundary-biased round-trip property tests.
//!
//! The strategies in `test_impls_pipeline.rs` deliberately stay small (ids ≤ 2000, runs ≤ 300,
//! length ≤ 1500) to keep runtime bounded — which means they structurally cannot reach any `u16`
//! wire-format boundary. This suite is the complement: its generators are *biased toward* the
//! representability edges where data-dependent encode bugs live:
//!
//! - run lengths straddling `u16::MAX` (64-bit-clean splitting in layer-1 RLE and the BEN32 body,
//!   and the TwoDelta long-run snapshot fallback);
//! - district ids `0` and `u16::MAX` (sentinel-adjacent values, TwoDelta pair synthesis);
//! - transition shapes mixing repeats, 2-id repaints (delta-eligible), and many-id changes
//!   (snapshot transitions).
//!
//! Every generated sequence must round-trip *exactly* through every `(variant × wire format)`
//! cell — there is no input a writer may reject, because every BEN-stack writer is total over
//! arbitrary `Vec<u16>` sequences (delta-shaped frames fall back to snapshots when a pair
//! projection exceeds the `u16` run limit).

use binary_ensemble::io::reader::BenStreamReader;
use binary_ensemble::io::writer::{BenStreamWriter, XzEncodeOptions};
use binary_ensemble::BenVariant;

use proptest::prelude::*;
use std::io::Cursor;

// =====================================================================
// Boundary-biased strategies
// =====================================================================

/// Run lengths weighted toward the `u16::MAX` boundary: mostly tiny (so values interleave), with
/// a real chance of runs just below, at, and above the 65,535 split/representability limit.
fn boundary_run_len() -> impl Strategy<Value = usize> {
    prop_oneof![
        4 => 1usize..=3,
        1 => 65_534usize..=65_535,
        2 => 65_536usize..=65_600,
    ]
}

/// District ids weighted toward the sentinel-adjacent edges: small ids, id `0`, and `u16::MAX`.
fn boundary_value() -> impl Strategy<Value = u16> {
    prop_oneof![
        4 => 1u16..=4,
        1 => Just(0u16),
        1 => Just(u16::MAX),
    ]
}

/// One assignment built from 1–4 boundary-biased runs (worst case ≈ 262k nodes).
fn boundary_assignment() -> impl Strategy<Value = Vec<u16>> {
    prop::collection::vec((boundary_value(), boundary_run_len()), 1..=4).prop_map(|runs| {
        let mut out = Vec::new();
        for (value, len) in runs {
            out.extend(std::iter::repeat_n(value, len));
        }
        out
    })
}

/// Repaint the positions occupied by two distinct values of `prev` with seed-derived alternating
/// stretches of those same two values: a valid 2-id transition (delta-eligible when both ids have
/// masks), whose pair projection inherits `prev`'s long runs — exactly the shape that forces the
/// TwoDelta long-run fallback.
fn repaint_pair(prev: &[u16], seed: u64) -> Vec<u16> {
    let mut distinct: Vec<u16> = prev.to_vec();
    distinct.sort_unstable();
    distinct.dedup();
    if distinct.len() < 2 {
        return prev.to_vec(); // single-district plan: degrade to a repeat
    }
    let a = distinct[(seed as usize) % distinct.len()];
    let mut b = distinct[((seed >> 8) as usize) % distinct.len()];
    if a == b {
        b = distinct[(distinct.iter().position(|&x| x == a).unwrap() + 1) % distinct.len()];
    }

    let mut next = prev.to_vec();
    let mut value = if seed & 1 == 0 { a } else { b };
    let mut stretch = 1 + (seed >> 16) as usize % 80_000;
    let mut placed = 0usize;
    for idx in 0..next.len() {
        if next[idx] != a && next[idx] != b {
            continue;
        }
        next[idx] = value;
        placed += 1;
        if placed == stretch {
            value = if value == a { b } else { a };
            stretch = 1 + (stretch.rotate_left(9) ^ 0x5bd1) % 80_000;
            placed = 0;
        }
    }
    next
}

/// Shift every district id by one (wrapping): a many-id transition that forces a snapshot frame
/// in TwoDelta while keeping the assignment's run structure (and its boundary runs) intact.
fn rotate_values(prev: &[u16]) -> Vec<u16> {
    prev.iter().map(|v| v.wrapping_add(1)).collect()
}

/// A short sample sequence over boundary-shaped assignments. Each step is a repeat (count
/// merging, repeat frames), a 2-id repaint (delta paths + long-run fallback), or a value
/// rotation (snapshot transitions).
fn boundary_sequence() -> impl Strategy<Value = Vec<Vec<u16>>> {
    (
        boundary_assignment(),
        prop::collection::vec(any::<u64>(), 0..=3),
    )
        .prop_map(|(base, ops)| {
            let mut seq = vec![base];
            for op in ops {
                let prev = seq.last().expect("sequence starts non-empty");
                let next = match op % 3 {
                    0 => prev.clone(),
                    1 => repaint_pair(prev, op),
                    _ => rotate_values(prev),
                };
                seq.push(next);
            }
            seq
        })
}

// =====================================================================
// Round-trip cells
// =====================================================================

fn encode_ben(samples: &[Vec<u16>], variant: BenVariant) -> Vec<u8> {
    let mut ben = Vec::new();
    let mut writer = BenStreamWriter::for_ben(&mut ben, variant).unwrap();
    for s in samples {
        writer.write_assignment(s.clone()).unwrap();
    }
    writer.finish().unwrap();
    drop(writer);
    ben
}

fn encode_xben(samples: &[Vec<u16>], variant: BenVariant) -> Vec<u8> {
    let mut xben = Vec::new();
    let mut writer =
        BenStreamWriter::for_xben(&mut xben, variant, XzEncodeOptions::default()).unwrap();
    for s in samples {
        writer.write_assignment(s.clone()).unwrap();
    }
    writer.finish().unwrap();
    drop(writer);
    xben
}

fn expand<R: std::io::Read>(reader: BenStreamReader<R>) -> Vec<Vec<u16>> {
    reader
        .silent(true)
        .flat_map(|r| {
            let (a, c) = r.unwrap();
            std::iter::repeat_n(a, c as usize)
        })
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 32,
        ..ProptestConfig::default()
    })]

    /// Every boundary-shaped sequence round-trips exactly through every variant on both wire
    /// formats. Writers are total: no generated input may be rejected.
    #[test]
    fn boundary_sequences_round_trip_every_variant_and_wire_format(seq in boundary_sequence()) {
        for variant in [BenVariant::Standard, BenVariant::MkvChain, BenVariant::TwoDelta] {
            let ben = encode_ben(&seq, variant);
            let decoded = expand(BenStreamReader::from_ben(ben.as_slice()).unwrap());
            prop_assert_eq!(&decoded, &seq, "{:?} plain BEN diverged", variant);

            let xben = encode_xben(&seq, variant);
            let decoded = expand(BenStreamReader::from_xben(Cursor::new(xben)).unwrap());
            prop_assert_eq!(&decoded, &seq, "{:?} XBEN diverged", variant);
        }
    }
}
