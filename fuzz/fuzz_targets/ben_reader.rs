//! Coverage-guided fuzzing of the plain-BEN read surface.
//!
//! The deterministic mutation harness (`ben/tests/test_fixture_mutations.rs`) covers every
//! single-byte corruption of the committed fixtures exhaustively; this target explores the
//! compound, multi-byte corruptions that enumeration cannot reach. The contract is the same:
//! arbitrary bytes may error anywhere, but must never panic, hang, or exhaust memory.

//! Full-drain entry points (`decode_ben_to_jsonl`) are deliberately absent: their parse
//! coverage is identical to the bounded iterators below, but a frame's expanded output is
//! `assignment length × count`, so the fuzzer inevitably discovers tiny inputs that legally
//! demand minutes of serialization work (the format's documented decompression-bomb
//! characteristic), drowning exploration in slow units. Relabeling runs in its bounded form
//! (`with_max_samples`): its label-map building and BEN→TwoDelta re-encode consume untrusted
//! decoded values that pure iteration never feeds into an encoder, so they need their own
//! coverage.

#![no_main]

use binary_ensemble::io::reader::{BenStreamFrameReader, BenStreamReader};
use binary_ensemble::ops::extract::extract_assignment_ben;
use binary_ensemble::ops::relabel::{relabel_ben_file, RelabelOptions};
use binary_ensemble::BenVariant;
use libfuzzer_sys::fuzz_target;

/// Bound on records pulled from iterator-style entry points: corrupt streams may yield errors
/// indefinitely without ending the iterator.
const MAX_PULLS: usize = 64;

fuzz_target!(|data: &[u8]| {
    if let Ok(reader) = BenStreamReader::from_ben(data) {
        for record in reader.silent(true).take(MAX_PULLS) {
            let _ = record;
        }
    }
    if let Ok(reader) = BenStreamReader::from_ben(data) {
        let _ = reader.silent(true).count_samples();
    }
    if let Ok(frames) = BenStreamFrameReader::from_ben(data) {
        for frame in frames.take(MAX_PULLS) {
            let _ = frame;
        }
    }
    if let Ok(reader) = BenStreamReader::from_ben(data) {
        for record in reader
            .silent(true)
            .into_subsample_by_range(1, 3)
            .take(MAX_PULLS)
        {
            let _ = record;
        }
    }

    let _ = relabel_ben_file(
        data,
        std::io::sink(),
        RelabelOptions::first_seen().with_max_samples(MAX_PULLS),
    );
    let _ = relabel_ben_file(
        data,
        std::io::sink(),
        RelabelOptions::convert_to(BenVariant::TwoDelta).with_max_samples(MAX_PULLS),
    );

    let _ = extract_assignment_ben(data, 2);
});
