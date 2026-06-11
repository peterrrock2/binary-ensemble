//! Coverage-guided fuzzing of the raw `.xben` container surface (xz framing + dispatch).
//!
//! Complement of `xben_body`: here the fuzz input is the container itself, so the xz layer, the
//! banner dispatch, and the error paths between them face the corruption.

#![no_main]

use binary_ensemble::codec::decode::{decode_xben_to_ben, decode_xben_to_jsonl, xz_decompress};
use binary_ensemble::io::reader::BenStreamReader;
use libfuzzer_sys::fuzz_target;
use std::io::BufReader;

const MAX_PULLS: usize = 64;

fuzz_target!(|data: &[u8]| {
    let _ = decode_xben_to_jsonl(BufReader::new(data), std::io::sink());
    let _ = decode_xben_to_ben(BufReader::new(data), std::io::sink());
    let _ = xz_decompress(BufReader::new(data), std::io::sink());

    if let Ok(reader) = BenStreamReader::from_xben(data) {
        for record in reader.silent(true).take(MAX_PULLS) {
            let _ = record;
        }
    }
});
