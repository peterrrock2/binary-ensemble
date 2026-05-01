use super::append::run_append;
use super::args::{AppendArgs, CreateArgs, ExtractArgs, InspectArgs, NamedAsset};
use super::create::run_create;
use super::extract::run_extract;
use super::helpers::{format_from_path, mode_str};
use super::inspect::run_inspect;
use crate::codec::encode::encode_jsonl_to_ben;
use crate::io::bundle::format::AssignmentFormat;
use crate::io::bundle::{BendlReader, BendlWriter};
use crate::test_utils::{sample_bendl_bytes, unique_path};
use clap::Parser;
use std::io::{BufReader, Cursor};
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
    assert!(reader.is_complete());
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
fn mode_str_returns_xben_for_xben() {
    assert_eq!(mode_str(AssignmentFormat::Xben), "xben");
}

#[test]
fn run_create_with_relabel_map_and_custom_asset() {
    let ben = {
        // Must end in .ben so format_from_path recognises it.
        let p = std::env::temp_dir().join(format!(
            "bendl-create-relabel-{}.ben",
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
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
        relabel_map: Some(relabel.clone()),
        assets: vec![asset_str.parse().unwrap()],
        overwrite: false,
        graph_raw: false,
    };
    run_create(args).unwrap();

    let reader = BendlReader::open(BufReader::new(std::fs::File::open(&out).unwrap())).unwrap();
    assert!(reader.find_asset_by_name("relabel_map.json").is_some());
    assert!(reader.find_asset_by_name("myblob").is_some());

    for p in [&ben, &relabel, &custom, &out] {
        let _ = std::fs::remove_file(p);
    }
}

#[test]
fn run_inspect_xben_format_and_checksum_flag() {
    use crate::io::bundle::format::ASSET_TYPE_CUSTOM;
    use crate::io::bundle::AddAssetOptions;

    // Build a .bendl with a checksum asset so the flag_parts checksum
    // branch is exercised.
    let mut buf: Vec<u8> = Vec::new();
    let mut writer = BendlWriter::new(Cursor::new(&mut buf), AssignmentFormat::Xben).unwrap();
    writer
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "checksummed",
            b"data",
            AddAssetOptions {
                checksum: Some(vec![0xAB, 0xCD]),
                ..AddAssetOptions::defaults()
            },
        )
        .unwrap();
    writer.write_stream_bytes(b"STANDARD BEN FILE\x00fake", 1).unwrap();
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
        relabel_map: None,
        assets: vec![],
        graph_raw: false,
    };
    run_append(args).unwrap();
    // File should be unchanged (bundle is still valid).
    let reader =
        BendlReader::open(BufReader::new(std::fs::File::open(&bendl).unwrap())).unwrap();
    assert!(reader.is_complete());
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
        relabel_map: Some(relabel.clone()),
        assets: vec![],
        graph_raw: false,
    };
    run_append(args).unwrap();

    let reader =
        BendlReader::open(BufReader::new(std::fs::File::open(&bendl).unwrap())).unwrap();
    assert!(reader.find_asset_by_name("metadata.json").is_some());
    assert!(reader.find_asset_by_name("relabel_map.json").is_some());

    for p in [&bendl, &meta, &relabel] {
        let _ = std::fs::remove_file(p);
    }
}

#[test]
fn run_create_with_graph_raw_flag() {
    let ben = {
        let p = std::env::temp_dir().join(format!(
            "bendl-create-raw-{}.ben",
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
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
        relabel_map: None,
        assets: vec![],
        overwrite: false,
        graph_raw: true,
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
        BENDL_MAGIC, BENDL_MAJOR_VERSION, BENDL_MINOR_VERSION, COMPLETE_NO, HEADER_SIZE,
    };

    // Build a header with an unknown assignment format byte and
    // complete=0 so sample_count() returns None.
    let mut header = [0u8; HEADER_SIZE];
    header[0..8].copy_from_slice(&BENDL_MAGIC);
    header[8..10].copy_from_slice(&BENDL_MAJOR_VERSION.to_le_bytes());
    header[10..12].copy_from_slice(&BENDL_MINOR_VERSION.to_le_bytes());
    header[12] = COMPLETE_NO;
    header[13] = 0xFF; // unknown format byte
    // stream_offset = HEADER_SIZE, stream_len = 0, sample_count = -1
    let stream_offset = HEADER_SIZE as u64;
    header[40..48].copy_from_slice(&stream_offset.to_le_bytes());
    let sample_count: i64 = -1;
    header[56..64].copy_from_slice(&sample_count.to_le_bytes());

    let path = unique_path("inspect_unknown.bendl");
    std::fs::write(&path, &header).unwrap();
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
        relabel_map: None,
        assets: vec![],
        graph_raw: true,
    };
    run_append(args).unwrap();

    let reader =
        BendlReader::open(BufReader::new(std::fs::File::open(&bendl).unwrap())).unwrap();
    assert!(reader.find_asset_by_name("graph.json").is_some());

    for p in [&bendl, &graph] {
        let _ = std::fs::remove_file(p);
    }
}

#[test]
fn run_extract_rejects_missing_stream_and_asset() {
    let args = ExtractArgs::try_parse_from([
        "extract",
        "--output",
        "/tmp/out.bin",
        "bundle.bendl",
    ])
    .unwrap();
    let err = run_extract(args).unwrap_err();
    assert!(err.contains("either --stream or --asset"));
}

#[test]
fn run_create_errors_on_missing_metadata_file() {
    let ben = {
        let p = std::env::temp_dir().join(format!(
            "bendl-err-meta-{}.ben",
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
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
        relabel_map: None,
        assets: vec![],
        overwrite: false,
        graph_raw: false,
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
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
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
        relabel_map: Some(unique_path("nonexistent_relabel.json")),
        assets: vec![],
        overwrite: false,
        graph_raw: false,
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
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
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
        relabel_map: None,
        assets: vec![asset_str.parse().unwrap()],
        overwrite: false,
        graph_raw: false,
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
    writer.write_stream_bytes(b"STANDARD BEN FILE\x00fake", 1).unwrap();
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
        relabel_map: None,
        assets: vec![],
        graph_raw: false,
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
        relabel_map: Some(unique_path("nonexistent_relabel.json")),
        assets: vec![],
        graph_raw: false,
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
        relabel_map: None,
        assets: vec![asset_str.parse().unwrap()],
        graph_raw: false,
    };
    let err = run_append(args).unwrap_err();
    assert!(err.contains("failed to read"));
    let _ = std::fs::remove_file(&bendl);
}
