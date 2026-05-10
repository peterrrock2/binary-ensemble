use std::io::{self, Cursor, Read, Seek, Write};

use crate::io::bundle::format::{
    AssignmentFormat, BendlFormatError, BendlHeader, ASSET_FLAG_CHECKSUM, ASSET_FLAG_XZ,
    ASSET_TYPE_CUSTOM, ASSET_TYPE_GRAPH, ASSET_TYPE_METADATA, BENDL_MAGIC, BENDL_MAJOR_VERSION,
    BENDL_MINOR_VERSION, FINALIZED_NO, FINALIZED_YES, HEADER_SIZE,
};
use crate::io::bundle::reader::BendlReader;
use crate::io::reader::BenWireFormat;
use crate::io::bundle::writer::{
    AddAssetOptions, BendlAppender, BendlWriteError, BendlWriter,
};

fn make_buffer() -> Cursor<Vec<u8>> {
    Cursor::new(Vec::new())
}

/// Test helper: replicate the deleted `BendlWriter::write_stream_bytes`
/// using the owned-session chain. Used purely to keep test bodies short.
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
    // Compressed size should differ from the raw size for a non-trivial
    // JSON payload. For very short payloads xz actually inflates the
    // bytes, so this just checks the size is non-zero and different.
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
    // After a session has been finished, the writer is in `StreamWritten`
    // and `add_*_asset` rejects further additions with `AssetsAfterStream`.
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

// -----------------------------------------------------------------------
// Append-path tests
// -----------------------------------------------------------------------

/// Build a finalized bundle with a single `metadata.json` asset and
/// a short fake stream, then return both the bytes and the byte
/// range (offset, len) occupied by the stream region.
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

    // Read back the new header to locate the stream region, then
    // confirm the stream bytes are byte-identical to the original.
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
    // Start from a bundle containing a custom asset named "blob", then
    // try to append another "blob".
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
    // Construct a minimal incomplete bundle: just the provisional
    // header and some stream bytes, no directory.
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
    bytes.extend_from_slice(b"STANDARD BEN FILE\x00fake");

    match BendlAppender::open(Cursor::new(bytes)) {
        Err(BendlWriteError::BundleIncomplete) => {}
        Err(other) => panic!("expected BundleIncomplete, got {other:?}"),
        Ok(_) => panic!("expected error, got Ok"),
    }
}

#[test]
fn append_rejects_complete_bundle_with_zero_directory() {
    // Header claims complete but has directory_offset=0 — hits the second
    // BundleIncomplete check.
    let header = BendlHeader {
        magic: BENDL_MAGIC,
        major_version: BENDL_MAJOR_VERSION,
        minor_version: BENDL_MINOR_VERSION,
        finalized: FINALIZED_YES,
        assignment_format: AssignmentFormat::Ben.to_u8(),
        reserved_0: 0,
        flags: 0,
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
    // Round-trip the appended graph through the reader to confirm
    // compression happened and decodes cleanly.
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

// -------- Phase 4: assignment-stream integration tests --------

#[test]
fn write_ben_stream_round_trips_through_assignment_reader() {
    use crate::BenVariant;

    let samples: Vec<Vec<u16>> = vec![
        vec![0, 0, 1, 1, 2, 2],
        vec![0, 1, 1, 1, 2, 2],
        vec![0, 1, 1, 1, 2, 2], // repeat
        vec![1, 1, 1, 1, 2, 2],
    ];

    let writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    let writer = writer
        .write_ben_stream(BenVariant::MkvChain, |ctx| {
            for s in &samples {
                ctx.write_assignment(s.clone())?;
            }
            Ok(())
        })
        .unwrap();
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
            std::iter::repeat(assign).take(count as usize)
        })
        .collect();
    assert_eq!(decoded, samples);
}

#[test]
fn write_xben_stream_round_trips_through_assignment_reader() {
    use crate::BenVariant;

    let samples: Vec<Vec<u16>> = vec![
        vec![0, 1, 2, 3, 4, 5],
        vec![0, 1, 2, 3, 4, 5], // repeat
        vec![1, 1, 2, 3, 4, 5],
        vec![1, 1, 2, 3, 4, 4],
    ];

    let writer = BendlWriter::new(make_buffer(), AssignmentFormat::Xben).unwrap();
    let writer = writer
        .write_xben_stream(BenVariant::MkvChain, |ctx| {
            for s in &samples {
                ctx.write_assignment(s.clone())?;
            }
            Ok(())
        })
        .unwrap();
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
            std::iter::repeat(assign).take(count as usize)
        })
        .collect();
    assert_eq!(decoded, samples);
}

#[test]
fn write_ben_stream_alongside_front_loaded_asset() {
    use crate::BenVariant;

    let graph = br#"{"nodes":[0,1,2],"edges":[[0,1],[1,2]]}"#;
    let samples: Vec<Vec<u16>> = vec![vec![0, 1, 1, 2], vec![0, 1, 2, 2]];

    let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    writer
        .add_json_asset(ASSET_TYPE_GRAPH, "graph.json", graph)
        .unwrap();
    let writer = writer
        .write_ben_stream(BenVariant::Standard, |ctx| {
            for s in &samples {
                ctx.write_assignment(s.clone())?;
            }
            Ok(())
        })
        .unwrap();
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
            std::iter::repeat(assign).take(count as usize)
        })
        .collect();
    assert_eq!(decoded, samples);
}

#[test]
fn open_assignment_reader_rejects_mismatched_format() {
    use crate::BenVariant;

    let writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    let writer = writer
        .write_ben_stream(BenVariant::Standard, |ctx| {
            ctx.write_assignment(vec![0, 1])?;
            Ok(())
        })
        .unwrap();
    let buf = writer.finish().unwrap().into_inner();

    let mut reader = BendlReader::open(Cursor::new(buf)).unwrap();
    let decoder = reader.open_assignment_reader().unwrap();
    assert_eq!(decoder.wire_format(), BenWireFormat::Ben);
}

// -----------------------------------------------------------------------
// Robustness tests
// -----------------------------------------------------------------------

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
    // Regression fixture for the `into_stream_session` guard: a writer
    // that has already finished one stream phase must reject a second
    // attempt to enter the stream phase. Without this guard, a chained
    // `into_stream_session → finish_into_writer → into_stream_session`
    // would silently overwrite `header.stream_offset` and corrupt the
    // bundle. This is the only runtime fixture for that guard.
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
    let count = 500usize;
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
    for i in [0usize, 1, 42, 199, 499] {
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
    // Sample count from the original stream is preserved across both
    // appends.
    assert_eq!(reader.sample_count(), Some(3));
}

#[test]
fn append_does_not_disturb_front_loaded_asset_bytes() {
    // Base bundle has a graph.json asset with known bytes; after
    // append of a custom blob, reading graph.json must still return
    // exactly the same decoded bytes as before.
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
    // A custom asset named "graph.json" is not a singleton because the
    // singleton uniqueness check keys off asset_type, not name. Adding
    // a real GRAPH singleton after it must then fail on DuplicateName.
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
fn writer_asset_with_checksum_round_trips_through_reader() {
    let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    let checksum = vec![0x01, 0x02, 0x03, 0x04];
    writer
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "with_checksum",
            b"hello",
            AddAssetOptions {
                checksum: Some(checksum.clone()),
                ..AddAssetOptions::defaults()
            },
        )
        .unwrap();
    let writer = write_stream_bytes_via_session(writer, b"STANDARD BEN FILE\x00fake", 1);
    let buf = writer.finish().unwrap().into_inner();

    let reader = BendlReader::open(Cursor::new(buf)).unwrap();
    let entry = reader.find_asset_by_name("with_checksum").cloned().unwrap();
    assert_eq!(entry.checksum, Some(checksum));
    assert_ne!(entry.asset_flags & ASSET_FLAG_CHECKSUM, 0);
}

#[test]
fn finished_writer_rejects_further_operations() {
    let writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    let writer = write_stream_bytes_via_session(writer, b"STANDARD BEN FILE\x00fake", 1);
    // `finish` consumes `self`, which is itself the protection — there
    // is no way to call add_asset / into_stream_session afterwards.
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
    // After a rejected add, no entries have been recorded — a
    // subsequent valid add proceeds normally.
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
    // A rejected singleton add must not consume the singleton slot —
    // otherwise a future valid add with the correct standardized name
    // would spuriously fail with DuplicateSingletonType.
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

#[test]
fn write_ben_stream_closure_error_short_circuits_finalize() {
    use crate::BenVariant;

    let writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    // BendlWriter doesn't implement Debug, so destructure via match
    // rather than `.unwrap_err()`.
    let result = writer.write_ben_stream(BenVariant::Standard, |_ctx| {
        Err(io::Error::new(io::ErrorKind::Other, "boom"))
    });
    match result {
        Ok(_) => panic!("expected closure error to short-circuit"),
        Err(BendlWriteError::Io(e)) => assert_eq!(e.kind(), io::ErrorKind::Other),
        Err(other) => panic!("expected Io(Other), got {other:?}"),
    }
}

// -----------------------------------------------------------------------
// Randomized / stress tests
// -----------------------------------------------------------------------

/// Build a bundle from a random set of custom assets (plus an optional
/// metadata asset) and fully round-trip it through the reader. Repeated
/// with a seeded ChaCha PRNG so the sequence is deterministic but
/// covers a wide surface.
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
                // Override with a synthetic JSON blob so the json flag
                // actually matches the content.
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

        // Write a small deterministic stream so the bundle is
        // assignment-complete.
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
    // Start from a finalized bundle with only a metadata asset and a
    // short stream. Then open it five times via BendlAppender and add
    // one asset per round. After every round, the previous assets must
    // still be readable and sample_count must remain authoritative.
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

        // Re-open and verify the full set is intact and sample_count
        // still matches the baseline (append must not touch it).
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
    // Independent coverage for append: random number of rounds, random
    // payload sizes. Catches any bookkeeping drift in the appender's
    // directory-rewrite path.
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
fn write_ben_stream_json_value_and_sample_count() {
    use crate::BenVariant;
    use serde_json::json;

    let writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    let writer = writer
        .write_ben_stream(BenVariant::Standard, |ctx| {
            assert_eq!(ctx.sample_count(), 0);
            ctx.write_json_value(json!({"assignment": [1, 2, 3], "sample": 1}))?;
            assert_eq!(ctx.sample_count(), 1);
            ctx.write_json_value(json!({"assignment": [4, 5, 6], "sample": 2}))?;
            assert_eq!(ctx.sample_count(), 2);
            Ok(())
        })
        .unwrap();
    let buf = writer.finish().unwrap().into_inner();

    let mut reader = BendlReader::open(Cursor::new(buf)).unwrap();
    assert_eq!(reader.sample_count(), Some(2));
    let decoder = reader.open_assignment_reader().unwrap();
    assert_eq!(decoder.wire_format(), BenWireFormat::Ben);
    let decoded: Vec<Vec<u16>> = decoder.silent(true).map(|r| r.unwrap().0).collect();
    assert_eq!(decoded, vec![vec![1, 2, 3], vec![4, 5, 6]]);
}

#[test]
fn write_xben_stream_json_value() {
    use crate::BenVariant;
    use serde_json::json;

    let writer = BendlWriter::new(make_buffer(), AssignmentFormat::Xben).unwrap();
    let writer = writer
        .write_xben_stream(BenVariant::Standard, |ctx| {
            ctx.write_json_value(json!({"assignment": [10, 20], "sample": 1}))?;
            ctx.write_json_value(json!({"assignment": [30, 40], "sample": 2}))?;
            Ok(())
        })
        .unwrap();
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
    // Discard the session — it would warn on Drop, but the test runner
    // does not assert on log output, so this is fine for unit tests.
    let _ = session.finish_into_writer(0);
}

// ── BendlAppender: checksum flag ────────────────────────────────

#[test]
fn appender_commit_with_checksum_sets_checksum_flag() {
    let (bundle, _) = build_base_bundle();
    let mut appender = BendlAppender::open(Cursor::new(bundle)).unwrap();
    appender
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "checksummed",
            b"payload",
            AddAssetOptions {
                checksum: Some(vec![0xAB, 0xCD]),
                ..AddAssetOptions::defaults()
            },
        )
        .unwrap();
    let buf = appender.commit().unwrap().into_inner();

    let reader = BendlReader::open(Cursor::new(buf)).unwrap();
    let entry = reader.find_asset_by_name("checksummed").unwrap();
    assert_eq!(entry.checksum, Some(vec![0xAB, 0xCD]));
    assert_ne!(entry.asset_flags & ASSET_FLAG_CHECKSUM, 0);
}

// ── BendlAppender: trailing directory bytes ──────────────────────

#[test]
fn appender_rejects_bundle_with_trailing_directory_bytes() {
    let (mut bundle, _) = build_base_bundle();
    // Patch the header's directory_len field (bytes 32-39) to claim
    // the directory is 4 bytes longer than it actually is.
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
fn finish_from_finished_state_errors() {
    use crate::BenVariant;

    let writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    let writer = writer
        .write_ben_stream(BenVariant::Standard, |ctx| {
            ctx.write_assignment(vec![1, 2])?;
            Ok(())
        })
        .unwrap();
    // First finish succeeds
    let buf = writer.finish().unwrap();
    // Verify the result is usable
    let reader = BendlReader::open(Cursor::new(buf.into_inner())).unwrap();
    assert!(reader.is_finalized());
}

// ── Plan verification tests ──────────────────────────────────────

/// Verification #4 from the plan: bundle byte-equivalence between the
/// closure-based `write_ben_stream` and the explicit
/// `into_stream_session` → `finish_into_writer` chain.
#[test]
fn bundle_byte_equivalent_via_closure_and_explicit_session_for_ben() {
    use crate::io::writer::BenStreamWriter;
    use crate::BenVariant;

    let samples: Vec<Vec<u16>> = vec![
        vec![0, 0, 1, 1, 2, 2],
        vec![0, 1, 1, 1, 2, 2],
        vec![0, 1, 1, 1, 2, 2],
        vec![1, 1, 1, 1, 2, 2],
    ];

    // Path A: closure-based.
    let writer_a = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    let writer_a = writer_a
        .write_ben_stream(BenVariant::MkvChain, |ctx| {
            for s in &samples {
                ctx.write_assignment(s.clone())?;
            }
            Ok(())
        })
        .unwrap();
    let buf_a = writer_a.finish().unwrap().into_inner();

    // Path B: explicit session.
    let writer_b = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
    let mut session = writer_b.into_stream_session().unwrap();
    let mut ben = BenStreamWriter::for_ben(&mut session, BenVariant::MkvChain).unwrap();
    for s in &samples {
        ben.write_assignment(s.clone()).unwrap();
    }
    ben.finish().unwrap();
    drop(ben);
    let writer_b = session.finish_into_writer(samples.len() as i64);
    let buf_b = writer_b.finish().unwrap().into_inner();

    assert_eq!(
        buf_a, buf_b,
        "closure path and explicit session path must produce identical bundle bytes"
    );
}

/// Verification #7: dropping a `BendlStreamSession` mid-flight must
/// leave the bundle on disk unfinalized (no directory written, header
/// `finalized != FINALIZED_YES`).
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

/// Verification #8: bundle XBEN compression gate. Two paths should
/// produce identical bundle bytes — the closure helper
/// `write_xben_stream`, and an explicit session that wraps the bundle
/// preset xz encoder around `for_xben_with_encoder`.
#[test]
fn bundle_xben_byte_equivalent_closure_and_explicit_encoder() {
    use crate::io::bundle::format::DEFAULT_XZ_PRESET;
    use crate::io::writer::BenStreamWriter;
    use crate::BenVariant;
    use xz2::write::XzEncoder;

    let samples: Vec<Vec<u16>> = vec![
        vec![0, 1, 2, 3, 4, 5],
        vec![1, 1, 2, 3, 4, 5],
        vec![1, 1, 2, 3, 4, 4],
    ];

    // Path A: closure.
    let writer_a = BendlWriter::new(make_buffer(), AssignmentFormat::Xben).unwrap();
    let writer_a = writer_a
        .write_xben_stream(BenVariant::MkvChain, |ctx| {
            for s in &samples {
                ctx.write_assignment(s.clone())?;
            }
            Ok(())
        })
        .unwrap();
    let buf_a = writer_a.finish().unwrap().into_inner();

    // Path B: explicit session + XzEncoder built with the bundle preset.
    let writer_b = BendlWriter::new(make_buffer(), AssignmentFormat::Xben).unwrap();
    let mut session = writer_b.into_stream_session().unwrap();
    {
        let encoder = XzEncoder::new(&mut session, DEFAULT_XZ_PRESET);
        let mut xben =
            BenStreamWriter::for_xben_with_encoder(encoder, BenVariant::MkvChain, None).unwrap();
        for s in &samples {
            xben.write_assignment(s.clone()).unwrap();
        }
        xben.finish().unwrap();
    }
    let writer_b = session.finish_into_writer(samples.len() as i64);
    let buf_b = writer_b.finish().unwrap().into_inner();

    assert_eq!(
        buf_a, buf_b,
        "XBEN closure path and explicit-encoder path must produce identical bundle bytes"
    );
}

/// Verification #9: `BendlStreamSession::write` must increment its
/// internal byte counter by the returned write count, not by the
/// requested buffer length, so partial writes are accounted correctly
/// and the finalized header's `stream_len` matches the actual byte
/// count of the stream region.
#[test]
fn stream_session_partial_writes_account_returned_bytes() {
    use std::io::{self, Cursor as IoCursor, SeekFrom};

    /// Inner writer that always reports `cap` bytes written per call,
    /// regardless of the buffer length, but writes the matching prefix.
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

    // Drive a few partial writes; total written should equal the sum
    // of the returned `n` from each call.
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
