use std::io::{Cursor, Read, Seek, Write};

use xz2::write::XzEncoder;

use crate::io::bundle::error::{BendlReadError, ChecksumError, ChecksumTarget};
use crate::io::bundle::format::{
    encode_directory, AssignmentFormat, BendlDirectoryEntry, BendlFormatError, BendlHeader,
    ASSET_FLAG_CHECKSUM, ASSET_FLAG_XZ, ASSET_TYPE_CUSTOM, ASSET_TYPE_GRAPH, ASSET_TYPE_METADATA,
    BENDL_MAGIC, BENDL_MAJOR_VERSION, BENDL_MINOR_VERSION, DEFAULT_XZ_PRESET, FINALIZED_NO,
    FINALIZED_YES, HEADER_FLAG_STREAM_CHECKSUM, HEADER_SIZE,
};
use crate::io::bundle::reader::BendlReader;
use crate::io::bundle::writer::{AddAssetOptions, BendlAppender, BendlWriteError, BendlWriter};
use crate::io::reader::BenWireFormat;
use crate::io::writer::BenStreamWriter;
use crate::BenVariant;

fn make_buffer() -> Cursor<Vec<u8>> {
    Cursor::new(Vec::new())
}

/// Test helper: replicate the deleted `BendlWriter::write_stream_bytes` using the owned-session
/// chain. Used purely to keep test bodies short.
fn write_stream_bytes_via_session(
    writer: BendlWriter<Cursor<Vec<u8>>>,
    bytes: &[u8],
    sample_count: i64,
) -> BendlWriter<Cursor<Vec<u8>>> {
    let mut session = writer.into_stream_session().unwrap();
    session.write_all(bytes).unwrap();
    session.finish_into_writer(sample_count)
}

#[test]
fn minimal_bundle_round_trip_through_reader() {
    let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    writer
        .add_json_asset(ASSET_TYPE_METADATA, "metadata.json", br#"{"note":"hello"}"#)
        .unwrap();
    let stream_bytes = b"STANDARD BEN FILE\x00\x01fake".to_vec();
    let writer = write_stream_bytes_via_session(writer, &stream_bytes, 7);
    let buf = writer.finish().unwrap().into_inner();

    let mut reader = BendlReader::open(Cursor::new(buf)).unwrap();
    assert!(reader.is_finalized());
    assert_eq!(reader.sample_count(), Some(7));
    assert_eq!(reader.assignment_format(), Some(AssignmentFormat::Ben));
    assert_eq!(reader.assets().len(), 1);

    let entry = reader
        .find_asset_by_type(ASSET_TYPE_METADATA)
        .cloned()
        .expect("metadata entry present");
    assert_eq!(entry.name, "metadata.json");
    assert_eq!(entry.asset_flags & ASSET_FLAG_XZ, 0);
    let meta_bytes = reader.asset_bytes(&entry).unwrap();
    assert_eq!(meta_bytes, br#"{"note":"hello"}"#);

    let mut stream_buf = Vec::new();
    reader
        .assignment_stream_reader()
        .unwrap()
        .read_to_end(&mut stream_buf)
        .unwrap();
    assert_eq!(stream_buf, stream_bytes);
}

#[test]
fn graph_asset_is_compressed_by_default() {
    let graph = br#"{"nodes":[0,1,2,3,4,5,6,7,8,9],"edges":[[0,1],[1,2],[2,3],[3,4]]}"#;
    let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    writer
        .add_json_asset(ASSET_TYPE_GRAPH, "graph.json", graph)
        .unwrap();
    let writer = write_stream_bytes_via_session(writer, b"STANDARD BEN FILE\x00fake", 1);
    let buf = writer.finish().unwrap().into_inner();

    let mut reader = BendlReader::open(Cursor::new(buf)).unwrap();
    let entry = reader
        .find_asset_by_type(ASSET_TYPE_GRAPH)
        .cloned()
        .expect("graph entry present");
    assert_ne!(entry.asset_flags & ASSET_FLAG_XZ, 0);
    // Compressed size should differ from the raw size for a non-trivial JSON payload. For very
    // short payloads xz actually inflates the bytes, so this just checks the size is non-zero and
    // different.
    assert_ne!(entry.payload_len, graph.len() as u64);

    // Decoded bytes round-trip.
    let decoded = reader.asset_bytes(&entry).unwrap();
    assert_eq!(decoded, graph);
}

#[test]
fn graph_asset_can_be_forced_raw() {
    let graph = br#"{"nodes":[0,1,2]}"#;
    let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    writer
        .add_asset(
            ASSET_TYPE_GRAPH,
            "graph.json",
            graph,
            AddAssetOptions::defaults().json().raw(),
        )
        .unwrap();
    let writer = write_stream_bytes_via_session(writer, b"STANDARD BEN FILE\x00fake", 1);
    let buf = writer.finish().unwrap().into_inner();

    let reader = BendlReader::open(Cursor::new(buf)).unwrap();
    let entry = reader
        .find_asset_by_type(ASSET_TYPE_GRAPH)
        .expect("graph entry present");
    assert_eq!(entry.asset_flags & ASSET_FLAG_XZ, 0);
    assert_eq!(entry.payload_len, graph.len() as u64);
}

#[test]
fn writer_rejects_second_graph() {
    let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    writer
        .add_json_asset(ASSET_TYPE_GRAPH, "graph.json", b"{}")
        .unwrap();
    let err = writer
        .add_json_asset(ASSET_TYPE_GRAPH, "graph.json", b"{}")
        .unwrap_err();
    assert!(matches!(err, BendlWriteError::DuplicateSingletonType(t) if t == ASSET_TYPE_GRAPH));
}

#[test]
fn writer_rejects_wrong_standardized_name_for_singleton() {
    let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    let err = writer
        .add_json_asset(ASSET_TYPE_GRAPH, "graph_but_wrong_name.json", b"{}")
        .unwrap_err();
    assert!(matches!(
        err,
        BendlWriteError::WrongCanonicalName {
            asset_type: ASSET_TYPE_GRAPH,
            ..
        }
    ));
}

#[test]
fn writer_rejects_duplicate_custom_name() {
    let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    writer
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "blob",
            b"first",
            AddAssetOptions::defaults(),
        )
        .unwrap();
    let err = writer
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "blob",
            b"second",
            AddAssetOptions::defaults(),
        )
        .unwrap_err();
    assert!(matches!(err, BendlWriteError::DuplicateName(ref n) if n == "blob"));
}

#[test]
fn writer_rejects_asset_added_after_stream_begins() {
    // After a session has been finished, the writer is in `StreamWritten` and `add_*_asset` rejects
    // further additions with `AssetsAfterStream`.
    let writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    let mut writer = write_stream_bytes_via_session(writer, b"STANDARD BEN FILE\x00fake", 1);
    let err = writer
        .add_json_asset(ASSET_TYPE_METADATA, "metadata.json", b"{}")
        .unwrap_err();
    assert!(matches!(err, BendlWriteError::AssetsAfterStream));
}

#[test]
fn asset_only_bundle_finalizes_with_empty_stream() {
    let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    writer
        .add_json_asset(ASSET_TYPE_METADATA, "metadata.json", b"{}")
        .unwrap();
    let buf = writer.finish().unwrap().into_inner();

    let reader = BendlReader::open(Cursor::new(buf)).unwrap();
    assert!(reader.is_finalized());
    assert_eq!(reader.sample_count(), Some(0));
    assert_eq!(reader.header().stream_len, 0);
}

#[test]
fn finalized_directory_lives_at_eof() {
    let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    writer
        .add_json_asset(ASSET_TYPE_METADATA, "metadata.json", b"{}")
        .unwrap();
    let writer = write_stream_bytes_via_session(writer, b"STANDARD BEN FILE\x00fake", 1);
    let buf = writer.finish().unwrap().into_inner();

    let reader = BendlReader::open(Cursor::new(buf.clone())).unwrap();
    let header = reader.header();
    let file_len = buf.len() as u64;
    assert_eq!(header.directory_offset + header.directory_len, file_len);
    // Stream ends where directory begins.
    assert_eq!(
        header.stream_offset + header.stream_len,
        header.directory_offset
    );
}

// =====================================================================
// Append-path tests
// =====================================================================

/// Build a finalized bundle with a single `metadata.json` asset and a short fake stream, then
/// return both the bytes and the byte range (offset, len) occupied by the stream region.
fn build_base_bundle() -> (Vec<u8>, (u64, u64)) {
    let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    writer
        .add_json_asset(ASSET_TYPE_METADATA, "metadata.json", b"{\"version\":1}")
        .unwrap();
    let stream = b"STANDARD BEN FILE\x00\x01\x02\x03\x04\x05stream bytes";
    let writer = write_stream_bytes_via_session(writer, stream, 3);
    let buf = writer.finish().unwrap().into_inner();

    let reader = BendlReader::open(Cursor::new(buf.clone())).unwrap();
    let range = (reader.header().stream_offset, reader.header().stream_len);
    (buf, range)
}

#[test]
fn append_adds_new_asset_and_preserves_old_entries() {
    let (bundle, _) = build_base_bundle();

    let mut appender = BendlAppender::open(Cursor::new(bundle)).unwrap();
    appender
        .add_json_asset(ASSET_TYPE_GRAPH, "graph.json", b"{\"nodes\":[]}")
        .unwrap();
    let buf = appender.commit().unwrap().into_inner();

    let reader = BendlReader::open(Cursor::new(buf)).unwrap();
    assert_eq!(reader.assets().len(), 2);
    assert!(reader.find_asset_by_name("metadata.json").is_some());
    assert!(reader.find_asset_by_name("graph.json").is_some());
    // Finalized bundle invariants still hold.
    assert!(reader.is_finalized());
    assert_eq!(reader.sample_count(), Some(3));
}

#[test]
fn append_leaves_stream_bytes_byte_for_byte_unchanged() {
    let (bundle, (stream_offset, stream_len)) = build_base_bundle();
    let original_stream_bytes =
        bundle[stream_offset as usize..(stream_offset + stream_len) as usize].to_vec();

    let mut appender = BendlAppender::open(Cursor::new(bundle)).unwrap();
    appender
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "blob",
            b"appended custom bytes",
            AddAssetOptions::defaults(),
        )
        .unwrap();
    let buf = appender.commit().unwrap().into_inner();

    // Read back the new header to locate the stream region, then confirm the stream bytes are
    // byte-identical to the original.
    let reader = BendlReader::open(Cursor::new(buf.clone())).unwrap();
    let (off, len) = (reader.header().stream_offset, reader.header().stream_len);
    let appended_stream_bytes = buf[off as usize..(off + len) as usize].to_vec();
    assert_eq!(appended_stream_bytes, original_stream_bytes);
    // Stream offset should not have moved either.
    assert_eq!(off, stream_offset);
    assert_eq!(len, stream_len);
}

#[test]
fn append_preserves_existing_entries_payload_offsets() {
    let (bundle, _) = build_base_bundle();

    // Snapshot the metadata entry's payload_offset before append.
    let reader = BendlReader::open(Cursor::new(bundle.clone())).unwrap();
    let old_offset = reader
        .find_asset_by_name("metadata.json")
        .unwrap()
        .payload_offset;
    drop(reader);

    let mut appender = BendlAppender::open(Cursor::new(bundle)).unwrap();
    appender
        .add_json_asset(ASSET_TYPE_GRAPH, "graph.json", b"{\"nodes\":[0,1,2,3,4,5]}")
        .unwrap();
    let buf = appender.commit().unwrap().into_inner();

    let reader = BendlReader::open(Cursor::new(buf)).unwrap();
    let new_offset = reader
        .find_asset_by_name("metadata.json")
        .unwrap()
        .payload_offset;
    assert_eq!(
        old_offset, new_offset,
        "existing asset offset must not move"
    );
}

#[test]
fn append_rejects_duplicate_singleton_without_touching_file() {
    let (bundle, _) = build_base_bundle();
    let bundle_before = bundle.clone();

    let mut appender = BendlAppender::open(Cursor::new(bundle)).unwrap();
    let err = appender
        .add_json_asset(ASSET_TYPE_METADATA, "metadata.json", b"{\"new\":true}")
        .unwrap_err();
    assert!(matches!(err, BendlWriteError::DuplicateSingletonType(_)));

    // Abort and confirm the file is byte-for-byte unchanged.
    let buf = appender.abort().into_inner();
    assert_eq!(buf, bundle_before);
}

#[test]
fn append_rejects_duplicate_custom_name_without_touching_file() {
    // Start from a bundle containing a custom asset named "blob", then try to append another
    // "blob".
    let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    writer
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "blob",
            b"original",
            AddAssetOptions::defaults(),
        )
        .unwrap();
    let writer = write_stream_bytes_via_session(writer, b"STANDARD BEN FILE\x00fake", 1);
    let bundle = writer.finish().unwrap().into_inner();
    let bundle_before = bundle.clone();

    let mut appender = BendlAppender::open(Cursor::new(bundle)).unwrap();
    let err = appender
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "blob",
            b"dup",
            AddAssetOptions::defaults(),
        )
        .unwrap_err();
    assert!(matches!(err, BendlWriteError::DuplicateName(ref n) if n == "blob"));

    let buf = appender.abort().into_inner();
    assert_eq!(buf, bundle_before);
}

#[test]
fn append_rejects_wrong_canonical_name_without_touching_file() {
    let (bundle, _) = build_base_bundle();
    let bundle_before = bundle.clone();

    let mut appender = BendlAppender::open(Cursor::new(bundle)).unwrap();
    let err = appender
        .add_json_asset(ASSET_TYPE_GRAPH, "not_graph.json", b"{}")
        .unwrap_err();
    assert!(matches!(
        err,
        BendlWriteError::WrongCanonicalName {
            asset_type: ASSET_TYPE_GRAPH,
            ..
        }
    ));

    let buf = appender.abort().into_inner();
    assert_eq!(buf, bundle_before);
}

#[test]
fn append_rejects_incomplete_bundle() {
    // Construct a minimal incomplete bundle: just the provisional header and some stream bytes, no
    // directory.
    let header = BendlHeader {
        magic: BENDL_MAGIC,
        major_version: BENDL_MAJOR_VERSION,
        minor_version: BENDL_MINOR_VERSION,
        finalized: FINALIZED_NO,
        assignment_format: AssignmentFormat::Ben.to_u8(),
        alignment_padding: 0,
        flags: 0,
        stream_checksum: 0,
        directory_offset: 0,
        directory_len: 0,
        stream_offset: HEADER_SIZE as u64,
        stream_len: 0,
        sample_count: -1,
    };
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&header.to_bytes());
    bytes.extend_from_slice(b"STANDARD BEN FILE\x00fake");

    match BendlAppender::open(Cursor::new(bytes)) {
        Err(BendlWriteError::BundleIncomplete) => {}
        Err(other) => panic!("expected BundleIncomplete, got {other:?}"),
        Ok(_) => panic!("expected error, got Ok"),
    }
}

#[test]
fn append_rejects_finalized_bundle_with_zero_directory() {
    // Header claims finalized but has directory_offset=0 — hits the second BundleIncomplete check.
    let header = BendlHeader {
        magic: BENDL_MAGIC,
        major_version: BENDL_MAJOR_VERSION,
        minor_version: BENDL_MINOR_VERSION,
        finalized: FINALIZED_YES,
        assignment_format: AssignmentFormat::Ben.to_u8(),
        alignment_padding: 0,
        flags: 0,
        stream_checksum: 0,
        directory_offset: 0,
        directory_len: 0,
        stream_offset: HEADER_SIZE as u64,
        stream_len: 0,
        sample_count: 0,
    };
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&header.to_bytes());

    match BendlAppender::open(Cursor::new(bytes)) {
        Err(BendlWriteError::BundleIncomplete) => {}
        Err(other) => panic!("expected BundleIncomplete, got {other:?}"),
        Ok(_) => panic!("expected error, got Ok"),
    }
}

#[test]
fn append_multiple_assets_in_one_commit() {
    let (bundle, _) = build_base_bundle();
    let mut appender = BendlAppender::open(Cursor::new(bundle)).unwrap();
    appender
        .add_json_asset(ASSET_TYPE_GRAPH, "graph.json", b"{\"n\":[0,1,2]}")
        .unwrap();
    appender
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "blob1",
            b"blob one",
            AddAssetOptions::defaults(),
        )
        .unwrap();
    appender
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "blob2",
            b"blob two",
            AddAssetOptions::defaults(),
        )
        .unwrap();
    let buf = appender.commit().unwrap().into_inner();

    let mut reader = BendlReader::open(Cursor::new(buf)).unwrap();
    assert_eq!(reader.assets().len(), 4);
    // Round-trip the appended graph through the reader to confirm compression happened and decodes
    // cleanly.
    let graph_entry = reader
        .find_asset_by_name("graph.json")
        .cloned()
        .expect("graph entry present");
    assert_ne!(graph_entry.asset_flags & ASSET_FLAG_XZ, 0);
    let graph_bytes = reader.asset_bytes(&graph_entry).unwrap();
    assert_eq!(graph_bytes, b"{\"n\":[0,1,2]}");
}

#[test]
fn append_rejects_conflicting_pending_additions() {
    let (bundle, _) = build_base_bundle();
    let bundle_before = bundle.clone();

    let mut appender = BendlAppender::open(Cursor::new(bundle)).unwrap();
    appender
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "new_blob",
            b"a",
            AddAssetOptions::defaults(),
        )
        .unwrap();
    let err = appender
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "new_blob",
            b"b",
            AddAssetOptions::defaults(),
        )
        .unwrap_err();
    assert!(matches!(err, BendlWriteError::DuplicateName(_)));

    let buf = appender.abort().into_inner();
    assert_eq!(buf, bundle_before);
}

// =====================================================================
// Phase 4: assignment-stream integration tests
// =====================================================================

#[test]
fn bundle_ben_stream_round_trips_through_assignment_reader() {
    use crate::BenVariant;

    let samples: Vec<Vec<u16>> = vec![
        vec![0, 0, 1, 1, 2, 2],
        vec![0, 1, 1, 1, 2, 2],
        vec![0, 1, 1, 1, 2, 2], // repeat
        vec![1, 1, 1, 1, 2, 2],
    ];

    let writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    let mut session = writer.into_stream_session().unwrap();
    {
        let mut ben = BenStreamWriter::for_ben(&mut session, BenVariant::MkvChain).unwrap();
        for s in &samples {
            ben.write_assignment(s.clone()).unwrap();
        }
        ben.finish().unwrap();
    }
    let writer = session.finish_into_writer(samples.len() as i64);
    let buf = writer.finish().unwrap().into_inner();

    let mut reader = BendlReader::open(Cursor::new(buf)).unwrap();
    assert!(reader.is_finalized());
    // Four write_assignment calls → sample_count == 4.
    assert_eq!(reader.sample_count(), Some(samples.len() as i64));
    assert_eq!(reader.assignment_format(), Some(AssignmentFormat::Ben));

    let decoder = reader.open_assignment_reader().unwrap();
    assert_eq!(decoder.wire_format(), BenWireFormat::Ben);
    let decoded: Vec<Vec<u16>> = decoder
        .silent(true)
        .flat_map(|r| {
            let (assign, count) = r.unwrap();
            std::iter::repeat_n(assign, count as usize)
        })
        .collect();
    assert_eq!(decoded, samples);
}

#[test]
fn bundle_xben_stream_round_trips_through_assignment_reader() {
    use crate::BenVariant;

    let samples: Vec<Vec<u16>> = vec![
        vec![0, 1, 2, 3, 4, 5],
        vec![0, 1, 2, 3, 4, 5], // repeat
        vec![1, 1, 2, 3, 4, 5],
        vec![1, 1, 2, 3, 4, 4],
    ];

    let writer = BendlWriter::new(make_buffer(), AssignmentFormat::Xben).unwrap();
    let mut session = writer.into_stream_session().unwrap();
    {
        let encoder = XzEncoder::new(&mut session, DEFAULT_XZ_PRESET);
        let mut xben =
            BenStreamWriter::for_xben_with_encoder(encoder, BenVariant::MkvChain, None).unwrap();
        for s in &samples {
            xben.write_assignment(s.clone()).unwrap();
        }
        xben.finish().unwrap();
    }
    let writer = session.finish_into_writer(samples.len() as i64);
    let buf = writer.finish().unwrap().into_inner();

    let mut reader = BendlReader::open(Cursor::new(buf)).unwrap();
    assert!(reader.is_finalized());
    assert_eq!(reader.sample_count(), Some(samples.len() as i64));
    assert_eq!(reader.assignment_format(), Some(AssignmentFormat::Xben));

    let decoder = reader.open_assignment_reader().unwrap();
    assert_eq!(decoder.wire_format(), BenWireFormat::XBen);
    let decoded: Vec<Vec<u16>> = decoder
        .silent(true)
        .flat_map(|r| {
            let (assign, count) = r.unwrap();
            std::iter::repeat_n(assign, count as usize)
        })
        .collect();
    assert_eq!(decoded, samples);
}

#[test]
fn bundle_ben_stream_alongside_front_loaded_asset() {
    use crate::BenVariant;

    let graph = br#"{"nodes":[0,1,2],"edges":[[0,1],[1,2]]}"#;
    let samples: Vec<Vec<u16>> = vec![vec![0, 1, 1, 2], vec![0, 1, 2, 2]];

    let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    writer
        .add_json_asset(ASSET_TYPE_GRAPH, "graph.json", graph)
        .unwrap();
    let mut session = writer.into_stream_session().unwrap();
    {
        let mut ben = BenStreamWriter::for_ben(&mut session, BenVariant::Standard).unwrap();
        for s in &samples {
            ben.write_assignment(s.clone()).unwrap();
        }
        ben.finish().unwrap();
    }
    let writer = session.finish_into_writer(samples.len() as i64);
    let buf = writer.finish().unwrap().into_inner();

    let mut reader = BendlReader::open(Cursor::new(buf)).unwrap();
    assert_eq!(reader.sample_count(), Some(samples.len() as i64));

    // Front-loaded graph asset survives round trip through xz.
    let entry = reader
        .find_asset_by_type(ASSET_TYPE_GRAPH)
        .cloned()
        .expect("graph asset present");
    assert_ne!(entry.asset_flags & ASSET_FLAG_XZ, 0);
    let decoded_graph = reader.asset_bytes(&entry).unwrap();
    assert_eq!(decoded_graph, graph);

    // Assignment stream is still intact after pulling asset bytes.
    let decoder = reader.open_assignment_reader().unwrap();
    assert_eq!(decoder.wire_format(), BenWireFormat::Ben);
    let decoded: Vec<Vec<u16>> = decoder
        .silent(true)
        .flat_map(|r| {
            let (assign, count) = r.unwrap();
            std::iter::repeat_n(assign, count as usize)
        })
        .collect();
    assert_eq!(decoded, samples);
}

#[test]
fn open_assignment_reader_reports_ben_wire_format() {
    use crate::BenVariant;

    let writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    let mut session = writer.into_stream_session().unwrap();
    {
        let mut ben = BenStreamWriter::for_ben(&mut session, BenVariant::Standard).unwrap();
        ben.write_assignment(vec![0, 1]).unwrap();
        ben.finish().unwrap();
    }
    let writer = session.finish_into_writer(1);
    let buf = writer.finish().unwrap().into_inner();

    let mut reader = BendlReader::open(Cursor::new(buf)).unwrap();
    let decoder = reader.open_assignment_reader().unwrap();
    assert_eq!(decoder.wire_format(), BenWireFormat::Ben);
}

// =====================================================================
// Robustness tests
// =====================================================================

#[test]
fn fully_empty_bundle_finalizes_and_round_trips() {
    // No assets, no stream bytes, no stream phase at all.
    let writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    let buf = writer.finish().unwrap().into_inner();
    let reader = BendlReader::open(Cursor::new(buf)).unwrap();
    assert!(reader.is_finalized());
    assert_eq!(reader.sample_count(), Some(0));
    assert_eq!(reader.header().stream_len, 0);
    assert_eq!(reader.assets().len(), 0);
    // Even with zero assets the directory is present and empty.
    assert_ne!(reader.header().directory_offset, 0);
    // directory_len should equal the 4-byte empty entry-count header.
    assert_eq!(reader.header().directory_len, 4);
}

#[test]
fn into_stream_session_after_stream_written_returns_wrong_state() {
    // Regression fixture for the `into_stream_session` guard: a writer that has already finished
    // one stream phase must reject a second attempt to enter the stream phase. Without this guard,
    // a chained `into_stream_session → finish_into_writer → into_stream_session` would silently
    // overwrite `header.stream_offset` and corrupt the bundle. This is the only runtime fixture for
    // that guard.
    let writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    let writer = write_stream_bytes_via_session(writer, b"STANDARD BEN FILE\x00fake", 1);
    // Writer is now in StreamWritten state; into_stream_session must fail.
    match writer.into_stream_session() {
        Err(BendlWriteError::WrongState {
            expected: "Assets",
            found: "StreamWritten",
        }) => {}
        Err(other) => panic!("expected WrongState, got {other:?}"),
        Ok(_) => panic!("into_stream_session after StreamWritten must fail"),
    }
}

#[test]
fn stress_many_custom_assets_round_trip() {
    let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    // Stays under MAX_DIRECTORY_ENTRIES so the directory is well-formed while still exercising the
    // many-entry seek/round-trip paths.
    let count = 200usize;
    for i in 0..count {
        let name = format!("blob_{i:05}");
        let payload = vec![(i & 0xFF) as u8; (i % 17) + 1];
        writer
            .add_asset(
                ASSET_TYPE_CUSTOM,
                &name,
                &payload,
                AddAssetOptions::defaults(),
            )
            .unwrap();
    }
    let writer = write_stream_bytes_via_session(writer, b"STANDARD BEN FILE\x00fake", 1);
    let buf = writer.finish().unwrap().into_inner();

    let mut reader = BendlReader::open(Cursor::new(buf)).unwrap();
    assert_eq!(reader.assets().len(), count);
    // Spot-check a handful of entries by reading their payload bytes back.
    for i in [0usize, 1, 42, 150, 199] {
        let name = format!("blob_{i:05}");
        let entry = reader.find_asset_by_name(&name).cloned().unwrap();
        let got = reader.asset_bytes(&entry).unwrap();
        assert_eq!(got, vec![(i & 0xFF) as u8; (i % 17) + 1]);
    }
}

#[test]
fn append_empty_commit_is_noop() {
    let (bundle, _) = build_base_bundle();
    let bundle_before = bundle.clone();
    let appender = BendlAppender::open(Cursor::new(bundle)).unwrap();
    // No add_asset calls. Commit should return the file unchanged.
    let buf = appender.commit().unwrap().into_inner();
    assert_eq!(buf, bundle_before);
}

#[test]
fn append_then_reopen_and_append_again() {
    let (bundle, _) = build_base_bundle();

    // First commit: add a graph.
    let mut appender = BendlAppender::open(Cursor::new(bundle)).unwrap();
    appender
        .add_json_asset(ASSET_TYPE_GRAPH, "graph.json", b"{\"n\":[0,1,2]}")
        .unwrap();
    let buf = appender.commit().unwrap().into_inner();

    // Second commit: reopen the same bytes and add a custom blob.
    let mut appender = BendlAppender::open(Cursor::new(buf)).unwrap();
    appender
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "extra.bin",
            b"later",
            AddAssetOptions::defaults(),
        )
        .unwrap();
    let buf = appender.commit().unwrap().into_inner();

    // Final read: all three assets should be present.
    let reader = BendlReader::open(Cursor::new(buf)).unwrap();
    let names: Vec<&str> = reader.assets().iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"metadata.json"));
    assert!(names.contains(&"graph.json"));
    assert!(names.contains(&"extra.bin"));
    // Sample count from the original stream is preserved across both appends.
    assert_eq!(reader.sample_count(), Some(3));
}

#[test]
fn append_does_not_disturb_front_loaded_asset_bytes() {
    // Base bundle has a graph.json asset with known bytes; after append of a custom blob, reading
    // graph.json must still return exactly the same decoded bytes as before.
    let graph = br#"{"nodes":[0,1,2,3,4,5,6,7,8,9,10]}"#;
    let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    writer
        .add_json_asset(ASSET_TYPE_GRAPH, "graph.json", graph)
        .unwrap();
    let writer = write_stream_bytes_via_session(writer, b"STANDARD BEN FILE\x00fake", 1);
    let bundle = writer.finish().unwrap().into_inner();

    let mut reader = BendlReader::open(Cursor::new(bundle.clone())).unwrap();
    let entry = reader
        .find_asset_by_type(ASSET_TYPE_GRAPH)
        .cloned()
        .unwrap();
    let graph_before = reader.asset_bytes(&entry).unwrap();
    drop(reader);

    let mut appender = BendlAppender::open(Cursor::new(bundle)).unwrap();
    appender
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "extra.bin",
            b"0123456789",
            AddAssetOptions::defaults(),
        )
        .unwrap();
    let buf = appender.commit().unwrap().into_inner();

    let mut reader = BendlReader::open(Cursor::new(buf)).unwrap();
    let entry = reader
        .find_asset_by_type(ASSET_TYPE_GRAPH)
        .cloned()
        .unwrap();
    let graph_after = reader.asset_bytes(&entry).unwrap();
    assert_eq!(graph_before, graph_after);
}

#[test]
fn writer_accepts_custom_asset_with_canonical_name_but_non_canonical_type() {
    // A custom asset named "graph.json" is not a singleton because the singleton uniqueness check
    // keys off asset_type, not name. Adding a real GRAPH singleton after it must then fail on
    // DuplicateName.
    let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    writer
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "graph.json",
            b"custom graph-ish bytes",
            AddAssetOptions::defaults(),
        )
        .unwrap();
    let err = writer
        .add_json_asset(ASSET_TYPE_GRAPH, "graph.json", b"{}")
        .unwrap_err();
    assert!(matches!(err, BendlWriteError::DuplicateName(ref n) if n == "graph.json"));
}

#[test]
fn writer_asset_round_trips_with_auto_computed_crc32c() {
    // Every asset gets ASSET_FLAG_CHECKSUM with a 4-byte CRC32C of the on-disk payload bytes
    // (post-compression for xz-flagged assets).
    let payload = b"hello".to_vec();
    let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    writer
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "with_checksum",
            &payload,
            AddAssetOptions::defaults().raw(),
        )
        .unwrap();
    let writer = write_stream_bytes_via_session(writer, b"STANDARD BEN FILE\x00fake", 1);
    let buf = writer.finish().unwrap().into_inner();

    let reader = BendlReader::open(Cursor::new(buf)).unwrap();
    let entry = reader.find_asset_by_name("with_checksum").cloned().unwrap();
    assert_ne!(entry.asset_flags & ASSET_FLAG_CHECKSUM, 0);
    let expected_crc = crc32c::crc32c(&payload);
    assert_eq!(entry.checksum_u32(), Some(expected_crc));
    assert_eq!(
        entry.checksum,
        Some(expected_crc.to_le_bytes().to_vec()),
        "stored checksum is the little-endian CRC32C"
    );
}

#[test]
fn writer_xz_asset_stores_crc_over_compressed_bytes_not_raw() {
    // The CRC contract for xz-flagged assets is "CRC32C over the on-disk bytes" — i.e. the
    // compressed bytes, not the raw input. Pin this directly: re-compress the same input, compute
    // the CRC over the compressed result, and assert it matches the stored value. Asserting that
    // the stored CRC does NOT equal `crc32c(raw_input)` is what catches the "writer accidentally
    // hashed pre-compression bytes" regression.
    let payload = b"the quick brown fox jumps over the lazy dog".to_vec();
    let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    writer
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "xz_asset",
            &payload,
            AddAssetOptions::defaults().compress(),
        )
        .unwrap();
    let writer = write_stream_bytes_via_session(writer, b"STANDARD BEN FILE\x00fake", 1);
    let buf = writer.finish().unwrap().into_inner();

    let reader = BendlReader::open(Cursor::new(buf)).unwrap();
    let entry = reader.find_asset_by_name("xz_asset").cloned().unwrap();
    assert_ne!(
        entry.asset_flags & ASSET_FLAG_XZ,
        0,
        "asset must be xz-flagged"
    );
    assert_ne!(entry.asset_flags & ASSET_FLAG_CHECKSUM, 0);

    let mut encoder = XzEncoder::new(Vec::new(), DEFAULT_XZ_PRESET);
    encoder.write_all(&payload).unwrap();
    let compressed = encoder.finish().unwrap();
    assert_eq!(
        entry.checksum_u32(),
        Some(crc32c::crc32c(&compressed)),
        "stored CRC must be over compressed on-disk bytes"
    );
    assert_ne!(
        entry.checksum_u32(),
        Some(crc32c::crc32c(&payload)),
        "stored CRC must NOT be over the raw pre-compression input"
    );
}

#[test]
fn finished_writer_rejects_further_operations() {
    let writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    let writer = write_stream_bytes_via_session(writer, b"STANDARD BEN FILE\x00fake", 1);
    // `finish` consumes `self`, which is itself the protection — there is no way to call add_asset
    // / into_stream_session afterwards.
    let buf = writer.finish().unwrap().into_inner();
    // The resulting buffer is a valid finalized bundle.
    let reader = BendlReader::open(Cursor::new(buf)).unwrap();
    assert!(reader.is_finalized());
}

#[test]
fn appender_commit_after_abort_is_not_possible_but_abort_leaves_bytes_unchanged() {
    let (bundle, _) = build_base_bundle();
    let before = bundle.clone();
    let mut appender = BendlAppender::open(Cursor::new(bundle)).unwrap();
    appender
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "wont_land",
            b"orphan",
            AddAssetOptions::defaults(),
        )
        .unwrap();
    let buf = appender.abort().into_inner();
    assert_eq!(buf, before, "abort must leave file bytes unchanged");
}

#[test]
fn writer_rejects_add_json_asset_with_wrong_canonical_metadata_name() {
    let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    let err = writer
        .add_json_asset(ASSET_TYPE_METADATA, "meta.json", b"{}")
        .unwrap_err();
    assert!(matches!(
        err,
        BendlWriteError::WrongCanonicalName {
            asset_type: ASSET_TYPE_METADATA,
            ..
        }
    ));
    // After a rejected add, no entries have been recorded — a subsequent valid add proceeds
    // normally.
    writer
        .add_json_asset(ASSET_TYPE_METADATA, "metadata.json", b"{}")
        .unwrap();
    let writer = write_stream_bytes_via_session(writer, b"STANDARD BEN FILE\x00fake", 1);
    let buf = writer.finish().unwrap().into_inner();
    let reader = BendlReader::open(Cursor::new(buf)).unwrap();
    assert_eq!(reader.assets().len(), 1);
}

#[test]
fn writer_rejected_add_leaves_singleton_slot_usable() {
    // A rejected singleton add must not consume the singleton slot — otherwise a future valid add
    // with the correct standardized name would spuriously fail with DuplicateSingletonType.
    let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    // First try with wrong standardized name — rejected.
    let _ = writer
        .add_json_asset(ASSET_TYPE_GRAPH, "not_graph.json", b"{}")
        .unwrap_err();
    // Now retry with correct name; should succeed.
    writer
        .add_json_asset(ASSET_TYPE_GRAPH, "graph.json", b"{}")
        .unwrap();
}

#[test]
fn append_rejects_duplicate_name_across_existing_and_pending() {
    let (bundle, _) = build_base_bundle();
    let mut appender = BendlAppender::open(Cursor::new(bundle)).unwrap();
    // First pending add: "blob".
    appender
        .add_asset(ASSET_TYPE_CUSTOM, "blob", b"1", AddAssetOptions::defaults())
        .unwrap();
    // Second pending add with same name must be rejected.
    let err = appender
        .add_asset(ASSET_TYPE_CUSTOM, "blob", b"2", AddAssetOptions::defaults())
        .unwrap_err();
    assert!(matches!(err, BendlWriteError::DuplicateName(_)));
    // Committing the still-valid first pending add should still work.
    let buf = appender.commit().unwrap().into_inner();
    let reader = BendlReader::open(Cursor::new(buf)).unwrap();
    assert!(reader.find_asset_by_name("blob").is_some());
}

// =====================================================================
// Randomized / stress tests
// =====================================================================

/// Build a bundle from a random set of custom assets (plus an optional metadata asset) and fully
/// round-trip it through the reader. Repeated with a seeded ChaCha PRNG so the sequence is
/// deterministic but covers a wide surface.
#[test]
fn randomized_round_trip_many_custom_assets() {
    use rand::{Rng, SeedableRng};
    use rand_chacha::ChaCha8Rng;

    for seed in 0u64..12 {
        let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0xA110_CADE_F00D);
        let n_assets: usize = rng.random_range(0..=25);
        let include_metadata = rng.random_bool(0.5);

        let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();

        let mut expected: Vec<(String, Vec<u8>, bool)> = Vec::new();
        if include_metadata {
            let payload = format!(r#"{{"seed":{seed}}}"#).into_bytes();
            writer
                .add_json_asset(ASSET_TYPE_METADATA, "metadata.json", &payload)
                .unwrap();
            expected.push(("metadata.json".to_string(), payload, false));
        }

        for i in 0..n_assets {
            let size: usize = rng.random_range(0..=512);
            let payload: Vec<u8> = (0..size).map(|_| rng.random::<u8>()).collect();
            let compress = rng.random_bool(0.4);
            let is_json = rng.random_bool(0.15) && size > 0;
            let payload = if is_json {
                // Override with a synthetic JSON blob so the json flag actually matches the
                // content.
                format!(r#"{{"i":{i},"seed":{seed}}}"#).into_bytes()
            } else {
                payload
            };

            let mut opts = AddAssetOptions::defaults();
            if compress {
                opts = opts.compress();
            } else {
                opts = opts.raw();
            }
            if is_json {
                opts = opts.json();
            }
            let name = format!("seed{seed}-asset{i}.bin");
            writer
                .add_asset(ASSET_TYPE_CUSTOM, &name, &payload, opts)
                .unwrap();
            expected.push((name, payload, is_json));
        }

        // Write a small deterministic stream so the bundle is assignment-complete.
        let sample_count: i64 = rng.random_range(0..=20);
        let fake_stream = b"STANDARD BEN FILE\x00\x01\x02payload".to_vec();
        let writer = write_stream_bytes_via_session(writer, &fake_stream, sample_count);
        let buf = writer.finish().unwrap().into_inner();

        let mut reader = BendlReader::open(Cursor::new(buf)).unwrap();
        assert!(reader.is_finalized(), "seed {seed}: not finalized");
        assert_eq!(reader.sample_count(), Some(sample_count));
        reader
            .validate_directory()
            .unwrap_or_else(|e| panic!("seed {seed}: validation failed: {e:?}"));
        assert_eq!(reader.assets().len(), expected.len(), "seed {seed}");

        for (name, want, _is_json) in &expected {
            let entry = reader
                .find_asset_by_name(name)
                .cloned()
                .unwrap_or_else(|| panic!("seed {seed}: asset {name:?} missing"));
            let got = reader.asset_bytes(&entry).unwrap();
            assert_eq!(&got, want, "seed {seed}: payload mismatch for {name}");
        }

        // Stream must also read back exactly.
        let mut stream_buf = Vec::new();
        reader
            .assignment_stream_reader()
            .unwrap()
            .read_to_end(&mut stream_buf)
            .unwrap();
        assert_eq!(stream_buf, fake_stream, "seed {seed}");
    }
}

#[test]
fn five_successive_appends_preserve_everything() {
    // Start from a finalized bundle with only a metadata asset and a short stream. Then open it
    // five times via BendlAppender and add one asset per round. After every round, the previous
    // assets must still be readable and sample_count must remain authoritative.
    let (mut buf, _) = build_base_bundle();

    // Sanity-check the baseline.
    let baseline_reader = BendlReader::open(Cursor::new(buf.clone())).unwrap();
    let baseline_samples = baseline_reader.sample_count();
    assert!(baseline_samples.is_some());
    drop(baseline_reader);

    let mut accumulated: Vec<(String, Vec<u8>)> =
        vec![("metadata.json".to_string(), br#"{"version":1}"#.to_vec())];

    for round in 0..5 {
        let cursor = Cursor::new(buf);
        let mut appender = BendlAppender::open(cursor).unwrap();
        let name = format!("round-{round}.bin");
        let payload: Vec<u8> = (0u8..=(round as u8 * 7 + 3)).collect();
        appender
            .add_asset(
                ASSET_TYPE_CUSTOM,
                &name,
                &payload,
                AddAssetOptions::defaults(),
            )
            .unwrap();
        let commit = appender.commit().unwrap();
        buf = commit.into_inner();
        accumulated.push((name, payload));

        // Re-open and verify the full set is intact and sample_count still matches the baseline
        // (append must not touch it).
        let mut reader = BendlReader::open(Cursor::new(buf.clone())).unwrap();
        assert!(reader.is_finalized(), "round {round}");
        assert_eq!(
            reader.sample_count(),
            baseline_samples,
            "sample count drifted at round {round}"
        );
        assert_eq!(
            reader.assets().len(),
            accumulated.len(),
            "asset count wrong at round {round}"
        );
        reader.validate_directory().unwrap();

        for (n, want) in &accumulated {
            let entry = reader
                .find_asset_by_name(n)
                .cloned()
                .unwrap_or_else(|| panic!("round {round}: {n:?} missing"));
            let got = reader.asset_bytes(&entry).unwrap();
            assert_eq!(&got, want, "round {round}: payload mismatch for {n}");
        }
    }
}

#[test]
fn randomized_append_sequence_preserves_all_prior_entries() {
    // Independent coverage for append: random number of rounds, random payload sizes. Catches any
    // bookkeeping drift in the appender's append-only replacement-directory path.
    use rand::{Rng, SeedableRng};
    use rand_chacha::ChaCha8Rng;

    let (mut buf, _) = build_base_bundle();
    let mut accumulated: Vec<(String, Vec<u8>)> =
        vec![("metadata.json".to_string(), br#"{"version":1}"#.to_vec())];

    let mut rng = ChaCha8Rng::seed_from_u64(0xDEAD_BEEF_CAFE_F00D);
    let rounds: usize = rng.random_range(3..=8);
    for round in 0..rounds {
        let adds: usize = rng.random_range(1..=4);
        let cursor = Cursor::new(buf);
        let mut appender = BendlAppender::open(cursor).unwrap();
        for k in 0..adds {
            let size: usize = rng.random_range(0..=256);
            let payload: Vec<u8> = (0..size).map(|_| rng.random::<u8>()).collect();
            let name = format!("r{round}-a{k}.bin");
            appender
                .add_asset(
                    ASSET_TYPE_CUSTOM,
                    &name,
                    &payload,
                    AddAssetOptions::defaults(),
                )
                .unwrap();
            accumulated.push((name, payload));
        }
        let commit = appender.commit().unwrap();
        buf = commit.into_inner();

        let mut reader = BendlReader::open(Cursor::new(buf.clone())).unwrap();
        reader.validate_directory().unwrap();
        assert_eq!(reader.assets().len(), accumulated.len());
        for (n, want) in &accumulated {
            let entry = reader.find_asset_by_name(n).cloned().unwrap();
            let got = reader.asset_bytes(&entry).unwrap();
            assert_eq!(&got, want, "append round {round}: {n}");
        }
    }
}

// ── write_json_value and sample_count coverage ──────────────────

#[test]
fn bundle_ben_stream_json_value_and_caller_sample_count() {
    use crate::BenVariant;
    use serde_json::json;

    let writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    let mut session = writer.into_stream_session().unwrap();
    {
        let mut ben = BenStreamWriter::for_ben(&mut session, BenVariant::Standard).unwrap();
        ben.write_json_value(json!({"assignment": [1, 2, 3], "sample": 1}))
            .unwrap();
        ben.write_json_value(json!({"assignment": [4, 5, 6], "sample": 2}))
            .unwrap();
        ben.finish().unwrap();
    }
    let writer = session.finish_into_writer(2);
    let buf = writer.finish().unwrap().into_inner();

    let mut reader = BendlReader::open(Cursor::new(buf)).unwrap();
    assert_eq!(reader.sample_count(), Some(2));
    let decoder = reader.open_assignment_reader().unwrap();
    assert_eq!(decoder.wire_format(), BenWireFormat::Ben);
    let decoded: Vec<Vec<u16>> = decoder.silent(true).map(|r| r.unwrap().0).collect();
    assert_eq!(decoded, vec![vec![1, 2, 3], vec![4, 5, 6]]);
}

#[test]
fn bundle_xben_stream_json_value() {
    use crate::BenVariant;
    use serde_json::json;

    let writer = BendlWriter::new(make_buffer(), AssignmentFormat::Xben).unwrap();
    let mut session = writer.into_stream_session().unwrap();
    {
        let encoder = XzEncoder::new(&mut session, DEFAULT_XZ_PRESET);
        let mut xben =
            BenStreamWriter::for_xben_with_encoder(encoder, BenVariant::Standard, None).unwrap();
        xben.write_json_value(json!({"assignment": [10, 20], "sample": 1}))
            .unwrap();
        xben.write_json_value(json!({"assignment": [30, 40], "sample": 2}))
            .unwrap();
        xben.finish().unwrap();
    }
    let writer = session.finish_into_writer(2);
    let buf = writer.finish().unwrap().into_inner();

    let mut reader = BendlReader::open(Cursor::new(buf)).unwrap();
    assert_eq!(reader.sample_count(), Some(2));
    let decoder = reader.open_assignment_reader().unwrap();
    assert_eq!(decoder.wire_format(), BenWireFormat::XBen);
    let decoded: Vec<Vec<u16>> = decoder.silent(true).map(|r| r.unwrap().0).collect();
    assert_eq!(decoded, vec![vec![10, 20], vec![30, 40]]);
}

// ── BendlStreamSession: flush ────────────────────────────────────

#[test]
fn stream_session_flush_succeeds() {
    let writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    let mut session = writer.into_stream_session().unwrap();
    session.flush().unwrap();
    // Discard the session — it would warn on Drop, but the test runner does not assert on log
    // output, so this is fine for unit tests.
    let _ = session.finish_into_writer(0);
}

// ── BendlAppender: checksum flag ────────────────────────────────

#[test]
fn appender_commit_auto_computes_crc32c_on_pending_assets() {
    let (bundle, _) = build_base_bundle();
    let payload = b"payload".to_vec();
    let mut appender = BendlAppender::open(Cursor::new(bundle)).unwrap();
    appender
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "checksummed",
            &payload,
            AddAssetOptions::defaults().raw(),
        )
        .unwrap();
    let buf = appender.commit().unwrap().into_inner();

    let reader = BendlReader::open(Cursor::new(buf)).unwrap();
    let entry = reader.find_asset_by_name("checksummed").unwrap();
    assert_ne!(entry.asset_flags & ASSET_FLAG_CHECKSUM, 0);
    assert_eq!(entry.checksum_u32(), Some(crc32c::crc32c(&payload)));
}

// ── BendlAppender: trailing directory bytes ──────────────────────

#[test]
fn appender_rejects_bundle_with_trailing_directory_bytes() {
    let (mut bundle, _) = build_base_bundle();
    // Patch the header's directory_len field (bytes 32-39) to claim the directory is 4 bytes longer
    // than it actually is.
    let old_len = u64::from_le_bytes(bundle[32..40].try_into().unwrap());
    let patched = (old_len + 4).to_le_bytes();
    bundle[32..40].copy_from_slice(&patched);

    match BendlAppender::open(Cursor::new(bundle)) {
        Err(BendlWriteError::Format(BendlFormatError::TrailingDirectoryBytes { .. })) => {}
        Err(other) => panic!("expected TrailingDirectoryBytes, got {other:?}"),
        Ok(_) => panic!("expected error, got Ok"),
    }
}

// ── finalize from wrong state ───────────────────────────────────

#[test]
fn finish_after_assignment_stream_produces_finalized_bundle() {
    use crate::BenVariant;

    let writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    let mut session = writer.into_stream_session().unwrap();
    {
        let mut ben = BenStreamWriter::for_ben(&mut session, BenVariant::Standard).unwrap();
        ben.write_assignment(vec![1, 2]).unwrap();
        ben.finish().unwrap();
    }
    let writer = session.finish_into_writer(1);
    let buf = writer.finish().unwrap();
    let reader = BendlReader::open(Cursor::new(buf.into_inner())).unwrap();
    assert!(reader.is_finalized());
    assert_eq!(reader.sample_count(), Some(1));
}

// ── Plan verification tests ──────────────────────────────────────

/// Verification #7: dropping a `BendlStreamSession` mid-flight must leave the bundle on disk
/// unfinalized (no directory written, header `finalized != FINALIZED_YES`).
#[test]
fn bundle_streaming_session_drop_leaves_unfinalized() {
    let mut buf: Vec<u8> = Vec::new();
    {
        let writer = BendlWriter::new(Cursor::new(&mut buf), AssignmentFormat::Ben).unwrap();
        let mut session = writer.into_stream_session().unwrap();
        session.write_all(b"STANDARD BEN FILE\x00partial").unwrap();
        // Drop without finish_into_writer.
        drop(session);
    }

    // The bundle on disk has a provisional header, no directory.
    let header = BendlHeader::read_from(&mut Cursor::new(&buf)).unwrap();
    assert_eq!(
        header.finalized, FINALIZED_NO,
        "dropped session must leave the bundle unfinalized"
    );
}

/// Verification #9: `BendlStreamSession::write` must increment its internal byte counter by the
/// returned write count, not by the requested buffer length, so partial writes are accounted
/// correctly and the finalized header's `stream_len` matches the actual byte count of the stream
/// region.
#[test]
fn stream_session_partial_writes_account_returned_bytes() {
    use std::io::{self, Cursor as IoCursor, SeekFrom};

    /// Inner writer that always reports `cap` bytes written per call, regardless of the buffer
    /// length, but writes the matching prefix.
    struct ShortWriter {
        cursor: IoCursor<Vec<u8>>,
        cap: usize,
    }

    impl Write for ShortWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let n = buf.len().min(self.cap);
            self.cursor.write_all(&buf[..n])?;
            Ok(n)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.cursor.flush()
        }
    }

    impl Seek for ShortWriter {
        fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
            self.cursor.seek(pos)
        }
    }

    let inner = ShortWriter {
        cursor: IoCursor::new(Vec::new()),
        cap: 3,
    };
    let writer = BendlWriter::new(inner, AssignmentFormat::Ben).unwrap();
    let mut session = writer.into_stream_session().unwrap();

    // Drive a few partial writes; total written should equal the sum of the returned `n` from each
    // call.
    let mut total_returned: u64 = 0;
    for _ in 0..5 {
        let n = session.write(b"hello world").unwrap();
        total_returned += n as u64;
    }
    assert_eq!(session.bytes_written(), total_returned);

    // Finalize and confirm `stream_len` in the patched header agrees.
    let writer = session.finish_into_writer(0);
    let final_inner = writer.finish().unwrap();
    let mut bundle_buf = final_inner.cursor.into_inner();
    let header = BendlHeader::read_from(&mut Cursor::new(&mut bundle_buf)).unwrap();
    assert_eq!(header.stream_len, total_returned);
}

// =====================================================================
// Stream CRC32C verification
// =====================================================================
//
// Tests pin the writer→reader round-trip of stream_checksum and the behavioral contract of the
// verified stream reader APIs across both the raw-copy path (assignment_stream_reader) and the
// decoded path (open_assignment_reader / count_samples / write_all_jsonl). Each verified API
// surfaces ChecksumError::Mismatch when the stored stream_checksum is corrupted in-place.

/// Build a finalized bundle containing a small plain-BEN stream with `count` samples. Returns
/// `(bundle_bytes, samples)`.
fn make_ben_stream_bundle(count: usize) -> (Vec<u8>, Vec<Vec<u16>>) {
    let samples: Vec<Vec<u16>> = (0..count).map(|i| vec![i as u16, (i + 1) as u16]).collect();
    let writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    let mut session = writer.into_stream_session().unwrap();
    {
        let mut ben = BenStreamWriter::for_ben(&mut session, BenVariant::Standard).unwrap();
        for s in &samples {
            ben.write_assignment(s.clone()).unwrap();
        }
        ben.finish().unwrap();
    }
    let writer = session.finish_into_writer(count as i64);
    let buf = writer.finish().unwrap().into_inner();
    (buf, samples)
}

/// Corrupt the stored `stream_checksum` field in-place by flipping a byte at header offset 20.
fn corrupt_stream_checksum(bytes: &mut [u8]) {
    bytes[20] ^= 0xFF;
}

/// Flip a byte in the stream payload to corrupt the stream contents without changing its length.
fn corrupt_stream_payload(bytes: &mut [u8], reader: &mut BendlReader<Cursor<Vec<u8>>>) {
    let (offset, len) = reader.assignment_stream_range().unwrap();
    assert!(
        len > 0,
        "stream must be non-empty to corrupt a payload byte"
    );
    // Flip the last byte of the stream region.
    bytes[(offset + len - 1) as usize] ^= 0x01;
}

#[test]
fn writer_sets_header_flag_stream_checksum_on_finalization() {
    let (buf, _) = make_ben_stream_bundle(3);
    let header = BendlHeader::read_from(&mut Cursor::new(&buf)).unwrap();
    assert!(
        header.flags & HEADER_FLAG_STREAM_CHECKSUM != 0,
        "HEADER_FLAG_STREAM_CHECKSUM must be set after finalization"
    );
    assert_ne!(
        header.stream_checksum, 0,
        "stream_checksum must be non-zero for a non-empty stream"
    );
}

#[test]
fn writer_sets_stream_checksum_zero_for_empty_stream() {
    // An empty stream has CRC32C(b"") = 0x00000000.
    let writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    let buf = writer.finish().unwrap().into_inner();
    let header = BendlHeader::read_from(&mut Cursor::new(&buf)).unwrap();
    assert!(header.flags & HEADER_FLAG_STREAM_CHECKSUM != 0);
    assert_eq!(header.stream_checksum, 0);
}

#[test]
fn assignment_stream_reader_verified_round_trips_stream_bytes() {
    let (buf, _) = make_ben_stream_bundle(3);
    // Capture the raw stream bytes first for comparison.
    let mut r = BendlReader::open(Cursor::new(buf.clone())).unwrap();
    let (off, len) = r.assignment_stream_range().unwrap();
    let raw_stream: Vec<u8> = buf[off as usize..(off + len) as usize].to_vec();
    drop(r);

    let mut reader = BendlReader::open(Cursor::new(buf)).unwrap();
    let mut got = Vec::new();
    reader
        .assignment_stream_reader()
        .unwrap()
        .read_to_end(&mut got)
        .unwrap();
    assert_eq!(got, raw_stream);
}

#[test]
fn assignment_stream_reader_detects_corrupt_stored_checksum() {
    let (mut buf, _) = make_ben_stream_bundle(3);
    corrupt_stream_checksum(&mut buf);
    let mut reader = BendlReader::open(Cursor::new(buf)).unwrap();
    let mut sink = Vec::new();
    let err = reader
        .assignment_stream_reader()
        .unwrap()
        .read_to_end(&mut sink)
        .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    let inner = err
        .get_ref()
        .and_then(|e| e.downcast_ref::<ChecksumError>())
        .expect("inner ChecksumError");
    assert!(
        matches!(
            inner,
            ChecksumError::Mismatch {
                target: ChecksumTarget::Stream,
                ..
            }
        ),
        "expected Stream Mismatch, got {inner:?}"
    );
}

#[test]
fn verify_stream_checksum_passes_on_intact_bundle() {
    let (buf, _) = make_ben_stream_bundle(3);
    let mut reader = BendlReader::open(Cursor::new(buf)).unwrap();
    reader.verify_stream_checksum().unwrap();
}

#[test]
fn verify_stream_checksum_fails_on_corrupt_stored_checksum() {
    let (mut buf, _) = make_ben_stream_bundle(3);
    corrupt_stream_checksum(&mut buf);
    let mut reader = BendlReader::open(Cursor::new(buf)).unwrap();
    let err = reader.verify_stream_checksum().unwrap_err();
    assert!(
        matches!(
            err,
            BendlReadError::Checksum(ChecksumError::Mismatch {
                target: ChecksumTarget::Stream,
                ..
            })
        ),
        "expected Mismatch(Stream), got {err:?}"
    );
}

#[test]
fn verify_stream_checksum_fails_on_corrupt_stream_payload() {
    let (mut buf, _) = make_ben_stream_bundle(3);
    {
        let mut r = BendlReader::open(Cursor::new(buf.clone())).unwrap();
        corrupt_stream_payload(&mut buf, &mut r);
    }
    let mut reader = BendlReader::open(Cursor::new(buf)).unwrap();
    let err = reader.verify_stream_checksum().unwrap_err();
    assert!(
        matches!(
            err,
            BendlReadError::Checksum(ChecksumError::Mismatch {
                target: ChecksumTarget::Stream,
                ..
            })
        ),
        "expected Mismatch(Stream), got {err:?}"
    );
}

#[test]
fn open_assignment_reader_iterator_detects_corrupt_stored_checksum() {
    let (mut buf, samples) = make_ben_stream_bundle(3);
    corrupt_stream_checksum(&mut buf);
    let mut reader = BendlReader::open(Cursor::new(buf)).unwrap();
    let mut decoder = reader.open_assignment_reader().unwrap();
    // Consume all real records; then the next call should report Mismatch.
    let mut decoded_count = 0usize;
    loop {
        match decoder.next() {
            Some(Ok(_)) => {
                decoded_count += 1;
            }
            Some(Err(e)) => {
                assert_eq!(e.kind(), std::io::ErrorKind::InvalidData);
                let inner = e
                    .get_ref()
                    .and_then(|x| x.downcast_ref::<ChecksumError>())
                    .expect("inner ChecksumError");
                assert!(
                    matches!(
                        inner,
                        ChecksumError::Mismatch {
                            target: ChecksumTarget::Stream,
                            ..
                        }
                    ),
                    "expected Stream Mismatch, got {inner:?}"
                );
                break;
            }
            None => panic!("expected ChecksumMismatch before None, got None"),
        }
    }
    // Subsequent calls must return None (not repeat the error).
    assert!(
        decoder.next().is_none(),
        "expected None after mismatch reported"
    );
    assert_eq!(decoded_count, samples.len());
}

#[test]
fn count_samples_detects_corrupt_stored_checksum() {
    let (mut buf, _) = make_ben_stream_bundle(4);
    corrupt_stream_checksum(&mut buf);
    let mut reader = BendlReader::open(Cursor::new(buf)).unwrap();
    let decoder = reader.open_assignment_reader().unwrap();
    let err = decoder.count_samples().unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    let inner = err
        .get_ref()
        .and_then(|x| x.downcast_ref::<ChecksumError>())
        .expect("inner ChecksumError");
    assert!(
        matches!(
            inner,
            ChecksumError::Mismatch {
                target: ChecksumTarget::Stream,
                ..
            }
        ),
        "expected Stream Mismatch, got {inner:?}"
    );
}

#[test]
fn write_all_jsonl_detects_corrupt_stored_checksum() {
    let (mut buf, _) = make_ben_stream_bundle(3);
    corrupt_stream_checksum(&mut buf);
    let mut reader = BendlReader::open(Cursor::new(buf)).unwrap();
    let mut decoder = reader.open_assignment_reader().unwrap();
    let err = decoder.write_all_jsonl(std::io::sink()).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    let inner = err
        .get_ref()
        .and_then(|x| x.downcast_ref::<ChecksumError>())
        .expect("inner ChecksumError");
    assert!(
        matches!(
            inner,
            ChecksumError::Mismatch {
                target: ChecksumTarget::Stream,
                ..
            }
        ),
        "expected Stream Mismatch, got {inner:?}"
    );
}

#[test]
fn for_each_assignment_detects_corrupt_stored_checksum_when_driven_to_eof() {
    let (mut buf, _) = make_ben_stream_bundle(3);
    corrupt_stream_checksum(&mut buf);
    let mut reader = BendlReader::open(Cursor::new(buf)).unwrap();
    let mut decoder = reader.open_assignment_reader().unwrap();
    // Callback always returns Ok(true) so it drives to natural EOF.
    let err = decoder.for_each_assignment(|_, _| Ok(true)).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    let inner = err
        .get_ref()
        .and_then(|x| x.downcast_ref::<ChecksumError>())
        .expect("inner ChecksumError");
    assert!(
        matches!(
            inner,
            ChecksumError::Mismatch {
                target: ChecksumTarget::Stream,
                ..
            }
        ),
        "expected Stream Mismatch, got {inner:?}"
    );
}

#[test]
fn open_assignment_reader_intact_bundle_round_trips_count_samples() {
    let (buf, samples) = make_ben_stream_bundle(5);
    let mut reader = BendlReader::open(Cursor::new(buf)).unwrap();
    let decoder = reader.open_assignment_reader().unwrap();
    let n = decoder.count_samples().unwrap();
    assert_eq!(n, samples.len());
}

// =====================================================================
// Forward-compat: appender preserves unknown asset-flag bits on existing entries
// =====================================================================

/// Build a finalized BENDL bundle with a single custom asset whose `asset_flags` carries a
/// reserved (unknown-in-v1.0.0) bit alongside the known `ASSET_FLAG_CHECKSUM` bit. Used to
/// confirm that `BendlAppender::commit` clones the existing entry verbatim, preserving the
/// reserved bit so future readers that grow the spec are not silently downgraded by today's
/// appender.
fn bundle_with_reserved_asset_flag_bit() -> (Vec<u8>, u16) {
    const RESERVED_BIT_7: u16 = 1 << 7;
    let payload = b"forward-compat asset".to_vec();
    let mut bytes = Vec::new();
    bytes.extend(std::iter::repeat_n(0u8, HEADER_SIZE));
    let payload_offset = bytes.len() as u64;
    bytes.extend_from_slice(&payload);

    let directory_offset = bytes.len() as u64;
    let crc = crc32c::crc32c(&payload).to_le_bytes().to_vec();
    let entries = vec![BendlDirectoryEntry {
        asset_type: ASSET_TYPE_CUSTOM,
        asset_flags: ASSET_FLAG_CHECKSUM | RESERVED_BIT_7,
        name: "forward.bin".to_string(),
        payload_offset,
        payload_len: payload.len() as u64,
        checksum: Some(crc),
    }];
    let directory = encode_directory(&entries).unwrap();
    bytes.extend_from_slice(&directory);

    let header = BendlHeader {
        magic: BENDL_MAGIC,
        major_version: BENDL_MAJOR_VERSION,
        minor_version: BENDL_MINOR_VERSION,
        finalized: FINALIZED_YES,
        assignment_format: AssignmentFormat::Ben.to_u8(),
        alignment_padding: 0,
        flags: HEADER_FLAG_STREAM_CHECKSUM,
        stream_checksum: 0,
        directory_offset,
        directory_len: directory.len() as u64,
        stream_offset: HEADER_SIZE as u64,
        stream_len: 0,
        sample_count: 0,
    };
    bytes[..HEADER_SIZE].copy_from_slice(&header.to_bytes());
    (bytes, RESERVED_BIT_7)
}

// =====================================================================
// Concurrent reader access
// =====================================================================

#[test]
fn two_parallel_readers_against_the_same_bundle_agree() {
    // Two `BendlReader`s opened from independent `Cursor`s over an `Arc<Vec<u8>>` shared
    // buffer must produce identical results across the full accessor surface. The bundle
    // bytes are immutable for the duration of the test — this pins that the reader holds no
    // shared mutable state internally (e.g., no static caches, no thread-local position
    // tracking) that would let one thread's reads scramble the other's.
    //
    // Reader-during-append is intentionally not covered here. The append path preserves the old
    // authoritative directory until the final header patch, so payload/directory writes alone do
    // not create a torn reader state; concurrent access to the same mutable file handle is still an
    // integration-level filesystem contract rather than a property of immutable reader state.
    use std::sync::Arc;
    use std::thread;

    let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    writer
        .add_asset(
            ASSET_TYPE_GRAPH,
            "graph.json",
            br#"{"nodes":4,"edges":[[0,1],[1,2],[2,3]]}"#,
            AddAssetOptions::defaults().json().compress(),
        )
        .unwrap();
    writer
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "extra.bin",
            b"a bit of custom payload",
            AddAssetOptions::defaults().raw(),
        )
        .unwrap();
    let mut session = writer.into_stream_session().unwrap();
    session.write_all(b"STANDARD BEN FILE\x00\x01\x02").unwrap();
    let writer = session.finish_into_writer(1);
    let bytes = writer.finish().unwrap().into_inner();
    let shared = Arc::new(bytes);

    // Pre-compute the expected (asset_name, decoded_bytes) pairs on the main thread so each
    // worker has a stable oracle to compare against without re-deriving it from the same
    // reader API under test.
    let oracle: Vec<(String, Vec<u8>)> = {
        let mut reader = BendlReader::open(Cursor::new(shared.as_slice())).unwrap();
        let entries: Vec<_> = reader.assets().to_vec();
        entries
            .iter()
            .map(|e| (e.name.clone(), reader.asset_bytes(e).unwrap()))
            .collect()
    };

    let mut handles = Vec::new();
    for _ in 0..4 {
        let shared = Arc::clone(&shared);
        let oracle = oracle.clone();
        handles.push(thread::spawn(move || {
            for _ in 0..16 {
                let mut reader = BendlReader::open(Cursor::new(shared.as_slice())).unwrap();
                assert!(reader.is_finalized());
                assert!(reader.header().has_stream_checksum());
                reader
                    .verify_all_asset_checksums()
                    .expect("asset checksums must verify under concurrent readers");
                reader
                    .verify_stream_checksum()
                    .expect("stream checksum must verify under concurrent readers");
                let entries: Vec<_> = reader.assets().to_vec();
                for (entry, (expected_name, expected_bytes)) in entries.iter().zip(oracle.iter()) {
                    assert_eq!(&entry.name, expected_name);
                    let got = reader.asset_bytes(entry).unwrap();
                    assert_eq!(&got, expected_bytes);
                }
            }
        }));
    }
    for h in handles {
        h.join().expect("worker thread panicked");
    }
}

#[test]
fn appender_preserves_unknown_asset_flag_bits_on_existing_entries() {
    // Open a bundle whose pre-existing entry has a reserved bit set; commit a new asset; reopen
    // and assert the reserved bit is still set on the original entry. The new entry must not
    // pick up any reserved bits.
    let (initial_bytes, reserved_bit) = bundle_with_reserved_asset_flag_bit();
    let known_v1_bits: u16 =
        ASSET_FLAG_CHECKSUM | ASSET_FLAG_XZ | crate::io::bundle::format::ASSET_FLAG_JSON;

    let mut appender = BendlAppender::open(Cursor::new(initial_bytes)).unwrap();
    appender
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "new.bin",
            b"new asset bytes",
            AddAssetOptions::defaults().raw(),
        )
        .unwrap();
    let final_bytes = appender.commit().unwrap().into_inner();

    let reader = BendlReader::open(Cursor::new(final_bytes)).unwrap();

    let original = reader.find_asset_by_name("forward.bin").unwrap();
    assert_ne!(
        original.asset_flags & reserved_bit,
        0,
        "appender must not clear reserved bits on existing entries"
    );

    let new_entry = reader.find_asset_by_name("new.bin").unwrap();
    assert_eq!(
        new_entry.asset_flags & !known_v1_bits,
        0,
        "appender must not set any unknown bits on newly written entries"
    );
}

// =====================================================================
// validation-failure paths and accessors
// =====================================================================

#[test]
fn stream_session_start_offset_returns_recorded_value() {
    // `BendlStreamSession::start_offset` records the file position at session-construction time so
    // a caller can later size the stream region. The getter is a one-line method but is the only
    // way to read this value, so pin it explicitly.
    let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    writer
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "blob.bin",
            b"abcdef",
            AddAssetOptions::defaults().raw(),
        )
        .unwrap();
    let session = writer.into_stream_session().unwrap();
    // Header is 64 bytes; one 6-byte asset payload follows → start_offset = 70.
    assert_eq!(session.start_offset(), HEADER_SIZE as u64 + 6);
}

#[test]
fn writer_failed_asset_write_does_not_poison_registry() {
    struct FailOnceAfterHeader {
        inner: Cursor<Vec<u8>>,
        failed: bool,
    }

    impl FailOnceAfterHeader {
        fn new() -> Self {
            Self {
                inner: Cursor::new(Vec::new()),
                failed: false,
            }
        }
    }

    impl Write for FailOnceAfterHeader {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if !self.failed && self.inner.position() >= HEADER_SIZE as u64 {
                self.failed = true;
                return Err(std::io::Error::other(
                    "simulated payload write failure",
                ));
            }
            self.inner.write(buf)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.inner.flush()
        }
    }

    impl Seek for FailOnceAfterHeader {
        fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
            self.inner.seek(pos)
        }
    }

    let mut writer = BendlWriter::new(FailOnceAfterHeader::new(), AssignmentFormat::Ben).unwrap();
    let err = writer
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "retry.bin",
            b"payload",
            AddAssetOptions::defaults().raw(),
        )
        .unwrap_err();
    assert!(matches!(err, BendlWriteError::Io(_)));

    writer
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "retry.bin",
            b"payload",
            AddAssetOptions::defaults().raw(),
        )
        .unwrap();
}

#[test]
fn writer_duplicate_name_after_singleton_check_leaves_writer_usable() {
    // A custom asset can claim the standardized name of a known singleton type. A later attempt to
    // add the actual singleton must fail cleanly during validation, without reserving any
    // singleton state or making the writer unusable for unrelated additions.
    let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    writer
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "graph.json",
            b"squatting on the canonical name",
            AddAssetOptions::defaults().raw(),
        )
        .unwrap();
    let err = writer
        .add_asset(
            ASSET_TYPE_GRAPH,
            "graph.json",
            b"the real graph",
            AddAssetOptions::defaults().json().compress(),
        )
        .unwrap_err();
    assert!(
        matches!(err, BendlWriteError::DuplicateName(ref n) if n == "graph.json"),
        "expected DuplicateName, got {err:?}"
    );

    writer
        .add_asset(
            ASSET_TYPE_METADATA,
            "metadata.json",
            br#"{"v":1}"#,
            AddAssetOptions::defaults().json().raw(),
        )
        .unwrap();
}

#[test]
fn appender_duplicate_name_after_singleton_check_leaves_appender_usable() {
    // Same validation contract for BendlAppender: a singleton-name collision must fail without
    // reserving pending singleton state, so the appender remains usable for unrelated additions.
    let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    writer
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "graph.json",
            b"squatter",
            AddAssetOptions::defaults().raw(),
        )
        .unwrap();
    let session = writer.into_stream_session().unwrap();
    let writer = session.finish_into_writer(0);
    let bundle = writer.finish().unwrap().into_inner();

    let mut appender = BendlAppender::open(Cursor::new(bundle)).unwrap();
    let err = appender
        .add_asset(
            ASSET_TYPE_GRAPH,
            "graph.json",
            b"the real graph",
            AddAssetOptions::defaults().json().compress(),
        )
        .unwrap_err();
    assert!(
        matches!(err, BendlWriteError::DuplicateName(ref n) if n == "graph.json"),
        "expected DuplicateName, got {err:?}"
    );

    appender
        .add_asset(
            ASSET_TYPE_METADATA,
            "metadata.json",
            br#"{"v":1}"#,
            AddAssetOptions::defaults().json().raw(),
        )
        .unwrap();
}
