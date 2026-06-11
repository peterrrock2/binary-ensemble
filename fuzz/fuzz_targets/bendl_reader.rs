//! Coverage-guided fuzzing of the `.bendl` bundle read surface.
//!
//! Mutants split into the same two classes as the deterministic harness: open-rejected (the
//! constructor is the whole reachable surface) and openable (every accessor must then hold the
//! no-panic contract, including the verified and unverified asset/stream readers).

#![no_main]

use binary_ensemble::io::bundle::reader::BendlReader;
use binary_ensemble::io::bundle::writer::BendlAppender;
use libfuzzer_sys::fuzz_target;
use std::io::{Cursor, Read};

const MAX_PULLS: usize = 64;

fuzz_target!(|data: &[u8]| {
    if let Ok(mut reader) = BendlReader::open(Cursor::new(data.to_vec())) {
        let _ = reader.is_finalized();
        let _ = reader.sample_count();
        let _ = reader.assignment_format();
        let _ = reader.validate_directory();

        for entry in reader.assets().to_vec() {
            let _ = reader.asset_bytes(&entry);
            let _ = reader.asset_bytes_unverified(&entry);
            if let Ok(mut payload) = reader.asset_payload_reader_unverified(&entry) {
                let _ = payload.read_to_end(&mut Vec::new());
            }
            let _ = reader.verify_asset_checksum(&entry);
        }
        let _ = reader.verify_all_asset_checksums();
        let _ = reader.verify_stream_checksum();

        if let Ok(mut stream) = reader.assignment_stream_reader() {
            let _ = stream.read_to_end(&mut Vec::new());
        }
        if let Ok(mut stream) = reader.assignment_stream_reader_unverified() {
            let _ = stream.read_to_end(&mut Vec::new());
        }
        if let Ok(verified) = reader.open_assignment_reader() {
            for record in verified.silent(true).take(MAX_PULLS) {
                let _ = record;
            }
        }
        if let Ok(verified) = reader.open_assignment_reader() {
            let _ = verified.count_samples();
        }
        if let Ok(unverified) = reader.open_assignment_reader_unverified() {
            for record in unverified.silent(true).take(MAX_PULLS) {
                let _ = record;
            }
        };
    }

    let _ = BendlAppender::open(Cursor::new(data.to_vec()));
});
