//! Exhaustive single-byte mutation fuzzing of the committed wire-format fixtures.
//!
//! For every byte position of every committed `tests/fixtures/v1.0.0/` binary fixture, three
//! mutants are produced (bit-flip, increment, zero) and each mutant is driven through **every
//! public read entry point** for its wire format. The contract under test is *panic freedom*:
//!
//! - A mutant may fail to parse — any `io::Result` error is acceptable.
//! - A mutant may even decode successfully (plain BEN and XBEN carry no integrity bytes, so a
//!   payload mutation can produce a different-but-structurally-valid stream). That is acceptable
//!   here too; whole-stream integrity is the `.bendl` layer's job and is covered by its own
//!   checksum tests.
//! - What a mutant must never do, at any byte position, is panic, abort, or hang an entry point.
//!
//! When a new public read API is added, register it in the matching `drive_*` function below —
//! that one registration extends the exhaustive corruption coverage to the new surface.

use std::io::{self, Cursor, Read};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use binary_ensemble::codec::decode::{
    decode_ben_to_jsonl, decode_xben_to_ben, decode_xben_to_jsonl, xz_decompress,
};
use binary_ensemble::codec::encode::{encode_ben_to_xben, xz_compress};
use binary_ensemble::io::bundle::reader::BendlReader;
use binary_ensemble::io::bundle::writer::BendlAppender;
use binary_ensemble::io::reader::{BenStreamFrameReader, BenStreamReader};
use binary_ensemble::ops::extract::{extract_assignment_ben, extract_assignment_xben};
use binary_ensemble::ops::relabel::{relabel_ben_file, RelabelOptions};
use binary_ensemble::BenVariant;

/// Upper bound on records pulled from any iterator-style entry point. A corrupt stream may yield
/// an error from `next()` without ending the iterator, so iteration is bounded rather than driven
/// to `None`; the fixtures hold five samples, so the bound is far above any legitimate yield
/// count.
const MAX_PULLS: usize = 1_000;

fn fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("v1.0.0")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("missing fixture {path:?}: {e}"))
}

/// The three mutation patterns applied at each byte position, deduplicated against the original
/// byte so every driven mutant really differs from the committed fixture.
fn mutations_of(original: u8) -> Vec<u8> {
    let mut out = vec![original ^ 0xFF, original.wrapping_add(1), 0x00];
    out.dedup();
    out.retain(|&b| b != original);
    out
}

/// Run one labeled entry point over one mutant under `catch_unwind`, converting any panic into a
/// test failure that names the fixture, byte position, mutated value, and entry point.
fn assert_no_panic(fixture_name: &str, pos: usize, byte: u8, label: &str, f: impl FnOnce()) {
    let outcome = catch_unwind(AssertUnwindSafe(f));
    assert!(
        outcome.is_ok(),
        "{label} panicked on {fixture_name} with byte {pos} set to {byte:#04x}"
    );
}

/// Drive every public plain-BEN read entry point over `bytes`.
fn drive_ben_entry_points(fixture_name: &str, pos: usize, byte: u8, bytes: &[u8]) {
    let run = |label: &str, f: &dyn Fn()| assert_no_panic(fixture_name, pos, byte, label, f);

    run("decode_ben_to_jsonl", &|| {
        let _ = decode_ben_to_jsonl(bytes, io::sink());
    });
    run("BenStreamReader::from_ben iterate", &|| {
        if let Ok(reader) = BenStreamReader::from_ben(bytes) {
            for record in reader.silent(true).take(MAX_PULLS) {
                let _ = record;
            }
        }
    });
    run("BenStreamReader::from_ben count_samples", &|| {
        if let Ok(reader) = BenStreamReader::from_ben(bytes) {
            let _ = reader.silent(true).count_samples();
        }
    });
    run("BenStreamFrameReader::from_ben iterate", &|| {
        if let Ok(frames) = BenStreamFrameReader::from_ben(bytes) {
            for frame in frames.take(MAX_PULLS) {
                let _ = frame;
            }
        }
    });
    run("into_subsample_by_indices", &|| {
        if let Ok(reader) = BenStreamReader::from_ben(bytes) {
            for record in reader
                .silent(true)
                .into_subsample_by_indices(vec![1, 3])
                .take(MAX_PULLS)
            {
                let _ = record;
            }
        }
    });
    run("into_subsample_by_range", &|| {
        if let Ok(reader) = BenStreamReader::from_ben(bytes) {
            for record in reader
                .silent(true)
                .into_subsample_by_range(1, 2)
                .take(MAX_PULLS)
            {
                let _ = record;
            }
        }
    });
    run("into_subsample_every", &|| {
        if let Ok(reader) = BenStreamReader::from_ben(bytes) {
            for record in reader.silent(true).into_subsample_every(2, 1).take(MAX_PULLS) {
                let _ = record;
            }
        }
    });
    run("relabel_ben_file first_seen", &|| {
        let _ = relabel_ben_file(bytes, io::sink(), RelabelOptions::first_seen());
    });
    run("relabel_ben_file convert_to MkvChain", &|| {
        let _ = relabel_ben_file(
            bytes,
            io::sink(),
            RelabelOptions::convert_to(BenVariant::MkvChain),
        );
    });
    run("relabel_ben_file convert_to TwoDelta", &|| {
        let _ = relabel_ben_file(
            bytes,
            io::sink(),
            RelabelOptions::convert_to(BenVariant::TwoDelta),
        );
    });
    run("relabel_ben_file node_permutation", &|| {
        let map = (0..4usize).map(|i| (i, 3 - i)).collect();
        let _ = relabel_ben_file(bytes, io::sink(), RelabelOptions::node_permutation(map));
    });
    run("extract_assignment_ben", &|| {
        let _ = extract_assignment_ben(bytes, 1);
        let _ = extract_assignment_ben(bytes, 3);
    });
    // Encode-side entry point that *reads* untrusted BEN: the BEN→XBEN converter re-parses every
    // frame (including the TwoDelta ingest path), so it faces the same corruption surface as the
    // decoders.
    run("encode_ben_to_xben", &|| {
        let _ = encode_ben_to_xben(
            io::BufReader::new(bytes),
            io::sink(),
            Some(1),
            Some(0),
            None,
            None,
        );
    });
}

/// Drive every public XBEN read entry point over `bytes`.
fn drive_xben_entry_points(fixture_name: &str, pos: usize, byte: u8, bytes: &[u8]) {
    let run = |label: &str, f: &dyn Fn()| assert_no_panic(fixture_name, pos, byte, label, f);

    run("decode_xben_to_jsonl", &|| {
        let _ = decode_xben_to_jsonl(io::BufReader::new(bytes), io::sink());
    });
    run("decode_xben_to_ben", &|| {
        let _ = decode_xben_to_ben(io::BufReader::new(bytes), io::sink());
    });
    run("xz_decompress", &|| {
        let _ = xz_decompress(io::BufReader::new(bytes), io::sink());
    });
    run("BenStreamReader::from_xben iterate", &|| {
        if let Ok(reader) = BenStreamReader::from_xben(bytes) {
            for record in reader.silent(true).take(MAX_PULLS) {
                let _ = record;
            }
        }
    });
    run("BenStreamReader::from_xben count_samples", &|| {
        if let Ok(reader) = BenStreamReader::from_xben(bytes) {
            let _ = reader.silent(true).count_samples();
        }
    });
    run("BenStreamFrameReader::from_xben iterate", &|| {
        if let Ok(frames) = BenStreamFrameReader::from_xben(bytes) {
            for frame in frames.take(MAX_PULLS) {
                let _ = frame;
            }
        }
    });
    run("from_xben into_subsample_by_range", &|| {
        if let Ok(reader) = BenStreamReader::from_xben(bytes) {
            for record in reader
                .silent(true)
                .into_subsample_by_range(1, 2)
                .take(MAX_PULLS)
            {
                let _ = record;
            }
        }
    });
    run("extract_assignment_xben", &|| {
        let _ = extract_assignment_xben(bytes, 1);
    });
}

/// Drive every public `.bendl` read entry point over `bytes`.
///
/// Mutants split into two classes: open-rejected (constructor errors — nothing else reachable)
/// and openable (every accessor must then hold the no-panic contract).
fn drive_bendl_entry_points(fixture_name: &str, pos: usize, byte: u8, bytes: &[u8]) {
    let run = |label: &str, f: &dyn Fn()| assert_no_panic(fixture_name, pos, byte, label, f);

    run("BendlReader full surface", &|| {
        let Ok(mut reader) = BendlReader::open(Cursor::new(bytes.to_vec())) else {
            return; // Open-rejected mutant: the constructor is the whole reachable surface.
        };

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
    });
    run("BendlAppender::open", &|| {
        let _ = BendlAppender::open(Cursor::new(bytes.to_vec()));
    });
}

/// Apply every single-byte mutation to `original` and hand each mutant to `drive`.
fn mutate_and_drive(fixture_name: &str, original: &[u8], drive: impl Fn(&str, usize, u8, &[u8])) {
    let mut mutant = original.to_vec();
    for pos in 0..original.len() {
        for byte in mutations_of(original[pos]) {
            mutant[pos] = byte;
            drive(fixture_name, pos, byte, &mutant);
        }
        mutant[pos] = original[pos];
    }
}

fn ben_fixture_names() -> [&'static str; 3] {
    ["standard.ben", "mkvchain.ben", "twodelta.ben"]
}

fn xben_fixture_names() -> [&'static str; 3] {
    ["standard.xben", "mkvchain.xben", "twodelta.xben"]
}

fn bendl_fixture_names() -> [&'static str; 2] {
    ["flags_set.bendl", "unknown_flags.bendl"]
}

#[test]
fn mutated_ben_fixtures_never_panic_any_entry_point() {
    for name in ben_fixture_names() {
        let original = fixture(name);
        mutate_and_drive(name, &original, drive_ben_entry_points);
    }
}

#[test]
fn mutated_xben_fixtures_never_panic_any_entry_point() {
    for name in xben_fixture_names() {
        let original = fixture(name);
        mutate_and_drive(name, &original, drive_xben_entry_points);
    }
}

#[test]
fn mutated_bendl_fixtures_never_panic_any_entry_point() {
    for name in bendl_fixture_names() {
        let original = fixture(name);
        mutate_and_drive(name, &original, drive_bendl_entry_points);
    }
}

/// Wrap a (possibly corrupt) decompressed XBEN body in a fresh, *valid* xz container.
///
/// Mutating the compressed fixture bytes mostly exercises the xz layer, whose own integrity
/// checks reject the mutant before the BEN32/TwoDelta parsers run. Re-compressing a mutated
/// body delivers the corruption past the xz wrapper, straight to the parsers under test.
fn recompress(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    xz_compress(io::BufReader::new(body), &mut out, Some(1), Some(0), None)
        .expect("compressing a small in-memory body cannot fail");
    out
}

/// Single-byte mutation of the *decompressed* XBEN bodies, re-wrapped in valid xz so the inner
/// BEN32/TwoDelta parsers (not the xz layer) face the corruption.
#[test]
fn mutated_xben_bodies_never_panic_any_entry_point() {
    for name in xben_fixture_names() {
        let compressed = fixture(name);
        let mut body = Vec::new();
        xz_decompress(io::BufReader::new(compressed.as_slice()), &mut body)
            .expect("committed fixture must decompress");

        let mut mutant = body.clone();
        for pos in 0..body.len() {
            for byte in mutations_of(body[pos]) {
                mutant[pos] = byte;
                drive_xben_entry_points(name, pos, byte, &recompress(&mutant));
            }
            mutant[pos] = body[pos];
        }
    }
}

/// Truncation of the *decompressed* XBEN bodies, re-wrapped in valid xz: a clean container whose
/// inner stream ends mid-frame — the corruption class a damaged-but-recompressed file presents.
#[test]
fn truncated_xben_bodies_never_panic_any_entry_point() {
    for name in xben_fixture_names() {
        let compressed = fixture(name);
        let mut body = Vec::new();
        xz_decompress(io::BufReader::new(compressed.as_slice()), &mut body)
            .expect("committed fixture must decompress");

        for end in 0..body.len() {
            drive_xben_entry_points(name, end, 0, &recompress(&body[..end]));
        }
    }
}

/// Truncation sweep: every prefix of every fixture, through the same entry points. Single-byte
/// mutation preserves length, so this covers the orthogonal corruption axis (short files).
#[test]
fn truncated_fixtures_never_panic_any_entry_point() {
    for name in ben_fixture_names() {
        let original = fixture(name);
        for end in 0..original.len() {
            drive_ben_entry_points(name, end, 0, &original[..end]);
        }
    }
    for name in xben_fixture_names() {
        let original = fixture(name);
        for end in 0..original.len() {
            drive_xben_entry_points(name, end, 0, &original[..end]);
        }
    }
    for name in bendl_fixture_names() {
        let original = fixture(name);
        for end in 0..original.len() {
            drive_bendl_entry_points(name, end, 0, &original[..end]);
        }
    }
}
