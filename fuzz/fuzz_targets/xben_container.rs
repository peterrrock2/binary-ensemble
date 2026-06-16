//! Coverage-guided fuzzing of the raw `.xben` container surface (xz framing + dispatch).
//!
//! Complement of `xben_body`: here the fuzz input is the container itself, so the xz layer, the
//! banner dispatch, and the error paths between them face the corruption.

//! Full-drain entry points are deliberately absent here too (see `xben_body.rs`): bounded
//! iteration covers the same dispatch and xz plumbing without the `length × count` output cost.

#![no_main]

use binary_ensemble::codec::decode::xz_decompress;
use binary_ensemble::io::reader::{BenStreamFrameReader, BenStreamReader};
use libfuzzer_sys::fuzz_target;
use std::io::BufReader;

const MAX_PULLS: usize = 64;

fuzz_target!(|data: &[u8]| {
    let _ = xz_decompress(BufReader::new(data), std::io::sink());

    if let Ok(reader) = BenStreamReader::from_xben(data) {
        for record in reader.silent(true).take(MAX_PULLS) {
            let _ = record;
        }
    }
    if let Ok(frames) = BenStreamFrameReader::from_xben(data) {
        for frame in frames.take(MAX_PULLS) {
            let _ = frame;
        }
    }
});
