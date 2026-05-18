use std::io;

use crate::io::bundle::format::*;

#[test]
fn magic_is_eight_bytes_and_matches_spec() {
    assert_eq!(BENDL_MAGIC.len(), 8);
    assert_eq!(&BENDL_MAGIC[..5], b"BENDL");
}

#[test]
fn standardized_name_lookup() {
    assert_eq!(
        standardized_name_for(ASSET_TYPE_METADATA),
        Some("metadata.json")
    );
    assert_eq!(standardized_name_for(ASSET_TYPE_GRAPH), Some("graph.json"));
    assert_eq!(
        standardized_name_for(ASSET_TYPE_NODE_PERMUTATION_MAP),
        Some("node_permutation_map.json")
    );
    assert_eq!(standardized_name_for(ASSET_TYPE_CUSTOM), None);
    assert_eq!(standardized_name_for(9999), None);
}

#[test]
fn default_compression_policy() {
    assert!(default_compresses_by_type(ASSET_TYPE_GRAPH));
    assert!(!default_compresses_by_type(ASSET_TYPE_METADATA));
    assert!(!default_compresses_by_type(ASSET_TYPE_NODE_PERMUTATION_MAP));
    assert!(!default_compresses_by_type(ASSET_TYPE_CUSTOM));
}

#[test]
fn assignment_format_roundtrip() {
    for fmt in [AssignmentFormat::Ben, AssignmentFormat::Xben] {
        assert_eq!(AssignmentFormat::from_u8(fmt.to_u8()), Some(fmt));
    }
    assert_eq!(AssignmentFormat::from_u8(0), None);
    assert_eq!(AssignmentFormat::from_u8(255), None);
}

#[test]
fn header_is_exactly_64_bytes() {
    let header = BendlHeader::provisional(AssignmentFormat::Ben, 64);
    assert_eq!(header.to_bytes().len(), HEADER_SIZE);
    assert_eq!(HEADER_SIZE, 64);
}

#[test]
fn header_round_trip_provisional() {
    let header = BendlHeader::provisional(AssignmentFormat::Xben, 64);
    let decoded = BendlHeader::from_bytes(&header.to_bytes()).unwrap();
    assert_eq!(header, decoded);
    assert!(!decoded.is_finalized());
    assert_eq!(
        decoded.assignment_format_typed(),
        Some(AssignmentFormat::Xben)
    );
    assert_eq!(decoded.sample_count, -1);
    assert_eq!(decoded.stream_len, 0);
    assert_eq!(decoded.directory_offset, 0);
}

#[test]
fn header_round_trip_finalized() {
    let header = BendlHeader {
        magic: BENDL_MAGIC,
        major_version: BENDL_MAJOR_VERSION,
        minor_version: BENDL_MINOR_VERSION,
        finalized: FINALIZED_YES,
        assignment_format: ASSIGNMENT_FORMAT_BEN,
        reserved_0: 0,
        flags: 0x0000_0000_0000_000F,
        directory_offset: 1_000_000,
        directory_len: 256,
        stream_offset: 64,
        stream_len: 999_936,
        sample_count: 4242,
    };
    let bytes = header.to_bytes();
    let decoded = BendlHeader::from_bytes(&bytes).unwrap();
    assert_eq!(decoded, header);
    assert!(decoded.is_finalized());
}

#[test]
fn header_rejects_invalid_magic() {
    let mut header = BendlHeader::provisional(AssignmentFormat::Ben, 64);
    header.magic = *b"NOTABEND";
    let err = BendlHeader::from_bytes(&header.to_bytes()).unwrap_err();
    assert!(matches!(err, BendlFormatError::InvalidMagic(_)));
}

#[test]
fn header_rejects_unsupported_major_version() {
    let mut bytes = BendlHeader::provisional(AssignmentFormat::Ben, 64).to_bytes();
    bytes[8..10].copy_from_slice(&999u16.to_le_bytes());
    let err = BendlHeader::from_bytes(&bytes).unwrap_err();
    assert!(matches!(
        err,
        BendlFormatError::UnsupportedMajorVersion { found: 999, .. }
    ));
}

#[test]
fn directory_entry_round_trip_no_checksum() {
    let entry = BendlDirectoryEntry {
        asset_type: ASSET_TYPE_GRAPH,
        asset_flags: ASSET_FLAG_JSON | ASSET_FLAG_XZ,
        name: STANDARDIZED_NAME_GRAPH.to_string(),
        payload_offset: 128,
        payload_len: 4096,
        checksum: None,
    };
    let bytes = entry.to_bytes().unwrap();
    assert_eq!(bytes.len(), entry.encoded_len());
    let mut cursor = &bytes[..];
    let decoded = BendlDirectoryEntry::read_from(&mut cursor).unwrap();
    assert_eq!(decoded, entry);
}

#[test]
fn directory_entry_round_trip_with_checksum() {
    // ASSET_FLAG_CHECKSUM ⇒ exactly four bytes of CRC32C.
    let entry = BendlDirectoryEntry {
        asset_type: ASSET_TYPE_CUSTOM,
        asset_flags: ASSET_FLAG_CHECKSUM,
        name: "custom_blob".to_string(),
        payload_offset: 2048,
        payload_len: 512,
        checksum: Some(vec![0xDE, 0xAD, 0xBE, 0xEF]),
    };
    let bytes = entry.to_bytes().unwrap();
    let mut cursor = &bytes[..];
    let decoded = BendlDirectoryEntry::read_from(&mut cursor).unwrap();
    assert_eq!(decoded, entry);
    assert_eq!(decoded.checksum.as_deref(), Some(&[0xDE, 0xAD, 0xBE, 0xEF][..]));
    assert_eq!(decoded.checksum_u32(), Some(0xEFBEADDE));
}

#[test]
fn directory_entry_rejects_flag_set_with_wrong_checksum_len() {
    // Construct entry bytes by hand: flag bit set but checksum_len == 6.
    let mut entry = BendlDirectoryEntry {
        asset_type: ASSET_TYPE_CUSTOM,
        asset_flags: ASSET_FLAG_CHECKSUM,
        name: "x".to_string(),
        payload_offset: 0,
        payload_len: 0,
        checksum: Some(vec![0xDE, 0xAD, 0xBE, 0xEF]),
    };
    let mut bytes = entry.to_bytes().unwrap();
    // Patch checksum_len at bytes 24..28 to claim 6 (also append two
    // bytes so we don't crash on short read in the negative path).
    bytes[24..28].copy_from_slice(&6u32.to_le_bytes());
    bytes.extend_from_slice(&[0x00, 0x00]); // pad to declared len
    entry.checksum = Some(vec![0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x00]);
    let mut cursor = &bytes[..];
    let err = BendlDirectoryEntry::read_from(&mut cursor).unwrap_err();
    assert!(matches!(
        err,
        BendlFormatError::InconsistentChecksumMetadata {
            flag_set: true,
            checksum_len: 6,
        }
    ));
}

#[test]
fn directory_entry_rejects_flag_clear_with_nonzero_checksum_len() {
    // Construct entry bytes with flag clear but checksum_len == 4.
    let mut entry = BendlDirectoryEntry {
        asset_type: ASSET_TYPE_CUSTOM,
        asset_flags: 0,
        name: "x".to_string(),
        payload_offset: 0,
        payload_len: 0,
        checksum: None,
    };
    let mut bytes = entry.to_bytes().unwrap();
    // The encoded bytes have checksum_len == 0 and no trailing checksum
    // bytes; patch checksum_len to 4 and append four bytes.
    bytes[24..28].copy_from_slice(&4u32.to_le_bytes());
    bytes.extend_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]);
    entry.checksum = Some(vec![0xAA, 0xBB, 0xCC, 0xDD]);
    let mut cursor = &bytes[..];
    let err = BendlDirectoryEntry::read_from(&mut cursor).unwrap_err();
    assert!(matches!(
        err,
        BendlFormatError::InconsistentChecksumMetadata {
            flag_set: false,
            checksum_len: 4,
        }
    ));
}

#[test]
fn directory_table_round_trip() {
    let entries = vec![
        BendlDirectoryEntry {
            asset_type: ASSET_TYPE_GRAPH,
            asset_flags: ASSET_FLAG_JSON | ASSET_FLAG_XZ,
            name: STANDARDIZED_NAME_GRAPH.to_string(),
            payload_offset: 64,
            payload_len: 2048,
            checksum: None,
        },
        BendlDirectoryEntry {
            asset_type: ASSET_TYPE_METADATA,
            asset_flags: ASSET_FLAG_JSON,
            name: STANDARDIZED_NAME_METADATA.to_string(),
            payload_offset: 2112,
            payload_len: 128,
            checksum: None,
        },
        BendlDirectoryEntry {
            asset_type: ASSET_TYPE_CUSTOM,
            asset_flags: 0,
            name: "provenance.bin".to_string(),
            payload_offset: 2240,
            payload_len: 32,
            checksum: None,
        },
    ];

    let encoded = encode_directory(&entries).unwrap();
    let mut cursor = &encoded[..];
    let decoded = read_directory(&mut cursor).unwrap();
    assert_eq!(decoded, entries);
}

#[test]
fn empty_directory_table_round_trip() {
    let encoded = encode_directory(&[]).unwrap();
    assert_eq!(encoded, vec![0, 0, 0, 0]);
    let mut cursor = &encoded[..];
    let decoded = read_directory(&mut cursor).unwrap();
    assert!(decoded.is_empty());
}

#[test]
fn header_and_directory_entry_header_sizes_are_stable() {
    // These sizes are baked into the on-disk format; regressing them
    // would silently break existing bundles.
    assert_eq!(HEADER_SIZE, 64);
    assert_eq!(DIRECTORY_ENTRY_HEADER_SIZE, 28);
}

#[test]
fn directory_entry_name_too_long() {
    let entry = BendlDirectoryEntry {
        asset_type: ASSET_TYPE_CUSTOM,
        asset_flags: 0,
        name: "x".repeat(u16::MAX as usize + 1),
        payload_offset: 0,
        payload_len: 0,
        checksum: None,
    };
    let err = entry.to_bytes().unwrap_err();
    assert!(matches!(err, BendlFormatError::NameTooLong { .. }));
    assert!(err.to_string().contains("exceeds"));
}

#[test]
fn directory_entry_name_not_utf8() {
    let mut bytes = BendlDirectoryEntry {
        asset_type: ASSET_TYPE_CUSTOM,
        asset_flags: 0,
        name: "ok".to_string(),
        payload_offset: 0,
        payload_len: 0,
        checksum: None,
    }
    .to_bytes()
    .unwrap();

    // Patch the name bytes to invalid UTF-8 (0xFF 0xFE)
    let name_offset = DIRECTORY_ENTRY_HEADER_SIZE;
    bytes[name_offset] = 0xFF;
    bytes[name_offset + 1] = 0xFE;

    let mut cursor = &bytes[..];
    let err = BendlDirectoryEntry::read_from(&mut cursor).unwrap_err();
    assert!(matches!(err, BendlFormatError::NameNotUtf8));
    assert!(err.to_string().contains("UTF-8"));
}

#[test]
fn header_read_from_truncated() {
    let short = [0u8; 10];
    let err = BendlHeader::read_from(&mut &short[..]).unwrap_err();
    assert!(matches!(err, BendlFormatError::Io(_)));
}

#[test]
fn bendl_format_error_io_passthrough() {
    let inner = io::Error::new(io::ErrorKind::BrokenPipe, "pipe broke");
    let fmt_err = BendlFormatError::Io(inner);
    let io_err: io::Error = fmt_err.into();
    assert_eq!(io_err.kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(io_err.to_string(), "pipe broke");
}

#[test]
fn bendl_format_error_non_io_becomes_invalid_data() {
    let fmt_err = BendlFormatError::MalformedDirectory("bad dir".to_string());
    let io_err: io::Error = fmt_err.into();
    assert_eq!(io_err.kind(), io::ErrorKind::InvalidData);
    assert!(io_err.to_string().contains("bad dir"));
}

#[test]
fn trailing_directory_bytes_error_display() {
    let err = BendlFormatError::TrailingDirectoryBytes { remaining: 42 };
    assert!(err.to_string().contains("42"));
    assert!(err.to_string().contains("trailing"));
}
