use super::append::run_append;
use super::args::{AppendArgs, CreateArgs, ExtractArgs, InspectArgs, NamedAsset};
use super::create::run_create;
use super::extract::run_extract;
use super::helpers::format_from_path;
use super::inspect::run_inspect;
use crate::codec::encode::encode_jsonl_to_ben;
use crate::io::bundle::format::AssignmentFormat;
use crate::io::bundle::{BendlReader, BendlWriter};
use crate::test_utils::{sample_bendl_bytes, unique_path};
use clap::Parser;
use std::io::{BufReader, Cursor, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Write a minimal finalized .bendl file and return its path.
fn write_temp_bendl(name: &str, format: AssignmentFormat) -> PathBuf {
    let path = unique_path(name);
    let buf = sample_bendl_bytes(b"STANDARD BEN FILE\x00fake", format);
    std::fs::write(&path, &buf).unwrap();
    path
}

#[test]
fn write_temp_bendl_xben_variant_works() {
    // Exercises the Xben branch of write_temp_bendl.
    let path = write_temp_bendl("xben_helper_check.bendl", AssignmentFormat::Xben);
    let reader = BendlReader::open(BufReader::new(std::fs::File::open(&path).unwrap())).unwrap();
    assert!(reader.is_finalized());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn named_asset_from_str_rejects_empty_name() {
    let err = "=path/to/file".parse::<NamedAsset>().unwrap_err();
    assert!(err.contains("non-empty"));
}

#[test]
fn format_from_path_detects_xben() {
    let fmt = format_from_path(std::path::Path::new("stream.xben")).unwrap();
    assert_eq!(fmt, AssignmentFormat::Xben);
}

#[test]
fn format_from_path_rejects_unknown_extension() {
    let err = format_from_path(std::path::Path::new("archive.tar")).unwrap_err();
    assert!(err.contains("expected .ben or .xben"));
}

#[test]
fn run_create_with_relabel_map_and_custom_asset() {
    let ben = {
        // Must end in .ben so format_from_path recognises it.
        let p = std::env::temp_dir().join(format!(
            "bendl-create-relabel-{}.ben",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let jsonl = b"{\"assignment\":[1,2,3],\"sample\":1}\n";
        let mut b = Vec::new();
        encode_jsonl_to_ben(Cursor::new(jsonl), &mut b, crate::BenVariant::Standard).unwrap();
        std::fs::write(&p, &b).unwrap();
        p
    };
    let relabel = unique_path("create_relabel_map.json");
    std::fs::write(&relabel, b"{\"0\":1,\"1\":0}").unwrap();
    let custom = unique_path("create_custom.bin");
    std::fs::write(&custom, b"custom bytes").unwrap();
    let out = unique_path("create_with_assets.bendl");

    let asset_str = format!("myblob={}", custom.display());
    let args = CreateArgs {
        input: ben.clone(),
        output: out.clone(),
        graph: None,
        metadata: None,
        node_permutation_map: Some(relabel.clone()),
        assets: vec![asset_str.parse().unwrap()],
        overwrite: false,
        graph_raw: false,
        asset_compression_level: None,
    };
    run_create(args).unwrap();

    let reader = BendlReader::open(BufReader::new(std::fs::File::open(&out).unwrap())).unwrap();
    assert!(reader
        .find_asset_by_name("node_permutation_map.json")
        .is_some());
    assert!(reader.find_asset_by_name("myblob").is_some());

    for p in [&ben, &relabel, &custom, &out] {
        let _ = std::fs::remove_file(p);
    }
}

#[test]
fn run_inspect_xben_format_and_checksum_flag() {
    use crate::io::bundle::format::ASSET_TYPE_CUSTOM;
    use crate::io::bundle::AddAssetOptions;

    // Every library-written asset carries ASSET_FLAG_CHECKSUM, so any add_asset call exercises the
    // checksum flag_parts branch in `run_inspect`.
    let mut buf: Vec<u8> = Vec::new();
    let mut writer = BendlWriter::new(Cursor::new(&mut buf), AssignmentFormat::Xben).unwrap();
    writer
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "checksummed",
            b"data",
            AddAssetOptions::defaults().raw(),
        )
        .unwrap();
    let mut session = writer.into_stream_session().unwrap();
    session.write_all(b"STANDARD BEN FILE\x00fake").unwrap();
    let writer = session.finish_into_writer(1);
    writer.finish().unwrap();
    let path = unique_path("inspect_xben.bendl");
    std::fs::write(&path, &buf).unwrap();

    run_inspect(InspectArgs {
        input: path.clone(),
    })
    .unwrap();
    let _ = std::fs::remove_file(&path);
}

#[test]
fn run_append_no_assets_is_noop() {
    let bendl = write_temp_bendl("append_noop.bendl", AssignmentFormat::Ben);
    let args = AppendArgs {
        input: bendl.clone(),
        graph: None,
        metadata: None,
        node_permutation_map: None,
        assets: vec![],
        graph_raw: false,
        asset_compression_level: None,
    };
    run_append(args).unwrap();
    // File should be unchanged (bundle is still valid).
    let reader = BendlReader::open(BufReader::new(std::fs::File::open(&bendl).unwrap())).unwrap();
    assert!(reader.is_finalized());
    let _ = std::fs::remove_file(&bendl);
}

#[test]
fn run_append_with_metadata_and_relabel_map() {
    let bendl = write_temp_bendl("append_assets.bendl", AssignmentFormat::Ben);
    let meta = unique_path("append_meta.json");
    std::fs::write(&meta, b"{\"version\":1}").unwrap();
    let relabel = unique_path("append_relabel.json");
    std::fs::write(&relabel, b"{\"0\":1}").unwrap();

    let args = AppendArgs {
        input: bendl.clone(),
        graph: None,
        metadata: Some(meta.clone()),
        node_permutation_map: Some(relabel.clone()),
        assets: vec![],
        graph_raw: false,
        asset_compression_level: None,
    };
    run_append(args).unwrap();

    let reader = BendlReader::open(BufReader::new(std::fs::File::open(&bendl).unwrap())).unwrap();
    assert!(reader.find_asset_by_name("metadata.json").is_some());
    assert!(reader
        .find_asset_by_name("node_permutation_map.json")
        .is_some());

    for p in [&bendl, &meta, &relabel] {
        let _ = std::fs::remove_file(p);
    }
}

#[test]
fn run_create_with_graph_raw_flag() {
    let ben = {
        let p = std::env::temp_dir().join(format!(
            "bendl-create-raw-{}.ben",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let jsonl = b"{\"assignment\":[1,2],\"sample\":1}\n";
        let mut b = Vec::new();
        encode_jsonl_to_ben(Cursor::new(jsonl), &mut b, crate::BenVariant::Standard).unwrap();
        std::fs::write(&p, &b).unwrap();
        p
    };
    let graph = unique_path("create_raw_graph.json");
    std::fs::write(&graph, b"{\"nodes\":[0,1]}").unwrap();
    let out = unique_path("create_raw.bendl");

    let args = CreateArgs {
        input: ben.clone(),
        output: out.clone(),
        graph: Some(graph.clone()),
        metadata: None,
        node_permutation_map: None,
        assets: vec![],
        overwrite: false,
        graph_raw: true,
        asset_compression_level: None,
    };
    run_create(args).unwrap();

    let reader = BendlReader::open(BufReader::new(std::fs::File::open(&out).unwrap())).unwrap();
    assert!(reader.find_asset_by_name("graph.json").is_some());

    for p in [&ben, &graph, &out] {
        let _ = std::fs::remove_file(p);
    }
}

#[test]
fn run_inspect_unknown_format_and_no_sample_count() {
    use crate::io::bundle::format::{
        BENDL_MAGIC, BENDL_MAJOR_VERSION, BENDL_MINOR_VERSION, FINALIZED_NO, HEADER_SIZE,
    };

    // Build a header with an unknown assignment format byte and finalized=0 so sample_count()
    // returns None.
    let mut header = [0u8; HEADER_SIZE];
    header[0..8].copy_from_slice(&BENDL_MAGIC);
    header[8..10].copy_from_slice(&BENDL_MAJOR_VERSION.to_le_bytes());
    header[10..12].copy_from_slice(&BENDL_MINOR_VERSION.to_le_bytes());
    header[12] = FINALIZED_NO;
    header[13] = 0xFF; // unknown format byte
                       // stream_offset = HEADER_SIZE, stream_len = 0, sample_count = -1
    let stream_offset = HEADER_SIZE as u64;
    header[40..48].copy_from_slice(&stream_offset.to_le_bytes());
    let sample_count: i64 = -1;
    header[56..64].copy_from_slice(&sample_count.to_le_bytes());

    let path = unique_path("inspect_unknown.bendl");
    std::fs::write(&path, header).unwrap();
    run_inspect(InspectArgs {
        input: path.clone(),
    })
    .unwrap();
    let _ = std::fs::remove_file(&path);
}

#[test]
fn run_append_with_graph_raw_and_graph_asset() {
    let bendl = write_temp_bendl("append_graph_raw.bendl", AssignmentFormat::Ben);
    let graph = unique_path("append_graph_raw.json");
    std::fs::write(&graph, b"{\"nodes\":[0,1,2]}").unwrap();

    let args = AppendArgs {
        input: bendl.clone(),
        graph: Some(graph.clone()),
        metadata: None,
        node_permutation_map: None,
        assets: vec![],
        graph_raw: true,
        asset_compression_level: None,
    };
    run_append(args).unwrap();

    let reader = BendlReader::open(BufReader::new(std::fs::File::open(&bendl).unwrap())).unwrap();
    assert!(reader.find_asset_by_name("graph.json").is_some());

    for p in [&bendl, &graph] {
        let _ = std::fs::remove_file(p);
    }
}

#[test]
fn run_extract_rejects_missing_stream_and_asset() {
    let args = ExtractArgs::try_parse_from(["extract", "--output", "/tmp/out.bin", "bundle.bendl"])
        .unwrap();
    let err = run_extract(args).unwrap_err();
    assert!(err.contains("either --stream or --asset"));
}

#[test]
fn run_create_errors_on_missing_metadata_file() {
    let ben = {
        let p = std::env::temp_dir().join(format!(
            "bendl-err-meta-{}.ben",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let jsonl = b"{\"assignment\":[1],\"sample\":1}\n";
        let mut b = Vec::new();
        encode_jsonl_to_ben(Cursor::new(jsonl), &mut b, crate::BenVariant::Standard).unwrap();
        std::fs::write(&p, &b).unwrap();
        p
    };
    let out = unique_path("err_meta.bendl");
    let args = CreateArgs {
        input: ben.clone(),
        output: out.clone(),
        graph: None,
        metadata: Some(unique_path("nonexistent_meta.json")),
        node_permutation_map: None,
        assets: vec![],
        overwrite: false,
        graph_raw: false,
        asset_compression_level: None,
    };
    let err = run_create(args).unwrap_err();
    assert!(err.contains("failed to read"));
    let _ = std::fs::remove_file(&ben);
    let _ = std::fs::remove_file(&out);
}

#[test]
fn run_create_errors_on_missing_relabel_map_file() {
    let ben = {
        let p = std::env::temp_dir().join(format!(
            "bendl-err-relabel-{}.ben",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut b = Vec::new();
        encode_jsonl_to_ben(
            Cursor::new(b"{\"assignment\":[1],\"sample\":1}\n"),
            &mut b,
            crate::BenVariant::Standard,
        )
        .unwrap();
        std::fs::write(&p, &b).unwrap();
        p
    };
    let out = unique_path("err_relabel.bendl");
    let args = CreateArgs {
        input: ben.clone(),
        output: out.clone(),
        graph: None,
        metadata: None,
        node_permutation_map: Some(unique_path("nonexistent_relabel.json")),
        assets: vec![],
        overwrite: false,
        graph_raw: false,
        asset_compression_level: None,
    };
    let err = run_create(args).unwrap_err();
    assert!(err.contains("failed to read"));
    let _ = std::fs::remove_file(&ben);
    let _ = std::fs::remove_file(&out);
}

#[test]
fn run_create_errors_on_missing_custom_asset_file() {
    let ben = {
        let p = std::env::temp_dir().join(format!(
            "bendl-err-custom-{}.ben",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut b = Vec::new();
        encode_jsonl_to_ben(
            Cursor::new(b"{\"assignment\":[1],\"sample\":1}\n"),
            &mut b,
            crate::BenVariant::Standard,
        )
        .unwrap();
        std::fs::write(&p, &b).unwrap();
        p
    };
    let out = unique_path("err_custom.bendl");
    let nonexistent: PathBuf = unique_path("nonexistent.bin");
    let asset_str = format!("myasset={}", nonexistent.display());
    let args = CreateArgs {
        input: ben.clone(),
        output: out.clone(),
        graph: None,
        metadata: None,
        node_permutation_map: None,
        assets: vec![asset_str.parse().unwrap()],
        overwrite: false,
        graph_raw: false,
        asset_compression_level: None,
    };
    let err = run_create(args).unwrap_err();
    assert!(err.contains("failed to read"));
    let _ = std::fs::remove_file(&ben);
    let _ = std::fs::remove_file(&out);
}

#[test]
fn run_extract_asset_by_name() {
    use crate::io::bundle::format::ASSET_TYPE_CUSTOM;
    use crate::io::bundle::AddAssetOptions;

    // Build a bundle with a named asset then extract it.
    let mut buf: Vec<u8> = Vec::new();
    let mut writer = BendlWriter::new(Cursor::new(&mut buf), AssignmentFormat::Ben).unwrap();
    writer
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "hello.txt",
            b"world",
            AddAssetOptions::defaults(),
        )
        .unwrap();
    let mut session = writer.into_stream_session().unwrap();
    session.write_all(b"STANDARD BEN FILE\x00fake").unwrap();
    let writer = session.finish_into_writer(1);
    writer.finish().unwrap();
    let bendl = unique_path("extract_asset.bendl");
    std::fs::write(&bendl, &buf).unwrap();

    let out = unique_path("extract_asset_out.txt");
    let args = ExtractArgs::try_parse_from([
        "extract",
        "--asset",
        "hello.txt",
        "--output",
        out.to_str().unwrap(),
        bendl.to_str().unwrap(),
    ])
    .unwrap();
    run_extract(args).unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), b"world");

    let _ = std::fs::remove_file(&bendl);
    let _ = std::fs::remove_file(&out);
}

#[test]
fn run_append_errors_on_missing_metadata_file() {
    let bendl = write_temp_bendl("append_err_meta.bendl", AssignmentFormat::Ben);
    let args = AppendArgs {
        input: bendl.clone(),
        graph: None,
        metadata: Some(unique_path("nonexistent_meta.json")),
        node_permutation_map: None,
        assets: vec![],
        graph_raw: false,
        asset_compression_level: None,
    };
    let err = run_append(args).unwrap_err();
    assert!(err.contains("failed to read"));
    let _ = std::fs::remove_file(&bendl);
}

#[test]
fn run_append_errors_on_missing_relabel_map_file() {
    let bendl = write_temp_bendl("append_err_relabel.bendl", AssignmentFormat::Ben);
    let args = AppendArgs {
        input: bendl.clone(),
        graph: None,
        metadata: None,
        node_permutation_map: Some(unique_path("nonexistent_relabel.json")),
        assets: vec![],
        graph_raw: false,
        asset_compression_level: None,
    };
    let err = run_append(args).unwrap_err();
    assert!(err.contains("failed to read"));
    let _ = std::fs::remove_file(&bendl);
}

#[test]
fn run_append_errors_on_missing_custom_asset_file() {
    let bendl = write_temp_bendl("append_err_custom.bendl", AssignmentFormat::Ben);
    let nonexistent = unique_path("nonexistent_custom.bin");
    let asset_str = format!("myasset={}", nonexistent.display());
    let args = AppendArgs {
        input: bendl.clone(),
        graph: None,
        metadata: None,
        node_permutation_map: None,
        assets: vec![asset_str.parse().unwrap()],
        graph_raw: false,
        asset_compression_level: None,
    };
    let err = run_append(args).unwrap_err();
    assert!(err.contains("failed to read"));
    let _ = std::fs::remove_file(&bendl);
}

// =====================================================================
// extract --stream + inspect display branches
// =====================================================================

#[test]
fn run_extract_stream_writes_raw_assignment_bytes() {
    // The existing run_extract_asset_by_name test covers --asset; this companion exercises
    // --stream (lines 27-31 of extract.rs).
    let known_stream = b"STANDARD BEN FILE\x00\x01known stream bytes";
    let bendl = unique_path("extract_stream.bendl");
    let buf = sample_bendl_bytes(known_stream, AssignmentFormat::Ben);
    std::fs::write(&bendl, &buf).unwrap();

    let out = unique_path("extract_stream_out.bin");
    let args = ExtractArgs::try_parse_from([
        "extract",
        "--stream",
        "--output",
        out.to_str().unwrap(),
        bendl.to_str().unwrap(),
    ])
    .unwrap();
    run_extract(args).unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), known_stream);

    let _ = std::fs::remove_file(&bendl);
    let _ = std::fs::remove_file(&out);
}

#[test]
fn run_extract_stream_allows_unfinalized_when_requested() {
    use crate::io::bundle::format::{AssignmentFormat, BendlHeader, FINALIZED_NO, HEADER_SIZE};

    let known_stream = b"STANDARD BEN FILE\x00partial stream bytes";
    let header = BendlHeader {
        magic: crate::io::bundle::format::BENDL_MAGIC,
        major_version: crate::io::bundle::format::BENDL_MAJOR_VERSION,
        minor_version: crate::io::bundle::format::BENDL_MINOR_VERSION,
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
    let mut buf = Vec::from(header.to_bytes());
    buf.extend_from_slice(known_stream);

    let bendl = unique_path("extract_unfinalized_stream.bendl");
    std::fs::write(&bendl, &buf).unwrap();
    let out = unique_path("extract_unfinalized_stream_out.bin");

    let default_args = ExtractArgs::try_parse_from([
        "extract",
        "--stream",
        "--output",
        out.to_str().unwrap(),
        bendl.to_str().unwrap(),
    ])
    .unwrap();
    let err = run_extract(default_args).unwrap_err();
    assert!(err.contains("unfinalized"), "unexpected error: {err}");
    assert!(!out.exists(), "failed extraction must not create output");

    let allow_args = ExtractArgs::try_parse_from([
        "extract",
        "--stream",
        "--allow-unfinalized",
        "--output",
        out.to_str().unwrap(),
        bendl.to_str().unwrap(),
    ])
    .unwrap();
    run_extract(allow_args).unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), known_stream);

    let _ = std::fs::remove_file(&bendl);
    let _ = std::fs::remove_file(&out);
}

#[test]
fn run_extract_asset_with_unknown_name_errors_cleanly() {
    // Pin the no-asset-named-X branch of extract.rs; find_asset_by_name returns None and the
    // caller surfaces a clear "no asset named ..." error.
    let bendl = write_temp_bendl("extract_unknown_asset.bendl", AssignmentFormat::Ben);
    let out = unique_path("extract_unknown_out.bin");
    let args = ExtractArgs::try_parse_from([
        "extract",
        "--asset",
        "does-not-exist.txt",
        "--output",
        out.to_str().unwrap(),
        bendl.to_str().unwrap(),
    ])
    .unwrap();
    let err = run_extract(args).unwrap_err();
    assert!(
        err.contains("no asset") && err.contains("does-not-exist"),
        "expected no-asset error mentioning the name, got: {err}"
    );
    let _ = std::fs::remove_file(&bendl);
    let _ = std::fs::remove_file(&out);
}

#[test]
fn run_inspect_displays_asset_with_no_flags_as_dash() {
    // Pin inspect.rs line 60: the `"-".to_string()` fallback for an asset whose asset_flags
    // bitmap has no known bits set. Reaching it requires hand-building a directory entry with
    // asset_flags=0 (the library writer always sets ASSET_FLAG_CHECKSUM).
    use crate::io::bundle::format::{
        encode_directory, BendlDirectoryEntry, BendlHeader, ASSET_TYPE_CUSTOM, BENDL_MAGIC,
        BENDL_MAJOR_VERSION, BENDL_MINOR_VERSION, FINALIZED_YES, HEADER_FLAG_STREAM_CHECKSUM,
        HEADER_SIZE,
    };

    let payload = b"raw bytes";
    let mut bytes = vec![0u8; HEADER_SIZE];
    let payload_offset = bytes.len() as u64;
    bytes.extend_from_slice(payload);

    let directory_offset = bytes.len() as u64;
    let entries = vec![BendlDirectoryEntry {
        asset_type: ASSET_TYPE_CUSTOM,
        asset_flags: 0,
        name: "flagless.bin".to_string(),
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

    let bendl = unique_path("inspect_flagless.bendl");
    std::fs::write(&bendl, &bytes).unwrap();
    let args = InspectArgs::try_parse_from(["inspect", bendl.to_str().unwrap()]).unwrap();
    run_inspect(args).unwrap();
    let _ = std::fs::remove_file(&bendl);
}

#[test]
fn run_create_rejects_non_json_metadata_file() {
    // --metadata stamps the JSON flag onto the file's bytes; a plain-text file used to be
    // accepted silently and only blow up weeks later in the consumer's read_metadata(). The
    // write must refuse instead.
    let ben = {
        let p = std::env::temp_dir().join(format!(
            "bendl-badjson-{}.ben",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut b = Vec::new();
        encode_jsonl_to_ben(
            Cursor::new(b"{\"assignment\":[1],\"sample\":1}\n"),
            &mut b,
            crate::BenVariant::Standard,
        )
        .unwrap();
        std::fs::write(&p, &b).unwrap();
        p
    };
    let notes = unique_path("notes.txt");
    std::fs::write(&notes, b"these are plain-text notes, not JSON").unwrap();
    let out = unique_path("badjson.bendl");
    let args = CreateArgs {
        input: ben.clone(),
        output: out.clone(),
        graph: None,
        metadata: Some(notes.clone()),
        node_permutation_map: None,
        assets: vec![],
        overwrite: false,
        graph_raw: false,
        asset_compression_level: None,
    };
    let err = run_create(args).unwrap_err();
    assert!(err.contains("not valid JSON"), "unexpected error: {err}");
    let _ = std::fs::remove_file(&ben);
    let _ = std::fs::remove_file(&notes);
    let _ = std::fs::remove_file(&out);
}

#[test]
fn run_create_accepts_asset_compression_level() {
    let ben = {
        let p = std::env::temp_dir().join(format!(
            "bendl-create-level-{}.ben",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let jsonl = b"{\"assignment\":[1,2],\"sample\":1}\n";
        let mut b = Vec::new();
        encode_jsonl_to_ben(Cursor::new(jsonl), &mut b, crate::BenVariant::Standard).unwrap();
        std::fs::write(&p, &b).unwrap();
        p
    };
    let graph = unique_path("create_level_graph.json");
    std::fs::write(&graph, b"{\"nodes\":[0,1]}").unwrap();
    let out = unique_path("create_level.bendl");

    let args = CreateArgs {
        input: ben.clone(),
        output: out.clone(),
        graph: Some(graph.clone()),
        metadata: None,
        node_permutation_map: None,
        assets: vec![],
        overwrite: false,
        graph_raw: false,
        asset_compression_level: Some(0),
    };
    run_create(args).unwrap();

    let mut reader = BendlReader::open(BufReader::new(std::fs::File::open(&out).unwrap())).unwrap();
    let entry = reader.find_asset_by_name("graph.json").cloned().unwrap();
    assert_eq!(reader.asset_bytes(&entry).unwrap(), b"{\"nodes\":[0,1]}");
    for p in [&ben, &graph, &out] {
        let _ = std::fs::remove_file(p);
    }
}
