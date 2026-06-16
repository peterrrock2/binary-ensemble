//! Forward-compatibility stability tests.
//!
//! Each `(BenVariant × wire format)` combination, plus two BENDL bundles, is committed as a
//! byte-identical fixture under `tests/fixtures/v1.0.0/`. These tests decode each fixture and
//! confirm the produced JSONL matches the canonical input that minted it.
//!
//! Committed fixtures are a permanent compatibility contract: they MUST continue to decode cleanly
//! in every future v1.x release of the library. The `generate_format_stability_fixtures` regen test
//! at the bottom of this file is marked `#[ignore]` precisely so it is never run by accident; if
//! the wire format ever needs to change, add a new `tests/fixtures/v<n>/` directory and a parallel
//! generator, but never overwrite an older one. See `docs/format-stability.md`.
//!
//! # Adding a new fixture for a new wire-format feature
//!
//! Within v1.0.0, the only way to add a fixture is alongside a minor-version feature that already
//! ships in v1.x. To add one:
//!
//! 1. Mint the new fixture into `tests/fixtures/v1.0.0/`.
//! 2. Add a stability test against the new file.
//! 3. Update `docs/format-stability.md` if the new fixture pins behavior not already covered.

use std::io::{BufReader, Cursor, Read};
use std::path::{Path, PathBuf};

use binary_ensemble::codec::decode::{decode_ben_to_jsonl, decode_xben_to_jsonl};
use binary_ensemble::codec::encode::{encode_jsonl_to_ben, encode_jsonl_to_xben};
use binary_ensemble::io::bundle::format::{
    AssignmentFormat, ASSET_FLAG_CHECKSUM, ASSET_FLAG_JSON, ASSET_FLAG_XZ, ASSET_TYPE_CUSTOM,
    ASSET_TYPE_GRAPH, ASSET_TYPE_METADATA, HEADER_FLAG_STREAM_CHECKSUM,
};
use binary_ensemble::io::bundle::reader::BendlReader;
use binary_ensemble::io::bundle::writer::{AddAssetOptions, BendlWriter};
use binary_ensemble::test_utils::{BendlBytes, HeaderField};
use binary_ensemble::BenVariant;

/// Canonical JSONL used to mint every codec fixture. Chosen to exercise both Standard and
/// run-length-encoded variants: each line has multiple distinct partitions, and consecutive lines
/// repeat exactly so MkvChain/TwoDelta hit their run-length code paths.
const CANONICAL_JSONL: &str = "\
{\"assignment\":[1,1,2,2],\"sample\":1}
{\"assignment\":[1,2,1,2],\"sample\":2}
{\"assignment\":[1,1,1,2],\"sample\":3}
{\"assignment\":[1,1,1,2],\"sample\":4}
{\"assignment\":[2,2,2,1],\"sample\":5}
";

/// Canonical JSONL used to mint the `TwoDelta` fixtures. They get their own source that
/// deliberately exercises mixed snapshot/delta framing: an anchor snapshot, a 2-swap delta, a
/// repeat (count), a **>2-district transition** that forces a mid-stream snapshot, and a 2-swap
/// delta rebased onto that snapshot.
const TWODELTA_CANONICAL_JSONL: &str = "\
{\"assignment\":[1,1,2,2],\"sample\":1}
{\"assignment\":[1,2,1,2],\"sample\":2}
{\"assignment\":[1,2,1,2],\"sample\":3}
{\"assignment\":[3,3,1,2],\"sample\":4}
{\"assignment\":[3,3,2,1],\"sample\":5}
";

/// Graph JSON committed as the `graph.json` asset inside the BENDL fixtures. Tiny but
/// representative of a real adjacency-style graph.
// A 64-node cycle graph. Large and repetitive enough that the writer's xz pass beats raw storage
// (514 -> ~236 bytes), so the graph asset in flags_set.bendl genuinely carries the ASSET_FLAG_XZ
// bit. A tiny graph would be stored raw, since xz cannot shrink a few dozen bytes below the
// container overhead.
const CANONICAL_GRAPH_JSON: &str = "{\"nodes\":64,\"edges\":[[0,1],[1,2],[2,3],[3,4],[4,5],[5,6],[6,7],[7,8],[8,9],[9,10],[10,11],[11,12],[12,13],[13,14],[14,15],[15,16],[16,17],[17,18],[18,19],[19,20],[20,21],[21,22],[22,23],[23,24],[24,25],[25,26],[26,27],[27,28],[28,29],[29,30],[30,31],[31,32],[32,33],[33,34],[34,35],[35,36],[36,37],[37,38],[38,39],[39,40],[40,41],[41,42],[42,43],[43,44],[44,45],[45,46],[46,47],[47,48],[48,49],[49,50],[50,51],[51,52],[52,53],[53,54],[54,55],[55,56],[56,57],[57,58],[58,59],[59,60],[60,61],[61,62],[62,63],[63,0]]}";

/// Metadata JSON committed as the `metadata.json` asset inside the BENDL fixtures.
const CANONICAL_METADATA_JSON: &str =
    "{\"variant\":\"standard\",\"bundle_version\":1,\"description\":\"v1.0.0 stability fixture\"}";

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("v1.0.0")
}

fn read_fixture(name: &str) -> Vec<u8> {
    let path = fixtures_dir().join(name);
    std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "missing fixture {path:?}: {e}. Did you run `cargo test -- --ignored \
             generate_format_stability_fixtures`?"
        )
    })
}

/// Decode a committed BEN fixture and assert the round-trip matches `expected`. The `expected`
/// source is a parameter (not a hardcoded constant) so the released Standard/MkvChain fixtures and
/// the separately-sourced TwoDelta fixture can share this helper without entangling their inputs.
fn assert_ben_fixture_round_trips(name: &str, expected: &str) {
    let bytes = read_fixture(name);
    let mut out = Vec::new();
    decode_ben_to_jsonl(&bytes[..], &mut out).expect("ben decode");
    assert_eq!(
        String::from_utf8(out).expect("decoder output is utf-8"),
        expected,
        "fixture {name} did not round-trip"
    );
}

/// Decode a committed XBEN fixture and assert the round-trip matches `expected`.
fn assert_xben_fixture_round_trips(name: &str, expected: &str) {
    let bytes = read_fixture(name);
    let mut out = Vec::new();
    decode_xben_to_jsonl(BufReader::new(&bytes[..]), &mut out).expect("xben decode");
    assert_eq!(
        String::from_utf8(out).expect("decoder output is utf-8"),
        expected,
        "fixture {name} did not round-trip"
    );
}

#[test]
fn standard_ben_v1_0_0_round_trips() {
    assert_ben_fixture_round_trips("standard.ben", CANONICAL_JSONL);
}

#[test]
fn mkvchain_ben_v1_0_0_round_trips() {
    assert_ben_fixture_round_trips("mkvchain.ben", CANONICAL_JSONL);
}

#[test]
fn twodelta_ben_v1_0_0_round_trips() {
    assert_ben_fixture_round_trips("twodelta.ben", TWODELTA_CANONICAL_JSONL);
}

#[test]
fn standard_xben_v1_0_0_round_trips() {
    assert_xben_fixture_round_trips("standard.xben", CANONICAL_JSONL);
}

#[test]
fn mkvchain_xben_v1_0_0_round_trips() {
    assert_xben_fixture_round_trips("mkvchain.xben", CANONICAL_JSONL);
}

#[test]
fn twodelta_xben_v1_0_0_round_trips() {
    assert_xben_fixture_round_trips("twodelta.xben", TWODELTA_CANONICAL_JSONL);
}

#[test]
fn flags_set_bendl_v1_0_0_decodes_all_assets_and_stream() {
    // Bundle minted with every currently-defined flag bit set: header has
    // HEADER_FLAG_STREAM_CHECKSUM and the mandatory header-checksum tail; an xz+json+checksum
    // graph asset is present; a json+checksum metadata asset is present. The reader must verify
    // both assets and the stream cleanly, and the decoded stream must round-trip back to the
    // canonical JSONL.
    let bytes = read_fixture("flags_set.bendl");
    let mut reader = BendlReader::open(Cursor::new(bytes)).expect("open bundle");

    assert!(reader.is_finalized());
    assert_eq!(reader.assignment_format(), Some(AssignmentFormat::Xben));
    assert!(reader.header().has_stream_checksum());

    let graph_entry = reader
        .find_asset_by_type(ASSET_TYPE_GRAPH)
        .expect("graph asset present")
        .clone();
    assert_eq!(
        graph_entry.asset_flags,
        ASSET_FLAG_JSON | ASSET_FLAG_XZ | ASSET_FLAG_CHECKSUM,
        "graph asset should have every defined bit set"
    );
    let graph_bytes = reader.asset_bytes(&graph_entry).expect("graph asset bytes");
    assert_eq!(graph_bytes, CANONICAL_GRAPH_JSON.as_bytes());

    let meta_entry = reader
        .find_asset_by_type(ASSET_TYPE_METADATA)
        .expect("metadata asset present")
        .clone();
    assert_eq!(
        meta_entry.asset_flags,
        ASSET_FLAG_JSON | ASSET_FLAG_CHECKSUM,
        "metadata asset should be json+checksum"
    );
    let meta_bytes = reader
        .asset_bytes(&meta_entry)
        .expect("metadata asset bytes");
    assert_eq!(meta_bytes, CANONICAL_METADATA_JSON.as_bytes());

    reader
        .verify_all_asset_checksums()
        .expect("all asset checksums valid");
    reader
        .verify_stream_checksum()
        .expect("stream checksum valid");

    let mut stream_bytes = Vec::new();
    reader
        .assignment_stream_reader_unverified()
        .expect("stream reader")
        .read_to_end(&mut stream_bytes)
        .expect("read stream");
    let mut decoded = Vec::new();
    decode_xben_to_jsonl(BufReader::new(&stream_bytes[..]), &mut decoded).expect("xben decode");
    assert_eq!(
        String::from_utf8(decoded).expect("decoder output is utf-8"),
        CANONICAL_JSONL,
        "stream did not round-trip"
    );
}

#[test]
fn unknown_flags_bendl_v1_0_0_opens_and_decodes_cleanly() {
    // Bundle minted by taking flags_set.bendl and setting reserved bits on both the header flags
    // and on the custom asset's asset_flags. Forward-compatible readers must ignore those bits:
    // the bundle still opens, assets still verify, the stream still decodes.
    let bytes = read_fixture("unknown_flags.bendl");
    let mut reader = BendlReader::open(Cursor::new(bytes)).expect("open bundle");

    // Confirm at least one reserved header bit is set so this fixture really exercises the
    // forward-compat surface. Otherwise this test would silently degrade if someone regenerated
    // the file without preserving the unknown-bits property.
    let known_header_bits = HEADER_FLAG_STREAM_CHECKSUM;
    assert_ne!(
        reader.header().flags & !known_header_bits,
        0,
        "expected at least one reserved header bit set"
    );

    // The custom asset has a reserved bit set in its asset_flags.
    let custom_entry = reader
        .assets()
        .iter()
        .find(|e| e.asset_type == ASSET_TYPE_CUSTOM)
        .expect("custom asset present")
        .clone();
    let known_asset_bits = ASSET_FLAG_JSON | ASSET_FLAG_XZ | ASSET_FLAG_CHECKSUM;
    assert_ne!(
        custom_entry.asset_flags & !known_asset_bits,
        0,
        "expected at least one reserved asset bit set"
    );

    // Despite the unknown bits, all known operations succeed.
    reader
        .verify_all_asset_checksums()
        .expect("checksums still verify with unknown bits set");
    reader
        .verify_stream_checksum()
        .expect("stream checksum still verifies");

    let mut stream_bytes = Vec::new();
    reader
        .assignment_stream_reader_unverified()
        .expect("stream reader")
        .read_to_end(&mut stream_bytes)
        .expect("read stream");
    let mut decoded = Vec::new();
    decode_xben_to_jsonl(BufReader::new(&stream_bytes[..]), &mut decoded).expect("xben decode");
    assert_eq!(
        String::from_utf8(decoded).expect("decoder output is utf-8"),
        CANONICAL_JSONL,
        "stream did not round-trip"
    );
}

// =====================================================================
// Fixture generation
// =====================================================================
//
// IMPORTANT: this is intentionally `#[ignore]`. Once v1.0.0 fixtures are committed, they MUST NOT
// be regenerated in place; see `docs/format-stability.md`. If a future format change requires
// new fixtures, add a `tests/fixtures/v<n>/` directory and a parallel generator; never overwrite
// an older directory.

fn write_fixture(name: &str, bytes: &[u8]) {
    let dir = fixtures_dir();
    std::fs::create_dir_all(&dir).expect("create fixtures dir");
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap_or_else(|e| panic!("write {path:?}: {e}"));
}

fn mint_ben(variant: BenVariant, jsonl: &str) -> Vec<u8> {
    let mut out = Vec::new();
    encode_jsonl_to_ben(Cursor::new(jsonl.as_bytes()), &mut out, variant).expect("encode ben");
    out
}

fn mint_xben(variant: BenVariant, jsonl: &str) -> Vec<u8> {
    let mut out = Vec::new();
    // Force single-threaded encoding with a fixed compression level so the bytes are deterministic.
    // Defaults vary across machines (n_threads = available parallelism), which would make
    // re-generation non-reproducible across hosts.
    encode_jsonl_to_xben(
        Cursor::new(jsonl.as_bytes()),
        &mut out,
        variant,
        Some(1),
        Some(6),
        None,
        None,
    )
    .expect("encode xben");
    out
}

fn mint_flags_set_bendl() -> Vec<u8> {
    let mut backing = Cursor::new(Vec::<u8>::new());
    let mut writer = BendlWriter::new(&mut backing, AssignmentFormat::Xben).expect("new writer");

    // Graph: json + xz + checksum (writer always adds the checksum bit).
    writer
        .add_known_asset(
            binary_ensemble::io::bundle::format::KnownAssetKind::Graph,
            CANONICAL_GRAPH_JSON.as_bytes(),
            AddAssetOptions::defaults().json().compress(),
        )
        .expect("add graph");

    // Metadata: json + checksum only (no xz).
    writer
        .add_known_asset(
            binary_ensemble::io::bundle::format::KnownAssetKind::Metadata,
            CANONICAL_METADATA_JSON.as_bytes(),
            AddAssetOptions::defaults().json().raw(),
        )
        .expect("add metadata");

    // Stream phase: write XBEN content driven from the canonical JSONL.
    let session = writer.into_stream_session().expect("into stream session");
    let mut session = session;
    encode_jsonl_to_xben(
        Cursor::new(CANONICAL_JSONL.as_bytes()),
        &mut session,
        BenVariant::Standard,
        Some(1),
        Some(6),
        None,
        None,
    )
    .expect("encode xben into session");
    // sample_count == 5 (lines in CANONICAL_JSONL).
    let writer = session.finish_into_writer(5);
    let _ = writer.finish().expect("finish bundle");

    backing.into_inner()
}

/// Returns a copy of `bytes` with reserved bits set on both the header flags and the custom
/// asset's asset_flags. Used to mint the `unknown_flags.bendl` fixture from a known-good bundle.
fn flip_unknown_flag_bits(bytes: Vec<u8>) -> Vec<u8> {
    // 1. Set header bit 1, an unknown header bit (bit 0 is the stream checksum), and re-stamp the
    //    header CRC. The flags field lives inside the CRC-covered [0, 64) region, so flipping it
    //    without re-CRC would trip the header-checksum gate when the appender below reopens the
    //    bundle. The `with_header_u64` setter re-stamps for us.
    let header_flags = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
    let bytes = BendlBytes::new(bytes)
        .with_header_u64(HeaderField::Flags, (header_flags | (1 << 1)) as u64)
        .into_bytes();

    // 2. Add a custom asset entry's asset_flags reserved bit. Since the writer-minted bundle does
    //    not include a custom asset, append one to the directory before flipping. Rather than
    //    surgery, do the simpler thing: reopen the bundle, append a custom asset via the appender
    //    API, then flip a reserved bit on its directory entry.
    let mut appender = binary_ensemble::io::bundle::writer::BendlAppender::open(Cursor::new(bytes))
        .expect("open appender");
    appender
        .add_custom_asset(
            "extra.bin",
            b"trailing custom asset",
            AddAssetOptions::defaults(),
        )
        .expect("add custom asset");
    let cursor = appender.commit().expect("commit appender");
    let mut bytes = cursor.into_inner();

    // 3. Locate the custom asset's directory entry and flip bit 7 of its asset_flags. Directory
    //    entry layout per `BendlDirectoryEntry::to_bytes`: [u16 asset_type][u16 asset_flags][u16
    //    name_len][u16 reserved][u64 payload_offset] [u64 payload_len][u32 checksum_len][name
    //    bytes][checksum bytes] asset_flags is at byte offset 2 within each entry. We scan the
    //    directory and patch the entry whose asset_type is ASSET_TYPE_CUSTOM.
    let directory_offset = u64::from_le_bytes(bytes[24..32].try_into().unwrap()) as usize;
    let entry_count_offset = directory_offset;
    let entry_count = u32::from_le_bytes(
        bytes[entry_count_offset..entry_count_offset + 4]
            .try_into()
            .unwrap(),
    ) as usize;

    let mut cursor = directory_offset + 4;
    for _ in 0..entry_count {
        let asset_type = u16::from_le_bytes(bytes[cursor..cursor + 2].try_into().unwrap());
        let name_len =
            u16::from_le_bytes(bytes[cursor + 4..cursor + 6].try_into().unwrap()) as usize;
        let checksum_len =
            u32::from_le_bytes(bytes[cursor + 24..cursor + 28].try_into().unwrap()) as usize;
        if asset_type == ASSET_TYPE_CUSTOM {
            let flags_offset = cursor + 2;
            let mut asset_flags =
                u16::from_le_bytes(bytes[flags_offset..flags_offset + 2].try_into().unwrap());
            asset_flags |= 1 << 7; // currently reserved
            bytes[flags_offset..flags_offset + 2].copy_from_slice(&asset_flags.to_le_bytes());
            return bytes;
        }
        cursor += 28 + name_len + checksum_len;
    }
    panic!("custom asset not found in directory");
}

#[test]
#[ignore = "regenerates committed v1.0.0 fixtures; never run as part of normal CI"]
fn generate_format_stability_fixtures() {
    write_fixture(
        "standard.ben",
        &mint_ben(BenVariant::Standard, CANONICAL_JSONL),
    );
    write_fixture(
        "mkvchain.ben",
        &mint_ben(BenVariant::MkvChain, CANONICAL_JSONL),
    );
    write_fixture(
        "twodelta.ben",
        &mint_ben(BenVariant::TwoDelta, TWODELTA_CANONICAL_JSONL),
    );
    write_fixture(
        "standard.xben",
        &mint_xben(BenVariant::Standard, CANONICAL_JSONL),
    );
    write_fixture(
        "mkvchain.xben",
        &mint_xben(BenVariant::MkvChain, CANONICAL_JSONL),
    );
    write_fixture(
        "twodelta.xben",
        &mint_xben(BenVariant::TwoDelta, TWODELTA_CANONICAL_JSONL),
    );

    let flags_set = mint_flags_set_bendl();
    write_fixture("flags_set.bendl", &flags_set);

    let unknown_flags = flip_unknown_flag_bits(flags_set);
    write_fixture("unknown_flags.bendl", &unknown_flags);

    // Also commit the canonical sources alongside so a human can read what the fixtures represent
    // without invoking the codec.
    write_fixture("source.jsonl", CANONICAL_JSONL.as_bytes());
    write_fixture("source_twodelta.jsonl", TWODELTA_CANONICAL_JSONL.as_bytes());
    write_fixture("source_graph.json", CANONICAL_GRAPH_JSON.as_bytes());
    write_fixture("source_metadata.json", CANONICAL_METADATA_JSON.as_bytes());

    // Print a checklist so the engineer regenerating fixtures sees what landed.
    eprintln!("Wrote v1.0.0 fixtures to {:?}", fixtures_dir());
}

/// The canonical assignments rendered in PCompress's zero-based line format (one JSON array per
/// line), the input the foreign `pcompress` encoder consumes. District ids are CANONICAL_JSONL's
/// minus one. The `ben pcompress` bridge transcodes ids unchanged (both formats are zero-based), so
/// converting the fixture yields the zero-based form of CANONICAL_JSONL.
const CANONICAL_PCOMPRESS_INPUT: &str = "\
[0,0,1,1]
[0,1,0,1]
[0,0,0,1]
[0,0,0,1]
[1,1,1,0]
";

#[test]
#[ignore = "mints only the foreign-format pcompress interop fixture; never run as part of normal CI"]
fn generate_pcompress_interop_fixture() {
    // Minted by the *foreign implementation*: the `pcompress` crates.io dependency is mggg's real
    // encoder, so these bytes pin interop with genuine PCompress output rather than with this
    // workspace's own rendering of the format. Re-minting is legitimate only if the pinned
    // `pcompress` dependency version changes its wire format, which would itself be an interop
    // event worth a dedicated PR.
    let mut reader = BufReader::new(CANONICAL_PCOMPRESS_INPUT.as_bytes());
    let mut writer = std::io::BufWriter::new(Vec::new());
    pcompress::encode::encode(&mut reader, &mut writer, false);
    let out = writer.into_inner().expect("flush pcompress fixture bytes");
    write_fixture("interop.pcompress", &out);

    eprintln!("Wrote interop.pcompress to {:?}", fixtures_dir());
}
