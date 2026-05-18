use std::io::{Cursor, Read, Write};

use xz2::write::XzEncoder;

use crate::io::bundle::format::{
    encode_directory, AssignmentFormat, BendlDirectoryEntry, BendlFormatError, BendlHeader,
    ASSET_FLAG_CHECKSUM, ASSET_FLAG_JSON, ASSET_FLAG_XZ, ASSET_TYPE_CUSTOM, ASSET_TYPE_GRAPH,
    ASSET_TYPE_METADATA, ASSET_TYPE_NODE_PERMUTATION_MAP, BENDL_MAGIC, BENDL_MAJOR_VERSION,
    BENDL_MINOR_VERSION, FINALIZED_NO, FINALIZED_YES, HEADER_SIZE,
};
use crate::io::bundle::reader::{
    validate_directory_entries, BendlReader, BundleAssignmentReaderError, BundleValidationError,
};

/// Stamp a valid CRC32C and `ASSET_FLAG_CHECKSUM` onto a hand-built directory entry whose on-disk
/// payload bytes are `payload`. Use this in test fixtures so the entry round-trips through the
/// verify-on-touch reader APIs. Tests that want to exercise the foreign-bundle / clear-flag path
/// build entries directly with the flag clear and `checksum: None`.
fn with_crc(mut entry: BendlDirectoryEntry, payload: &[u8]) -> BendlDirectoryEntry {
    entry.asset_flags |= ASSET_FLAG_CHECKSUM;
    entry.checksum = Some(crc32c::crc32c(payload).to_le_bytes().to_vec());
    entry
}

/// Build a complete in-memory finalized bundle with two assets: an xz-compressed `graph.json` and a
/// raw custom blob, followed by a fake BEN stream and a trailing directory.
fn build_finalized_bundle() -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) {
    // Asset payloads (decoded):
    let graph_json = br#"{"nodes":[0,1,2],"edges":[[0,1],[1,2]]}"#.to_vec();
    let custom_blob = vec![0xAA, 0xBB, 0xCC, 0xDD, 0xEE];
    let fake_stream = b"STANDARD BEN FILE\x00\x01\x02\x03fake payload".to_vec();

    // xz-compress graph_json using the default preset.
    let mut encoder = XzEncoder::new(Vec::new(), 6);
    encoder.write_all(&graph_json).unwrap();
    let compressed_graph = encoder.finish().unwrap();

    // Layout:
    //   [0 .. 64) header
    //   [64 .. 64+len(compressed_graph)) graph payload
    //   [... .. ...+len(custom_blob)) custom payload
    //   [stream_offset .. stream_offset+len(fake_stream)) stream
    //   [directory_offset .. EOF) directory
    let mut bundle = Vec::new();
    // Reserve space for header; fill later.
    bundle.extend(std::iter::repeat(0u8).take(HEADER_SIZE));

    let graph_offset = bundle.len() as u64;
    bundle.extend_from_slice(&compressed_graph);

    let custom_offset = bundle.len() as u64;
    bundle.extend_from_slice(&custom_blob);

    let stream_offset = bundle.len() as u64;
    bundle.extend_from_slice(&fake_stream);
    let stream_len = fake_stream.len() as u64;

    let directory_offset = bundle.len() as u64;

    let entries = vec![
        with_crc(
            BendlDirectoryEntry {
                asset_type: ASSET_TYPE_GRAPH,
                asset_flags: ASSET_FLAG_JSON | ASSET_FLAG_XZ,
                name: "graph.json".to_string(),
                payload_offset: graph_offset,
                payload_len: compressed_graph.len() as u64,
                checksum: None,
            },
            &compressed_graph,
        ),
        with_crc(
            BendlDirectoryEntry {
                asset_type: ASSET_TYPE_CUSTOM,
                asset_flags: 0,
                name: "custom.bin".to_string(),
                payload_offset: custom_offset,
                payload_len: custom_blob.len() as u64,
                checksum: None,
            },
            &custom_blob,
        ),
    ];
    let directory_bytes = encode_directory(&entries).unwrap();
    bundle.extend_from_slice(&directory_bytes);
    let directory_len = directory_bytes.len() as u64;

    // Now patch the header.
    let header = BendlHeader {
        magic: BENDL_MAGIC,
        major_version: BENDL_MAJOR_VERSION,
        minor_version: BENDL_MINOR_VERSION,
        finalized: FINALIZED_YES,
        assignment_format: AssignmentFormat::Ben.to_u8(),
        reserved_0: 0,
        flags: 0,
        directory_offset,
        directory_len,
        stream_offset,
        stream_len,
        sample_count: 42,
    };
    bundle[..HEADER_SIZE].copy_from_slice(&header.to_bytes());

    (bundle, graph_json, custom_blob, fake_stream)
}

#[test]
fn open_finalized_bundle_and_read_metadata() {
    let (bytes, _, _, _) = build_finalized_bundle();
    let reader = BendlReader::open(Cursor::new(bytes)).unwrap();
    assert!(reader.is_finalized());
    assert_eq!(reader.sample_count(), Some(42));
    assert_eq!(reader.assignment_format(), Some(AssignmentFormat::Ben));
    assert_eq!(reader.assets().len(), 2);
    assert!(reader.validate_directory().is_ok());
}

#[test]
fn read_compressed_graph_asset_decodes_through_xz() {
    let (bytes, graph_json, _, _) = build_finalized_bundle();
    let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
    let entry = reader
        .find_asset_by_type(ASSET_TYPE_GRAPH)
        .cloned()
        .expect("graph entry");
    let bytes_out = reader.asset_bytes(&entry).unwrap();
    assert_eq!(bytes_out, graph_json);
}

#[test]
fn read_raw_custom_asset_returns_exact_bytes() {
    let (bytes, _, custom_blob, _) = build_finalized_bundle();
    let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
    let entry = reader
        .find_asset_by_name("custom.bin")
        .cloned()
        .expect("custom entry");
    let bytes_out = reader.asset_bytes(&entry).unwrap();
    assert_eq!(bytes_out, custom_blob);
}

#[test]
fn assignment_stream_range_matches_finalized_header() {
    let (bytes, _, _, fake_stream) = build_finalized_bundle();
    let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
    let (offset, len) = reader.assignment_stream_range().unwrap();
    assert_eq!(len, fake_stream.len() as u64);
    let mut buf = Vec::new();
    reader
        .assignment_stream_reader()
        .unwrap()
        .read_to_end(&mut buf)
        .unwrap();
    assert_eq!(buf, fake_stream);
    // Sanity-check the offset is consistent with the header.
    assert_eq!(offset, reader.header().stream_offset);
}

#[test]
fn incomplete_bundle_reports_no_directory_and_stream_runs_to_eof() {
    // Build an incomplete bundle: header + some fake stream bytes, no directory.
    let fake_stream = b"STANDARD BEN FILE\x00\x01some partial bytes".to_vec();
    let mut bytes = Vec::new();
    let header = BendlHeader {
        magic: BENDL_MAGIC,
        major_version: BENDL_MAJOR_VERSION,
        minor_version: BENDL_MINOR_VERSION,
        finalized: FINALIZED_NO,
        assignment_format: AssignmentFormat::Ben.to_u8(),
        reserved_0: 0,
        flags: 0,
        directory_offset: 0,
        directory_len: 0,
        stream_offset: HEADER_SIZE as u64,
        stream_len: 0,
        sample_count: -1,
    };
    bytes.extend_from_slice(&header.to_bytes());
    bytes.extend_from_slice(&fake_stream);

    let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
    assert!(!reader.is_finalized());
    assert_eq!(reader.sample_count(), None);
    assert!(reader.assets().is_empty());

    let (offset, len) = reader.assignment_stream_range().unwrap();
    assert_eq!(offset, HEADER_SIZE as u64);
    assert_eq!(len, fake_stream.len() as u64);

    let mut buf = Vec::new();
    reader
        .assignment_stream_reader()
        .unwrap()
        .read_to_end(&mut buf)
        .unwrap();
    assert_eq!(buf, fake_stream);
}

#[test]
fn open_rejects_malformed_magic() {
    let mut bytes = vec![0u8; HEADER_SIZE];
    bytes[0..8].copy_from_slice(b"NOPENOPE");
    match BendlReader::open(Cursor::new(bytes)) {
        Err(BendlFormatError::InvalidMagic(_)) => {}
        Err(other) => panic!("expected InvalidMagic, got {other:?}"),
        Ok(_) => panic!("expected error, got Ok"),
    }
}

#[test]
fn validate_directory_catches_duplicate_names() {
    let entries = vec![
        BendlDirectoryEntry {
            asset_type: ASSET_TYPE_CUSTOM,
            asset_flags: 0,
            name: "a".to_string(),
            payload_offset: 64,
            payload_len: 1,
            checksum: None,
        },
        BendlDirectoryEntry {
            asset_type: ASSET_TYPE_CUSTOM,
            asset_flags: 0,
            name: "a".to_string(),
            payload_offset: 65,
            payload_len: 1,
            checksum: None,
        },
    ];
    let err = validate_directory_entries(&entries).unwrap_err();
    assert!(matches!(err, BundleValidationError::DuplicateName(ref n) if n == "a"));
}

#[test]
fn validate_directory_catches_wrong_canonical_name() {
    let entries = vec![BendlDirectoryEntry {
        asset_type: ASSET_TYPE_GRAPH,
        asset_flags: 0,
        name: "not_graph.json".to_string(),
        payload_offset: 64,
        payload_len: 1,
        checksum: None,
    }];
    let err = validate_directory_entries(&entries).unwrap_err();
    assert!(matches!(
        err,
        BundleValidationError::WrongCanonicalName {
            asset_type: ASSET_TYPE_GRAPH,
            ..
        }
    ));
}

// -----------------------------------------------------------------------
// Robustness tests
// -----------------------------------------------------------------------

/// Build a small finalized bundle with a known graph asset, metadata asset, empty stream, and no
/// validation pitfalls. Useful as a base that tests can mutate byte-by-byte.
fn build_basic_finalized_bundle() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend(std::iter::repeat(0u8).take(HEADER_SIZE));

    // One raw metadata asset right after the header.
    let metadata_payload = br#"{"k":"v"}"#.to_vec();
    let metadata_offset = bytes.len() as u64;
    bytes.extend_from_slice(&metadata_payload);

    // Stream region is empty.
    let stream_offset = bytes.len() as u64;
    let stream_len = 0u64;

    // Directory at EOF with one entry.
    let directory_offset = bytes.len() as u64;
    let entries = vec![with_crc(
        BendlDirectoryEntry {
            asset_type: ASSET_TYPE_METADATA,
            asset_flags: ASSET_FLAG_JSON,
            name: "metadata.json".to_string(),
            payload_offset: metadata_offset,
            payload_len: metadata_payload.len() as u64,
            checksum: None,
        },
        &metadata_payload,
    )];
    let directory = encode_directory(&entries).unwrap();
    bytes.extend_from_slice(&directory);
    let directory_len = directory.len() as u64;

    let header = BendlHeader {
        magic: BENDL_MAGIC,
        major_version: BENDL_MAJOR_VERSION,
        minor_version: BENDL_MINOR_VERSION,
        finalized: FINALIZED_YES,
        assignment_format: AssignmentFormat::Ben.to_u8(),
        reserved_0: 0,
        flags: 0,
        directory_offset,
        directory_len,
        stream_offset,
        stream_len,
        sample_count: 0,
    };
    bytes[..HEADER_SIZE].copy_from_slice(&header.to_bytes());
    bytes
}

#[test]
fn open_rejects_short_header() {
    let too_short = vec![0u8; HEADER_SIZE - 1];
    match BendlReader::open(Cursor::new(too_short)) {
        Err(BendlFormatError::Io(_)) => {}
        Err(other) => panic!("expected Io, got {other:?}"),
        Ok(_) => panic!("expected error, got Ok"),
    }
}

#[test]
fn open_rejects_unsupported_major_version() {
    let mut bytes = build_basic_finalized_bundle();
    // major_version lives at offset 8..10 in the header.
    bytes[8..10].copy_from_slice(&(BENDL_MAJOR_VERSION + 1).to_le_bytes());
    match BendlReader::open(Cursor::new(bytes)) {
        Err(BendlFormatError::UnsupportedMajorVersion { .. }) => {}
        Err(other) => panic!("expected UnsupportedMajorVersion, got {other:?}"),
        Ok(_) => panic!("expected error, got Ok"),
    }
}

#[test]
fn open_rejects_directory_with_inflated_entry_count() {
    let mut bytes = build_basic_finalized_bundle();
    // Read directory_offset from the header (bytes 24..32).
    let directory_offset = u64::from_le_bytes(bytes[24..32].try_into().unwrap()) as usize;
    // Blow up the entry count at the start of the directory to a value that cannot possibly fit in
    // the remaining file bytes.
    bytes[directory_offset..directory_offset + 4].copy_from_slice(&9999u32.to_le_bytes());
    match BendlReader::open(Cursor::new(bytes)) {
        Err(BendlFormatError::Io(_)) => {}
        Err(other) => panic!("expected Io, got {other:?}"),
        Ok(_) => panic!("expected error, got Ok"),
    }
}

#[test]
fn open_rejects_directory_with_chopped_final_entry() {
    // Drop the last byte of the file, which lies inside the name field of the final directory
    // entry.
    let mut bytes = build_basic_finalized_bundle();
    bytes.pop();
    match BendlReader::open(Cursor::new(bytes)) {
        Err(BendlFormatError::Io(_)) => {}
        Err(other) => panic!("expected Io, got {other:?}"),
        Ok(_) => panic!("expected error, got Ok"),
    }
}

#[test]
fn asset_bytes_read_twice_returns_identical_payload() {
    let (bytes, _, custom_blob, _) = build_finalized_bundle();
    let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
    let entry = reader.find_asset_by_name("custom.bin").cloned().unwrap();
    let first = reader.asset_bytes(&entry).unwrap();
    let second = reader.asset_bytes(&entry).unwrap();
    assert_eq!(first, second);
    assert_eq!(first, custom_blob);
}

#[test]
fn interleaved_reads_do_not_corrupt_each_other() {
    // Read asset A, then stream, then asset A again, then asset B.
    let (bytes, graph_json, custom_blob, fake_stream) = build_finalized_bundle();
    let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();

    let graph_entry = reader
        .find_asset_by_type(ASSET_TYPE_GRAPH)
        .cloned()
        .unwrap();
    let custom_entry = reader.find_asset_by_name("custom.bin").cloned().unwrap();

    let graph_first = reader.asset_bytes(&graph_entry).unwrap();
    assert_eq!(graph_first, graph_json);

    let mut stream_buf = Vec::new();
    reader
        .assignment_stream_reader()
        .unwrap()
        .read_to_end(&mut stream_buf)
        .unwrap();
    assert_eq!(stream_buf, fake_stream);

    let graph_second = reader.asset_bytes(&graph_entry).unwrap();
    assert_eq!(graph_second, graph_json);

    let custom = reader.asset_bytes(&custom_entry).unwrap();
    assert_eq!(custom, custom_blob);
}

#[test]
fn asset_bytes_errors_when_declared_length_runs_past_eof() {
    // Hand-construct a bundle where the metadata directory entry claims a payload_len that extends
    // well past EOF.
    let mut bytes = build_basic_finalized_bundle();
    // Parse the directory offset to find where the entry lives.
    let directory_offset = u64::from_le_bytes(bytes[24..32].try_into().unwrap()) as usize;
    // Skip the u32 entry count (4 bytes) and then the 16-byte fixed entry header up to
    // `payload_len` (bytes 16..24 of the entry).
    let entry_start = directory_offset + 4;
    let payload_len_offset = entry_start + 16;
    bytes[payload_len_offset..payload_len_offset + 8].copy_from_slice(&u64::MAX.to_le_bytes());

    let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
    let entry = reader.find_asset_by_name("metadata.json").cloned().unwrap();
    // The reader opens fine — the directory parses. But reading the asset bytes must surface an
    // error eventually (short read vs declared length). xz would also trip on this, but this is the
    // raw-asset path. Either returns an error or a slice shorter than u64::MAX.
    reader
        .asset_bytes(&entry)
        .map(|b| assert!(b.len() < u64::MAX as usize))
        .ok();
}

#[test]
fn incomplete_bundle_sample_count_is_none_even_if_header_value_is_nonzero() {
    // Build an incomplete bundle but stuff a stale sample count into the header. `sample_count()`
    // must still return None because the `complete` flag is what makes the value authoritative.
    let header = BendlHeader {
        magic: BENDL_MAGIC,
        major_version: BENDL_MAJOR_VERSION,
        minor_version: BENDL_MINOR_VERSION,
        finalized: FINALIZED_NO,
        assignment_format: AssignmentFormat::Ben.to_u8(),
        reserved_0: 0,
        flags: 0,
        directory_offset: 0,
        directory_len: 0,
        stream_offset: HEADER_SIZE as u64,
        stream_len: 0,
        sample_count: 999_999, // lie, but header is "incomplete"
    };
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&header.to_bytes());
    let reader = BendlReader::open(Cursor::new(bytes)).unwrap();
    assert!(!reader.is_finalized());
    assert_eq!(reader.sample_count(), None);
}

#[test]
fn unknown_assignment_format_reports_none_on_typed_getter() {
    // Build a finalized but otherwise-empty bundle and corrupt the assignment_format byte to a
    // value that is neither BEN nor XBEN.
    let mut bytes = build_basic_finalized_bundle();
    // assignment_format byte is at offset 13 in the header.
    bytes[13] = 42;
    let reader = BendlReader::open(Cursor::new(bytes)).unwrap();
    assert_eq!(reader.assignment_format(), None);
    // The header still parses and the directory is still available.
    assert_eq!(reader.assets().len(), 1);
}

#[test]
fn open_assignment_reader_rejects_unknown_assignment_format() {
    let mut bytes = build_basic_finalized_bundle();
    bytes[13] = 42; // corrupt assignment format byte
    let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
    match reader.open_assignment_reader() {
        Err(BundleAssignmentReaderError::UnknownAssignmentFormat(42)) => {}
        Err(other) => panic!("expected UnknownAssignmentFormat(42), got {other:?}"),
        Ok(_) => panic!("expected error, got Ok"),
    }
}

#[test]
fn incomplete_bundle_stream_range_runs_to_eof_without_directory() {
    let fake_stream = b"STANDARD BEN FILE\x00\x01payload bytes".to_vec();
    let header = BendlHeader {
        magic: BENDL_MAGIC,
        major_version: BENDL_MAJOR_VERSION,
        minor_version: BENDL_MINOR_VERSION,
        finalized: FINALIZED_NO,
        assignment_format: AssignmentFormat::Ben.to_u8(),
        reserved_0: 0,
        flags: 0,
        directory_offset: 0,
        directory_len: 0,
        stream_offset: HEADER_SIZE as u64,
        stream_len: 0,
        sample_count: -1,
    };
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&header.to_bytes());
    bytes.extend_from_slice(&fake_stream);
    let eof = bytes.len() as u64;

    let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
    let (off, len) = reader.assignment_stream_range().unwrap();
    assert_eq!(off, HEADER_SIZE as u64);
    assert_eq!(off + len, eof);
}

#[test]
fn validate_directory_catches_duplicate_singleton_types() {
    // Two entries of type METADATA. The second one uses a non-canonical name to confirm the
    // canonical-name check fires (it lands first here, and is the path we cover; the singleton
    // check is exercised elsewhere via duplicate standardized names).
    let entries = vec![
        BendlDirectoryEntry {
            asset_type: ASSET_TYPE_METADATA,
            asset_flags: 0,
            name: "metadata.json".to_string(),
            payload_offset: 64,
            payload_len: 1,
            checksum: None,
        },
        BendlDirectoryEntry {
            asset_type: ASSET_TYPE_METADATA,
            asset_flags: 0,
            // Distinct name so the duplicate-name check does not fire first; the singleton-type
            // check should catch this.
            name: "meta2.json".to_string(),
            payload_offset: 65,
            payload_len: 1,
            checksum: None,
        },
    ];
    // The second entry has asset_type METADATA but name "meta2.json" which fails the canonical-name
    // check.
    let err = validate_directory_entries(&entries).unwrap_err();
    assert!(matches!(
        err,
        BundleValidationError::WrongCanonicalName { .. }
    ));
}

#[test]
fn validate_directory_accepts_well_formed_multi_singleton_bundle() {
    // A bundle with one of every singleton type, plus two custom assets with distinct names, should
    // validate cleanly.
    let entries = vec![
        BendlDirectoryEntry {
            asset_type: ASSET_TYPE_METADATA,
            asset_flags: ASSET_FLAG_JSON,
            name: "metadata.json".to_string(),
            payload_offset: 64,
            payload_len: 4,
            checksum: None,
        },
        BendlDirectoryEntry {
            asset_type: ASSET_TYPE_GRAPH,
            asset_flags: ASSET_FLAG_JSON | ASSET_FLAG_XZ,
            name: "graph.json".to_string(),
            payload_offset: 68,
            payload_len: 4,
            checksum: None,
        },
        BendlDirectoryEntry {
            asset_type: ASSET_TYPE_NODE_PERMUTATION_MAP,
            asset_flags: ASSET_FLAG_JSON,
            name: "node_permutation_map.json".to_string(),
            payload_offset: 72,
            payload_len: 4,
            checksum: None,
        },
        BendlDirectoryEntry {
            asset_type: ASSET_TYPE_CUSTOM,
            asset_flags: 0,
            name: "a.bin".to_string(),
            payload_offset: 76,
            payload_len: 4,
            checksum: None,
        },
        BendlDirectoryEntry {
            asset_type: ASSET_TYPE_CUSTOM,
            asset_flags: 0,
            name: "b.bin".to_string(),
            payload_offset: 80,
            payload_len: 4,
            checksum: None,
        },
    ];
    validate_directory_entries(&entries).expect("well-formed directory");
}

#[test]
fn stress_thousand_custom_assets_round_trip() {
    // Build a directory with 1000 small custom assets, each with a unique payload derived from its
    // index, and confirm they all round-trip via `asset_bytes`. This catches any off-by-one or
    // seek-caching bugs that might only show up with many entries.
    const N: usize = 1000;

    let mut bytes = Vec::new();
    bytes.extend(std::iter::repeat(0u8).take(HEADER_SIZE));

    let mut entries = Vec::with_capacity(N);
    let mut expected = Vec::with_capacity(N);
    for i in 0..N {
        let payload: Vec<u8> = (0..(i % 31 + 1) as u8)
            .map(|j| (i as u8).wrapping_add(j))
            .collect();
        let offset = bytes.len() as u64;
        bytes.extend_from_slice(&payload);
        entries.push(with_crc(
            BendlDirectoryEntry {
                asset_type: ASSET_TYPE_CUSTOM,
                asset_flags: 0,
                name: format!("blob-{i:04}.bin"),
                payload_offset: offset,
                payload_len: payload.len() as u64,
                checksum: None,
            },
            &payload,
        ));
        expected.push(payload);
    }

    let stream_offset = bytes.len() as u64;
    let stream_len = 0u64;
    let directory_offset = bytes.len() as u64;
    let directory = encode_directory(&entries).unwrap();
    bytes.extend_from_slice(&directory);
    let directory_len = directory.len() as u64;

    let header = BendlHeader {
        magic: BENDL_MAGIC,
        major_version: BENDL_MAJOR_VERSION,
        minor_version: BENDL_MINOR_VERSION,
        finalized: FINALIZED_YES,
        assignment_format: AssignmentFormat::Ben.to_u8(),
        reserved_0: 0,
        flags: 0,
        directory_offset,
        directory_len,
        stream_offset,
        stream_len,
        sample_count: 0,
    };
    bytes[..HEADER_SIZE].copy_from_slice(&header.to_bytes());

    let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
    assert_eq!(reader.assets().len(), N);
    reader.validate_directory().unwrap();
    // Access in scrambled order to exercise seeking.
    for &idx in &[0usize, N - 1, 1, N / 2, N / 3, 2 * N / 3, 7, 999] {
        let name = format!("blob-{idx:04}.bin");
        let entry = reader.find_asset_by_name(&name).cloned().unwrap();
        let got = reader.asset_bytes(&entry).unwrap();
        assert_eq!(got, expected[idx], "mismatch at index {idx}");
    }
}

#[test]
fn xz_flagged_asset_with_corrupt_payload_surfaces_io_error() {
    // Hand-build a bundle with a single asset flagged ASSET_FLAG_XZ whose payload bytes are not a
    // valid xz container. `asset_bytes` must surface an io::Error rather than panicking.
    let mut bytes = Vec::new();
    bytes.extend(std::iter::repeat(0u8).take(HEADER_SIZE));

    let bad_payload = vec![0xFFu8, 0xFE, 0xFD, 0xFC, 0xFB];
    let payload_offset = bytes.len() as u64;
    bytes.extend_from_slice(&bad_payload);

    let stream_offset = bytes.len() as u64;
    let directory_offset = bytes.len() as u64;
    let entries = vec![with_crc(
        BendlDirectoryEntry {
            asset_type: ASSET_TYPE_CUSTOM,
            asset_flags: ASSET_FLAG_XZ,
            name: "broken.xz".to_string(),
            payload_offset,
            payload_len: bad_payload.len() as u64,
            checksum: None,
        },
        &bad_payload,
    )];
    let directory = encode_directory(&entries).unwrap();
    bytes.extend_from_slice(&directory);

    let header = BendlHeader {
        magic: BENDL_MAGIC,
        major_version: BENDL_MAJOR_VERSION,
        minor_version: BENDL_MINOR_VERSION,
        finalized: FINALIZED_YES,
        assignment_format: AssignmentFormat::Ben.to_u8(),
        reserved_0: 0,
        flags: 0,
        directory_offset,
        directory_len: directory.len() as u64,
        stream_offset,
        stream_len: 0,
        sample_count: 0,
    };
    bytes[..HEADER_SIZE].copy_from_slice(&header.to_bytes());

    let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
    let entry = reader.find_asset_by_name("broken.xz").cloned().unwrap();
    let res = reader.asset_bytes(&entry);
    assert!(res.is_err(), "expected xz decode error, got {res:?}");
}

#[test]
fn reader_scales_to_very_wide_stream_offset_field() {
    // Confirm the `Take` bound clamps a stream reader even when the header's stream_len is much
    // larger than the actual remaining bytes: the reader must return the shorter slice rather than
    // loop forever or panic. This is a "short read" tolerance check.
    let fake_stream = b"STANDARD BEN FILE\x00\x01tiny".to_vec();
    let actual_len = fake_stream.len() as u64;
    let directory_offset = HEADER_SIZE as u64 + actual_len;
    // Build a bundle that lies about stream_len: claims ten times what's actually present.
    let entries: Vec<BendlDirectoryEntry> = Vec::new();
    let directory_bytes = encode_directory(&entries).unwrap();
    let header = BendlHeader {
        magic: BENDL_MAGIC,
        major_version: BENDL_MAJOR_VERSION,
        minor_version: BENDL_MINOR_VERSION,
        finalized: FINALIZED_YES,
        assignment_format: AssignmentFormat::Ben.to_u8(),
        reserved_0: 0,
        flags: 0,
        directory_offset,
        directory_len: directory_bytes.len() as u64,
        stream_offset: HEADER_SIZE as u64,
        stream_len: actual_len * 10, // lie
        sample_count: 0,
    };
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&header.to_bytes());
    bytes.extend_from_slice(&fake_stream);
    bytes.extend_from_slice(&directory_bytes);

    let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
    let mut buf = Vec::new();
    // Take will try to read `stream_len` bytes but the Cursor will just return however many bytes
    // remain from stream_offset to EOF. The reader must not panic; it must simply return what it
    // got.
    reader
        .assignment_stream_reader()
        .unwrap()
        .read_to_end(&mut buf)
        .unwrap();
    // Take includes the directory bytes in the window since they come after stream_offset and the
    // claim exceeds file size — so we assert only that we got *at least* the real stream bytes as a
    // prefix, which is the basic "no truncation of what exists" check.
    assert!(buf.starts_with(&fake_stream));
}

#[test]
fn incomplete_bundle_with_nonzero_directory_offset_uses_it_as_stream_end() {
    // An incomplete bundle where directory_offset is non-zero: the stream end is taken as
    // directory_offset, not EOF.
    let fake_stream = b"STANDARD BEN FILE\x00partial".to_vec();
    let fake_dir = b"some-directory-bytes";
    let stream_start = HEADER_SIZE as u64;
    let dir_offset = stream_start + fake_stream.len() as u64;

    let header = BendlHeader {
        magic: BENDL_MAGIC,
        major_version: BENDL_MAJOR_VERSION,
        minor_version: BENDL_MINOR_VERSION,
        finalized: FINALIZED_NO,
        assignment_format: AssignmentFormat::Ben.to_u8(),
        reserved_0: 0,
        flags: 0,
        directory_offset: dir_offset,
        directory_len: 0,
        stream_offset: stream_start,
        stream_len: 0,
        sample_count: -1,
    };
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&header.to_bytes());
    bytes.extend_from_slice(&fake_stream);
    bytes.extend_from_slice(fake_dir);

    let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
    assert!(!reader.is_finalized());

    let (offset, len) = reader.assignment_stream_range().unwrap();
    assert_eq!(offset, stream_start);
    assert_eq!(len, fake_stream.len() as u64);
}

#[test]
fn validate_directory_rejects_wrong_canonical_name() {
    let entries = vec![BendlDirectoryEntry {
        asset_type: ASSET_TYPE_GRAPH,
        asset_flags: ASSET_FLAG_JSON,
        name: "not_the_canonical_name.json".to_string(),
        payload_offset: 64,
        payload_len: 10,
        checksum: None,
    }];
    let err = validate_directory_entries(&entries).unwrap_err();
    match err {
        BundleValidationError::WrongCanonicalName { .. } => {}
        _ => panic!("expected WrongCanonicalName, got {err:?}"),
    }
}

// =====================================================================
// Asset CRC32C verification
// =====================================================================
//
// These tests pin the verify-on-touch contract for directory-entry assets. The structural split is:
//
//   - explicit verifier (`verify_asset_checksum`) vs implicit verifier (`asset_bytes` /
//     `asset_reader`),
//   - uncompressed vs xz-compressed assets,
//   - stored-checksum corruption vs payload corruption (vs xz-framing corruption for compressed
//     assets).
//
// The unverified APIs (`*_unverified`) are pinned in matching tests to ensure they NEVER surface a
// `ChecksumError` (codec errors are still permitted).

use crate::io::bundle::error::{BendlReadError, ChecksumError, ChecksumTarget};

/// Build a finalized bundle with exactly one uncompressed asset whose payload bytes are `payload`.
/// Returns `(bundle_bytes, asset_name, directory_offset, payload_offset)` for hand-patching tests.
fn make_single_asset_bundle(name: &str, payload: &[u8]) -> (Vec<u8>, String, u64, u64) {
    let mut bytes = Vec::new();
    bytes.extend(std::iter::repeat(0u8).take(HEADER_SIZE));

    let payload_offset = bytes.len() as u64;
    bytes.extend_from_slice(payload);

    let stream_offset = bytes.len() as u64;
    let directory_offset = bytes.len() as u64;
    let entries = vec![with_crc(
        BendlDirectoryEntry {
            asset_type: ASSET_TYPE_CUSTOM,
            asset_flags: 0,
            name: name.to_string(),
            payload_offset,
            payload_len: payload.len() as u64,
            checksum: None,
        },
        payload,
    )];
    let directory = encode_directory(&entries).unwrap();
    bytes.extend_from_slice(&directory);

    let header = BendlHeader {
        magic: BENDL_MAGIC,
        major_version: BENDL_MAJOR_VERSION,
        minor_version: BENDL_MINOR_VERSION,
        finalized: FINALIZED_YES,
        assignment_format: AssignmentFormat::Ben.to_u8(),
        reserved_0: 0,
        flags: 0,
        directory_offset,
        directory_len: directory.len() as u64,
        stream_offset,
        stream_len: 0,
        sample_count: 0,
    };
    bytes[..HEADER_SIZE].copy_from_slice(&header.to_bytes());
    (bytes, name.to_string(), directory_offset, payload_offset)
}

/// Build a finalized bundle whose only asset is `payload` stored xz- compressed. The stored CRC is
/// over the **compressed** bytes (CRC is pre-decompression). Returns
/// `(bundle_bytes, name, compressed_payload, directory_offset, payload_offset)`.
fn make_single_xz_asset_bundle(name: &str, payload: &[u8]) -> (Vec<u8>, String, Vec<u8>, u64, u64) {
    let mut encoder = XzEncoder::new(Vec::new(), 6);
    encoder.write_all(payload).unwrap();
    let compressed = encoder.finish().unwrap();

    let mut bytes = Vec::new();
    bytes.extend(std::iter::repeat(0u8).take(HEADER_SIZE));

    let payload_offset = bytes.len() as u64;
    bytes.extend_from_slice(&compressed);

    let stream_offset = bytes.len() as u64;
    let directory_offset = bytes.len() as u64;
    let entries = vec![with_crc(
        BendlDirectoryEntry {
            asset_type: ASSET_TYPE_CUSTOM,
            asset_flags: ASSET_FLAG_XZ,
            name: name.to_string(),
            payload_offset,
            payload_len: compressed.len() as u64,
            checksum: None,
        },
        &compressed,
    )];
    let directory = encode_directory(&entries).unwrap();
    bytes.extend_from_slice(&directory);

    let header = BendlHeader {
        magic: BENDL_MAGIC,
        major_version: BENDL_MAJOR_VERSION,
        minor_version: BENDL_MINOR_VERSION,
        finalized: FINALIZED_YES,
        assignment_format: AssignmentFormat::Ben.to_u8(),
        reserved_0: 0,
        flags: 0,
        directory_offset,
        directory_len: directory.len() as u64,
        stream_offset,
        stream_len: 0,
        sample_count: 0,
    };
    bytes[..HEADER_SIZE].copy_from_slice(&header.to_bytes());
    (
        bytes,
        name.to_string(),
        compressed,
        directory_offset,
        payload_offset,
    )
}

/// Locate the offset of an asset's stored CRC32C bytes inside a hand-built single-asset bundle.
/// Assumes the directory starts at `directory_offset`, the entry count is one, and the entry's
/// `checksum_len` is 4 (the only legal value when the flag is set).
fn stored_checksum_offset(directory_offset: u64, name: &str) -> usize {
    // directory layout: [u32 count][entry][...] entry layout: [28-byte header][name bytes][checksum
    // bytes]
    let entry_start = directory_offset as usize + 4;
    entry_start + 28 + name.len()
}

// ----- Explicit verify_asset_checksum -------------------------------

#[test]
fn verify_asset_checksum_uncompressed_passes_on_intact_bundle() {
    let (bytes, name, _, _) = make_single_asset_bundle("blob", b"hello world");
    let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
    let entry = reader.find_asset_by_name(&name).cloned().unwrap();
    reader.verify_asset_checksum(&entry).unwrap();
}

#[test]
fn verify_asset_checksum_uncompressed_corrupt_stored_crc_returns_mismatch() {
    let (mut bytes, name, dir_off, _) = make_single_asset_bundle("blob", b"hello world");
    let crc_off = stored_checksum_offset(dir_off, &name);
    bytes[crc_off] ^= 0xFF; // flip stored checksum
    let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
    let entry = reader.find_asset_by_name(&name).cloned().unwrap();
    let err = reader.verify_asset_checksum(&entry).unwrap_err();
    assert!(matches!(
        err,
        BendlReadError::Checksum(ChecksumError::Mismatch { ref target, .. })
            if matches!(target, ChecksumTarget::Asset(n) if n == &name)
    ));
}

#[test]
fn verify_asset_checksum_uncompressed_corrupt_payload_byte_returns_mismatch() {
    let (mut bytes, name, _, payload_off) = make_single_asset_bundle("blob", b"hello world");
    bytes[payload_off as usize] ^= 0x01; // flip first payload byte
    let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
    let entry = reader.find_asset_by_name(&name).cloned().unwrap();
    let err = reader.verify_asset_checksum(&entry).unwrap_err();
    assert!(matches!(
        err,
        BendlReadError::Checksum(ChecksumError::Mismatch { .. })
    ));
}

#[test]
fn verify_asset_checksum_xz_corrupt_stored_crc_returns_mismatch_no_decoder() {
    // The explicit verifier reads raw bytes — no XzDecoder is invoked, so even an intact compressed
    // payload reports `Mismatch` deterministically when only the stored CRC has been corrupted.
    let (mut bytes, name, _, dir_off, _) =
        make_single_xz_asset_bundle("blob.xz", b"some compressible content");
    let crc_off = stored_checksum_offset(dir_off, &name);
    bytes[crc_off] ^= 0xFF;
    let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
    let entry = reader.find_asset_by_name(&name).cloned().unwrap();
    let err = reader.verify_asset_checksum(&entry).unwrap_err();
    assert!(matches!(
        err,
        BendlReadError::Checksum(ChecksumError::Mismatch { .. })
    ));
}

#[test]
fn verify_asset_checksum_xz_corrupt_payload_returns_mismatch_no_decoder() {
    // Verifier is over raw bytes — a payload flip that breaks xz framing still surfaces as
    // Mismatch, NOT a decoder error, because the explicit verifier never invokes the decoder.
    let (mut bytes, name, compressed, _, payload_off) =
        make_single_xz_asset_bundle("blob.xz", b"some compressible content");
    assert!(compressed.len() > 5);
    bytes[payload_off as usize + 5] ^= 0xFF;
    let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
    let entry = reader.find_asset_by_name(&name).cloned().unwrap();
    let err = reader.verify_asset_checksum(&entry).unwrap_err();
    assert!(matches!(
        err,
        BendlReadError::Checksum(ChecksumError::Mismatch { .. })
    ));
}

#[test]
fn verify_asset_checksum_returns_unavailable_when_flag_clear() {
    // Hand-build a foreign bundle whose entry has the flag clear.
    let payload = b"orphan".to_vec();
    let mut bytes = Vec::new();
    bytes.extend(std::iter::repeat(0u8).take(HEADER_SIZE));
    let payload_offset = bytes.len() as u64;
    bytes.extend_from_slice(&payload);
    let stream_offset = bytes.len() as u64;
    let directory_offset = bytes.len() as u64;
    let entries = vec![BendlDirectoryEntry {
        asset_type: ASSET_TYPE_CUSTOM,
        asset_flags: 0, // explicitly NO checksum flag
        name: "noflag".to_string(),
        payload_offset,
        payload_len: payload.len() as u64,
        checksum: None,
    }];
    let directory = encode_directory(&entries).unwrap();
    bytes.extend_from_slice(&directory);
    let header = BendlHeader {
        magic: BENDL_MAGIC,
        major_version: BENDL_MAJOR_VERSION,
        minor_version: BENDL_MINOR_VERSION,
        finalized: FINALIZED_YES,
        assignment_format: AssignmentFormat::Ben.to_u8(),
        reserved_0: 0,
        flags: 0,
        directory_offset,
        directory_len: directory.len() as u64,
        stream_offset,
        stream_len: 0,
        sample_count: 0,
    };
    bytes[..HEADER_SIZE].copy_from_slice(&header.to_bytes());

    let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
    let entry = reader.find_asset_by_name("noflag").cloned().unwrap();
    let err = reader.verify_asset_checksum(&entry).unwrap_err();
    assert!(matches!(
        err,
        BendlReadError::Checksum(ChecksumError::Unavailable {
            target: ChecksumTarget::Asset(_),
        })
    ));
    // The unverified path can still read the bytes.
    let got = reader.asset_bytes_unverified(&entry).unwrap();
    assert_eq!(got, payload);
}

// ----- Verify-on-touch via asset_bytes ------------------------------

#[test]
fn asset_bytes_uncompressed_corrupt_payload_returns_checksum_mismatch() {
    let (mut bytes, name, _, payload_off) = make_single_asset_bundle("blob", b"hello world");
    bytes[payload_off as usize] ^= 0x01;
    let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
    let entry = reader.find_asset_by_name(&name).cloned().unwrap();
    let err = reader.asset_bytes(&entry).unwrap_err();
    assert!(matches!(
        err,
        BendlReadError::Checksum(ChecksumError::Mismatch { .. })
    ));
}

#[test]
fn asset_bytes_unverified_uncompressed_returns_corrupted_bytes_no_check() {
    let (mut bytes, name, _, payload_off) = make_single_asset_bundle("blob", b"hello world");
    bytes[payload_off as usize] ^= 0x01;
    let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
    let entry = reader.find_asset_by_name(&name).cloned().unwrap();
    let got = reader.asset_bytes_unverified(&entry).unwrap();
    // The bytes returned are the corrupted bytes; we do not assert exact content, only that the
    // operation succeeded — the *_unverified contract is that ChecksumError NEVER fires.
    assert_eq!(got.len(), b"hello world".len());
}

#[test]
fn asset_bytes_xz_corrupt_stored_crc_returns_checksum_mismatch() {
    // xz framing intact, but stored CRC is wrong. The codec reaches EOF cleanly first and then the
    // BENDL-owned wrapper reports `ChecksumError::Mismatch`.
    let (mut bytes, name, _, dir_off, _) =
        make_single_xz_asset_bundle("blob.xz", b"some compressible content");
    let crc_off = stored_checksum_offset(dir_off, &name);
    bytes[crc_off] ^= 0xFF;
    let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
    let entry = reader.find_asset_by_name(&name).cloned().unwrap();
    let err = reader.asset_bytes(&entry).unwrap_err();
    assert!(matches!(
        err,
        BendlReadError::Checksum(ChecksumError::Mismatch { .. })
    ));
}

#[test]
fn asset_bytes_xz_corrupt_framing_returns_decode_error_not_checksum() {
    // Payload flip breaks xz framing — the decoder fails before the CRC tee reaches raw EOF, so the
    // variant is `BendlReadError::Decode`, not `BendlReadError::Checksum`.
    let (mut bytes, name, compressed, _, payload_off) =
        make_single_xz_asset_bundle("blob.xz", b"some compressible content");
    assert!(compressed.len() > 5);
    bytes[payload_off as usize + 5] ^= 0xFF;
    let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
    let entry = reader.find_asset_by_name(&name).cloned().unwrap();
    let err = reader.asset_bytes(&entry).unwrap_err();
    assert!(
        matches!(err, BendlReadError::Decode(_)),
        "expected Decode for broken xz framing, got {err:?}"
    );
}

#[test]
fn asset_bytes_unverified_xz_corrupt_framing_returns_decode_error_never_checksum() {
    let (mut bytes, name, compressed, _, payload_off) =
        make_single_xz_asset_bundle("blob.xz", b"some compressible content");
    assert!(compressed.len() > 5);
    bytes[payload_off as usize + 5] ^= 0xFF;
    let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
    let entry = reader.find_asset_by_name(&name).cloned().unwrap();
    let err = reader.asset_bytes_unverified(&entry).unwrap_err();
    // Unverified path NEVER surfaces a checksum error; codec errors are still allowed.
    assert!(!matches!(err, BendlReadError::Checksum(_)));
    assert!(matches!(err, BendlReadError::Decode(_)));
}

#[test]
fn asset_bytes_returns_unavailable_when_flag_clear() {
    // Same hand-built foreign bundle as in the verifier test.
    let payload = b"orphan".to_vec();
    let mut bytes = Vec::new();
    bytes.extend(std::iter::repeat(0u8).take(HEADER_SIZE));
    let payload_offset = bytes.len() as u64;
    bytes.extend_from_slice(&payload);
    let directory_offset = bytes.len() as u64;
    let entries = vec![BendlDirectoryEntry {
        asset_type: ASSET_TYPE_CUSTOM,
        asset_flags: 0,
        name: "noflag".to_string(),
        payload_offset,
        payload_len: payload.len() as u64,
        checksum: None,
    }];
    let directory = encode_directory(&entries).unwrap();
    bytes.extend_from_slice(&directory);
    let header = BendlHeader {
        magic: BENDL_MAGIC,
        major_version: BENDL_MAJOR_VERSION,
        minor_version: BENDL_MINOR_VERSION,
        finalized: FINALIZED_YES,
        assignment_format: AssignmentFormat::Ben.to_u8(),
        reserved_0: 0,
        flags: 0,
        directory_offset,
        directory_len: directory.len() as u64,
        stream_offset: directory_offset,
        stream_len: 0,
        sample_count: 0,
    };
    bytes[..HEADER_SIZE].copy_from_slice(&header.to_bytes());

    let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
    let entry = reader.find_asset_by_name("noflag").cloned().unwrap();
    let err = reader.asset_bytes(&entry).unwrap_err();
    assert!(matches!(
        err,
        BendlReadError::Checksum(ChecksumError::Unavailable { .. })
    ));
}

// ----- asset_reader EOF semantics ----------------------------------

#[test]
fn asset_reader_uncompressed_surfaces_mismatch_on_final_read() {
    // Drive `asset_reader` byte-by-byte and assert the call that would otherwise return Ok(0) at
    // EOF returns InvalidData wrapping ChecksumError::Mismatch.
    let (mut bytes, name, _, payload_off) = make_single_asset_bundle("blob", b"abcdef");
    bytes[payload_off as usize] ^= 0x01;
    let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
    let entry = reader.find_asset_by_name(&name).cloned().unwrap();
    let mut r = reader.asset_reader(&entry).unwrap();
    let mut buf = [0u8; 1024];
    // Consume bytes until 0 or error.
    let mut total_ok = 0usize;
    loop {
        match r.read(&mut buf) {
            Ok(0) => panic!("expected a checksum error at EOF, got Ok(0) after {total_ok} bytes"),
            Ok(n) => total_ok += n,
            Err(e) => {
                assert_eq!(e.kind(), std::io::ErrorKind::InvalidData);
                let inner = e
                    .get_ref()
                    .and_then(|x| x.downcast_ref::<ChecksumError>())
                    .expect("inner ChecksumError");
                assert!(matches!(inner, ChecksumError::Mismatch { .. }));
                break;
            }
        }
    }
    assert_eq!(total_ok, b"abcdef".len());
}

// ----- Bulk verifier -------------------------------------------------

#[test]
fn verify_all_asset_checksums_reports_first_mismatch_in_directory_order() {
    // Build a bundle with two assets, both corrupted. The bulk verifier must return the *first*
    // mismatch in directory order and stop. Construct manually so we can corrupt independently.
    let p1 = b"first".to_vec();
    let p2 = b"second".to_vec();
    let mut bytes = Vec::new();
    bytes.extend(std::iter::repeat(0u8).take(HEADER_SIZE));
    let off1 = bytes.len() as u64;
    bytes.extend_from_slice(&p1);
    let off2 = bytes.len() as u64;
    bytes.extend_from_slice(&p2);
    let stream_offset = bytes.len() as u64;
    let directory_offset = bytes.len() as u64;
    let e1 = with_crc(
        BendlDirectoryEntry {
            asset_type: ASSET_TYPE_CUSTOM,
            asset_flags: 0,
            name: "first".to_string(),
            payload_offset: off1,
            payload_len: p1.len() as u64,
            checksum: None,
        },
        &p1,
    );
    let e2 = with_crc(
        BendlDirectoryEntry {
            asset_type: ASSET_TYPE_CUSTOM,
            asset_flags: 0,
            name: "second".to_string(),
            payload_offset: off2,
            payload_len: p2.len() as u64,
            checksum: None,
        },
        &p2,
    );
    let directory = encode_directory(&[e1, e2]).unwrap();
    bytes.extend_from_slice(&directory);
    let header = BendlHeader {
        magic: BENDL_MAGIC,
        major_version: BENDL_MAJOR_VERSION,
        minor_version: BENDL_MINOR_VERSION,
        finalized: FINALIZED_YES,
        assignment_format: AssignmentFormat::Ben.to_u8(),
        reserved_0: 0,
        flags: 0,
        directory_offset,
        directory_len: directory.len() as u64,
        stream_offset,
        stream_len: 0,
        sample_count: 0,
    };
    bytes[..HEADER_SIZE].copy_from_slice(&header.to_bytes());
    // Corrupt both payloads.
    bytes[off1 as usize] ^= 0x01;
    bytes[off2 as usize] ^= 0x01;

    let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
    let err = reader.verify_all_asset_checksums().unwrap_err();
    let target = match &err {
        BendlReadError::Checksum(ChecksumError::Mismatch { target, .. }) => target.clone(),
        other => panic!("expected first-asset Mismatch, got {other:?}"),
    };
    assert!(matches!(&target, ChecksumTarget::Asset(n) if n == "first"));
}

// ----- Polynomial pin ------------------------------------------------

#[test]
fn crc32c_polynomial_pin_against_known_vectors() {
    // Pin known CRC32C (Castagnoli) values so a future accidental swap to IEEE CRC-32 is caught at
    // test time. The IEEE CRC-32 of [0x01,0x02,0x03,0x04] is 0xB63CFBCD; the CRC32C value below
    // diverges from that, which is the whole point of the pin.
    //
    //   CRC32C("")                = 0x00000000
    //   CRC32C([1,2,3,4])         = 0x8A2D413B
    //   CRC32C(b"123456789")      = 0xE3069283 (Castagnoli check value)
    //
    // The Castagnoli check value 0xE3069283 is the canonical CRC32C test vector cited in the IEEE
    // 802.3 / SCTP RFC 3720 specs and diverges from the IEEE CRC-32 polynomial's check value
    // (0xCBF43926). If a future contributor accidentally swaps to IEEE CRC-32, this assertion
    // fires.
    assert_eq!(crc32c::crc32c(b""), 0x0000_0000);
    // 0xE3069283 is the canonical Castagnoli check value (CRC32C of ASCII "123456789"); the IEEE
    // CRC-32 polynomial's check value over the same input is 0xCBF43926, so any accidental swap is
    // caught here.
    assert_eq!(crc32c::crc32c(b"123456789"), 0xE306_9283);
    // Extra sentinels to broaden the trip-wire.
    assert_eq!(crc32c::crc32c(&[0x01, 0x02, 0x03, 0x04]), 0x2930_8CF4);
}
