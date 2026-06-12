//! Coverage-guided fuzzing of the decompressed XBEN body parsers.
//!
//! Fuzzing the raw `.xben` container mostly exercises the xz layer, whose integrity checks
//! reject mutants before the BEN32/TwoDelta parsers run. This target re-wraps the fuzz input in
//! a fresh, valid xz container so corruption lands directly on the inner parsers — the same
//! trick as the deterministic harness's recompressed-body sweeps, but coverage-guided.

//! Full-drain entry points (`decode_xben_to_jsonl`/`decode_xben_to_ben`) are deliberately
//! absent: a frame's expanded output is `assignment length × count`, so the fuzzer inevitably
//! discovers tiny bodies that legally demand minutes of serialization work (the format's
//! documented decompression-bomb characteristic). The bounded iterators below cover the same
//! parsers without the unbounded output cost.

#![no_main]

use binary_ensemble::codec::encode::xz_compress;
use binary_ensemble::io::reader::{BenStreamFrameReader, BenStreamReader};
use binary_ensemble::ops::extract::extract_assignment_xben;
use libfuzzer_sys::fuzz_target;
use std::io::BufReader;

const MAX_PULLS: usize = 64;

fuzz_target!(|data: &[u8]| {
    let mut container = Vec::new();
    xz_compress(
        BufReader::new(data),
        &mut container,
        Some(1),
        Some(0),
        None,
    )
    .expect("compressing an in-memory body cannot fail");

    if let Ok(reader) = BenStreamReader::from_xben(container.as_slice()) {
        for record in reader.silent(true).take(MAX_PULLS) {
            let _ = record;
        }
    }
    if let Ok(reader) = BenStreamReader::from_xben(container.as_slice()) {
        let _ = reader.silent(true).count_samples();
    }
    if let Ok(frames) = BenStreamFrameReader::from_xben(container.as_slice()) {
        for frame in frames.take(MAX_PULLS) {
            let _ = frame;
        }
    }
    let _ = extract_assignment_xben(container.as_slice(), 2);
});
