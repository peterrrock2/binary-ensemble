//! Property-based equivalence tests for the high-level ops (`relabel`, `extract`, `convert`).
//!
//! Existing proptests in `tests/test_impls_pipeline.rs` cover encoder/decoder round-trips
//! (`translate` direction). The complementary properties here pin the algebraic identities of
//! the post-decode operations:
//!
//! - **relabel composition:** for any node permutation `P` of length `L`, `relabel(P^-1, relabel(P,
//!   x)) == x`.
//! - **extract correctness:** for any sample index `i` in `0..N`, `extract(i, encode(x)) == x[i]`.
//! - **convert variant round-trip:** for any variant pair `(A, B)`, `convert(A, convert(B, x)) ==
//!   x` (compared at the decoded-assignment level, since BEN variants differ in frame structure and
//!   counts but not assignment data).

use binary_ensemble::codec::decode::decode_ben_to_jsonl;
use binary_ensemble::codec::encode::encode_jsonl_to_ben;
use binary_ensemble::ops::extract::extract_assignment_ben;
use binary_ensemble::ops::relabel::{convert_ben_file, relabel_ben_file, RelabelOptions};
use binary_ensemble::BenVariant;
use proptest::prelude::*;
use serde_json::json;
use std::collections::HashMap;
use std::io::{BufReader, Cursor, Write};

/// Build canonical JSONL from a sequence of equal-length assignments.
fn jsonl_from(seq: &[Vec<u16>]) -> Vec<u8> {
    let mut buf = Vec::new();
    for (i, a) in seq.iter().enumerate() {
        writeln!(&mut buf, "{}", json!({"assignment": a, "sample": i + 1})).unwrap();
    }
    buf
}

/// Strategy: a sequence of `n` assignments, each of length `L`, with values in `1..=max_val`.
fn strat_fixed_length_seq(
    max_val: u16,
    len: usize,
    max_samples: usize,
) -> impl Strategy<Value = Vec<Vec<u16>>> {
    prop::collection::vec(prop::collection::vec(1u16..=max_val, len), 1..=max_samples)
}

/// Invert a permutation `P` (new_idx → old_idx). The inverse maps `old_idx → new_idx`. Given
/// the contract that callers pass a contiguous bijection over `0..len`, the inverse is also a
/// bijection over the same range and is what we apply to undo `P`.
fn invert_permutation(p: &HashMap<usize, usize>) -> HashMap<usize, usize> {
    p.iter().map(|(&new, &old)| (old, new)).collect()
}

/// Build a `Vec<usize>` permutation of `0..len` by shuffling deterministically from a `u64` seed.
/// Used inside proptest bodies to derive a shuffle from a generated seed input rather than
/// strategy-composing one (which would require ValueTree plumbing).
fn shuffled_indices(len: usize, seed: u64) -> Vec<usize> {
    use rand::seq::SliceRandom;
    use rand::SeedableRng;
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
    let mut indices: Vec<usize> = (0..len).collect();
    indices.shuffle(&mut rng);
    indices
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 48,
        ..ProptestConfig::default()
    })]

    /// `relabel(P^-1, relabel(P, x)) == x` for any node permutation `P` of fixed length. The
    /// relabel API operates at the BEN-stream level: we encode `x` as BEN, run `relabel(P)`,
    /// then `relabel(P^-1)`, decode the resulting BEN and compare the assignment sequence
    /// against the original.
    #[test]
    fn relabel_composition_is_identity(
        len in 1usize..=12,
        n_samples in 1usize..=8usize,
    ) {
        let seq = (0..n_samples)
            .map(|i| (0..len).map(|j| ((i * 31 + j * 7) % 5 + 1) as u16).collect::<Vec<u16>>())
            .collect::<Vec<_>>();

        // Generate a permutation deterministically from `len` so this case is reproducible.
        let mut p: HashMap<usize, usize> = HashMap::new();
        for i in 0..len {
            // Rotate by 1: simple non-identity permutation that exercises every position.
            p.insert(i, (i + 1) % len);
        }
        let p_inv = invert_permutation(&p);

        let jsonl = jsonl_from(&seq);
        let mut ben = Vec::new();
        encode_jsonl_to_ben(BufReader::new(jsonl.as_slice()), &mut ben, BenVariant::Standard)
            .unwrap();

        let mut after_p = Vec::new();
        relabel_ben_file(
            ben.as_slice(),
            &mut after_p,
            RelabelOptions::node_permutation(p),
        )
        .unwrap();

        let mut after_p_inv = Vec::new();
        relabel_ben_file(
            after_p.as_slice(),
            &mut after_p_inv,
            RelabelOptions::node_permutation(p_inv),
        )
        .unwrap();

        let mut decoded = Vec::new();
        decode_ben_to_jsonl(after_p_inv.as_slice(), &mut decoded).unwrap();
        prop_assert_eq!(decoded, jsonl);
    }

    /// `relabel` with a random shuffle composed with its inverse is identity. The shuffle is
    /// seeded from a generated `u64` so proptest can shrink the seed when a counterexample is
    /// found.
    #[test]
    fn relabel_random_shuffle_composition_is_identity(
        seq in strat_fixed_length_seq(5, 8, 6),
        seed in any::<u64>(),
    ) {
        let len = seq[0].len();
        let shuffled = shuffled_indices(len, seed);
        let p: HashMap<usize, usize> =
            (0..len).map(|new_idx| (new_idx, shuffled[new_idx])).collect();
        let p_inv = invert_permutation(&p);

        let jsonl = jsonl_from(&seq);
        let mut ben = Vec::new();
        encode_jsonl_to_ben(BufReader::new(jsonl.as_slice()), &mut ben, BenVariant::Standard)
            .unwrap();

        let mut after_p = Vec::new();
        relabel_ben_file(
            ben.as_slice(),
            &mut after_p,
            RelabelOptions::node_permutation(p),
        )
        .unwrap();

        let mut after_p_inv = Vec::new();
        relabel_ben_file(
            after_p.as_slice(),
            &mut after_p_inv,
            RelabelOptions::node_permutation(p_inv),
        )
        .unwrap();

        let mut decoded = Vec::new();
        decode_ben_to_jsonl(after_p_inv.as_slice(), &mut decoded).unwrap();
        prop_assert_eq!(decoded, jsonl);
    }

    /// `extract(i, encode(x)) == x[i]` for every zero-based sample index `i` in `0..N`.
    /// Sweeps the entire sequence (not just a random index) because extract correctness for one
    /// index is almost free to verify for all of them once the BEN file is built.
    #[test]
    fn extract_returns_the_correct_sample(
        seq in strat_fixed_length_seq(8, 5, 6),
    ) {
        let jsonl = jsonl_from(&seq);
        let mut ben = Vec::new();
        encode_jsonl_to_ben(BufReader::new(jsonl.as_slice()), &mut ben, BenVariant::Standard)
            .unwrap();

        for (i, expected) in seq.iter().enumerate() {
            let extracted = extract_assignment_ben(Cursor::new(&ben), i).unwrap();
            prop_assert_eq!(&extracted, expected,
                "extract(index={}) returned the wrong assignment", i);
        }
    }

    /// `convert(A, convert(B, x)) == x`, asserted at the decoded-assignment level. BEN
    /// variants encode assignment runs differently (Standard packs label/count pairs; MkvChain
    /// adds an outer repetition count for adjacent equal assignments) and the round trip must
    /// preserve every materialized assignment regardless of which intermediate representation
    /// is used. Pairs `(Standard, MkvChain)` and the same in reverse pin both directions.
    ///
    /// TwoDelta is intentionally excluded from this sweep: it imposes a structural constraint
    /// that each delta assignment must only contain values from a 2-value pair shared with the
    /// previous assignment, so `convert(arbitrary_BEN, TwoDelta)` is not well-defined for the
    /// general inputs this strategy generates. TwoDelta round-trips are exercised by the
    /// dedicated `fuzz_roundtrip_*_twodelta` proptests in `test_impls_pipeline.rs`, which use
    /// the `strat_twodelta_seq` strategy that respects those constraints.
    #[test]
    fn convert_variant_round_trip_preserves_assignments(
        seq in strat_fixed_length_seq(8, 5, 6),
    ) {
        let jsonl = jsonl_from(&seq);

        for (source, intermediate) in &[
            (BenVariant::Standard, BenVariant::MkvChain),
            (BenVariant::MkvChain, BenVariant::Standard),
        ] {
            let mut start_ben = Vec::new();
            encode_jsonl_to_ben(BufReader::new(jsonl.as_slice()), &mut start_ben, *source)
                .unwrap();

            let mut mid_ben = Vec::new();
            convert_ben_file(start_ben.as_slice(), &mut mid_ben, *intermediate).unwrap();
            let mut end_ben = Vec::new();
            convert_ben_file(mid_ben.as_slice(), &mut end_ben, *source).unwrap();

            // Decode both endpoints to JSONL and compare. Direct byte comparison would over-pin
            // frame boundaries and run-length grouping (e.g. `convert(MkvChain, ...)` may merge
            // adjacent equal assignments into a single repeat-count frame).
            let mut start_jsonl = Vec::new();
            decode_ben_to_jsonl(start_ben.as_slice(), &mut start_jsonl).unwrap();
            let mut end_jsonl = Vec::new();
            decode_ben_to_jsonl(end_ben.as_slice(), &mut end_jsonl).unwrap();
            prop_assert_eq!(end_jsonl, start_jsonl,
                "convert {:?} -> {:?} -> {:?} did not preserve assignments",
                source, intermediate, source);
        }
    }
}
