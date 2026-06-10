use super::args::resolve_variant;
use super::args::{Args, CliVariant, Mode};
use super::bundle::{
    append_graph_asset, run_encode_bundle_with_graph, run_xencode_bundle_with_graph,
};
use super::paths::{
    count_jsonl_lines, decode_setup, encode_setup, open_derived_writer, open_reader, open_writer,
};
use crate::test_utils::unique_path;
use crate::BenVariant;
use clap::{CommandFactory, Parser};
use std::fs;
use std::io::{self, Write};

#[test]
fn clap_metadata_uses_package_version() {
    let mut command = Args::command();
    let help = command.render_long_help().to_string();

    assert_eq!(command.get_version(), Some(env!("CARGO_PKG_VERSION")));
    assert!(help.contains("Binary Ensemble CLI Tool"));
    assert!(help.contains("--mode"));
    assert!(help.contains("x-encode"));
}

#[test]
fn parse_encode_args() {
    let args = Args::try_parse_from([
        "ben",
        "--mode",
        "encode",
        "--output-file",
        "out.ben",
        "--save-all",
        "--verbose",
        "input.jsonl",
    ])
    .unwrap();

    assert_eq!(args.mode, Mode::Encode);
    assert_eq!(args.input_file.as_deref(), Some("input.jsonl"));
    assert_eq!(args.output_file.as_deref(), Some("out.ben"));
    assert!(args.save_all);
    assert!(args.verbose);
}

#[test]
fn parse_variant_flag() {
    let args = Args::try_parse_from([
        "ben",
        "--mode",
        "encode",
        "--variant",
        "twodelta",
        "input.jsonl",
    ])
    .unwrap();

    assert_eq!(args.variant, Some(CliVariant::Twodelta));
}

#[test]
fn parse_variant_aliases() {
    let args = Args::try_parse_from([
        "ben",
        "--mode",
        "encode",
        "--variant",
        "mkv_chain",
        "input.jsonl",
    ])
    .unwrap();
    assert_eq!(args.variant, Some(CliVariant::Mkvchain));

    let args = Args::try_parse_from([
        "ben",
        "--mode",
        "encode",
        "--variant",
        "two_delta",
        "input.jsonl",
    ])
    .unwrap();
    assert_eq!(args.variant, Some(CliVariant::Twodelta));
}

#[test]
fn resolve_variant_precedence() {
    // --variant takes precedence over --save-all
    assert_eq!(
        resolve_variant(Some(CliVariant::Twodelta), true),
        BenVariant::TwoDelta
    );
    assert_eq!(
        resolve_variant(Some(CliVariant::Mkvchain), true),
        BenVariant::MkvChain
    );
    // --save-all alone means Standard
    assert_eq!(resolve_variant(None, true), BenVariant::Standard);
    // neither means MkvChain
    assert_eq!(resolve_variant(None, false), BenVariant::MkvChain);
}

#[test]
fn parse_xencode_stream_flags() {
    let args = Args::try_parse_from([
        "ben",
        "--mode",
        "x-encode",
        "--jsonl-and-xben",
        "--ben-and-xben",
        "--jsonl-and-ben",
    ])
    .unwrap();

    assert_eq!(args.mode, Mode::XEncode);
    assert!(args.jsonl_and_xben);
    assert!(args.ben_and_xben);
    assert!(args.jsonl_and_ben);
}

#[test]
fn encode_setup_derives_extensions() {
    assert_eq!(
        encode_setup(Mode::Encode, "samples.jsonl".to_string(), None, true, false).unwrap(),
        "samples.jsonl.ben"
    );
    assert_eq!(
        encode_setup(Mode::XEncode, "samples.ben".to_string(), None, true, false).unwrap(),
        "samples.xben"
    );
    assert_eq!(
        encode_setup(
            Mode::XzCompress,
            "samples.jsonl".to_string(),
            None,
            true,
            false
        )
        .unwrap(),
        "samples.jsonl.xz"
    );
}

#[test]
fn encode_setup_with_graph_derives_bendl_extension() {
    // JSONL + encode + graph → .bendl
    assert_eq!(
        encode_setup(Mode::Encode, "samples.jsonl".to_string(), None, true, true).unwrap(),
        "samples.jsonl.bendl"
    );
    // .ben input to x-encode with graph trims the .ben suffix
    assert_eq!(
        encode_setup(Mode::XEncode, "samples.ben".to_string(), None, true, true).unwrap(),
        "samples.bendl"
    );
    // .xben input to x-encode with graph trims the .xben suffix
    assert_eq!(
        encode_setup(Mode::XEncode, "samples.xben".to_string(), None, true, true).unwrap(),
        "samples.bendl"
    );
}

#[test]
fn encode_setup_respects_explicit_output() {
    assert_eq!(
        encode_setup(
            Mode::Encode,
            "ignored.jsonl".to_string(),
            Some("custom-output.ben".to_string()),
            true,
            false,
        )
        .unwrap(),
        "custom-output.ben"
    );
}

#[test]
fn encode_setup_checks_overwrite() {
    let path = unique_path("existing.ben");
    fs::write(&path, "already here").unwrap();

    let err = encode_setup(
        Mode::Encode,
        "input.jsonl".to_string(),
        Some(path.to_string_lossy().into_owned()),
        true,
        false,
    );
    assert!(err.is_ok());

    fs::remove_file(path).unwrap();
}

#[test]
fn decode_setup_derives_ben_and_xben_outputs() {
    assert_eq!(
        decode_setup("samples.ben".to_string(), None, false, true).unwrap(),
        "samples"
    );
    assert_eq!(
        decode_setup("samples.xben".to_string(), None, false, true).unwrap(),
        "samples.ben"
    );
    assert_eq!(
        decode_setup("samples.xben".to_string(), None, true, true).unwrap(),
        "samples"
    );
}

#[test]
fn decode_setup_rejects_xz_input() {
    let err = decode_setup("samples.xz".to_string(), None, false, true).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
}

#[test]
fn decode_setup_rejects_unknown_input() {
    let err = decode_setup("samples.data".to_string(), None, false, true).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
}

#[test]
fn decode_setup_respects_explicit_output() {
    assert_eq!(
        decode_setup(
            "samples.xben".to_string(),
            Some("custom.jsonl".to_string()),
            true,
            true,
        )
        .unwrap(),
        "custom.jsonl"
    );
}

#[test]
fn open_reader_reads_file_contents() {
    let path = unique_path("reader.txt");
    fs::write(&path, "hello\nworld\n").unwrap();

    let mut reader = open_reader(Some(path.to_str().unwrap())).unwrap();
    let mut content = String::new();
    std::io::Read::read_to_string(&mut reader, &mut content).unwrap();

    assert_eq!(content, "hello\nworld\n");
    fs::remove_file(path).unwrap();
}

#[test]
fn open_reader_accepts_stdin() {
    let _reader = open_reader(None).unwrap();
}

#[test]
fn open_reader_missing_file_errors_instead_of_panicking() {
    let err = match open_reader(Some("/nonexistent/definitely-missing.jsonl")) {
        Ok(_) => panic!("expected open_reader to fail for a missing file"),
        Err(e) => e,
    };
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    assert!(err.to_string().contains("definitely-missing"));
}

#[test]
fn open_writer_creates_file_and_writes() {
    let path = unique_path("writer.txt");
    {
        let mut writer = open_writer(Some(path.to_str().unwrap()), false, true).unwrap();
        writer.write_all(b"written").unwrap();
    }

    assert_eq!(fs::read_to_string(&path).unwrap(), "written");
    fs::remove_file(path).unwrap();
}

#[test]
fn open_writer_supports_stdout_and_print() {
    let mut stdout_writer = open_writer(None, false, true).unwrap();
    stdout_writer.write_all(b"").unwrap();

    let mut print_writer = open_writer(Some("ignored.txt"), true, false).unwrap();
    print_writer.write_all(b"").unwrap();
}

#[test]
fn open_derived_writer_creates_file() {
    let path = unique_path("derived.txt");
    {
        let mut writer = open_derived_writer(path.to_string_lossy().into_owned()).unwrap();
        writer.write_all(b"derived").unwrap();
    }

    assert_eq!(fs::read_to_string(&path).unwrap(), "derived");
    fs::remove_file(path).unwrap();
}

#[test]
fn resolve_variant_standard_arm() {
    assert_eq!(
        resolve_variant(Some(CliVariant::Standard), false),
        BenVariant::Standard
    );
}

#[test]
fn count_jsonl_lines_counts_nonempty_lines() {
    let path = unique_path("count.jsonl");
    fs::write(&path, b"{\"a\":1}\n\n{\"b\":2}\n").unwrap();
    let count = count_jsonl_lines(&path).unwrap();
    assert_eq!(count, 2);
    fs::remove_file(path).unwrap();
}

/// Write a two-sample Standard BEN JSONL file to a temp path.
fn write_temp_jsonl(name: &str) -> std::path::PathBuf {
    let path = unique_path(name);
    fs::write(
        &path,
        b"{\"assignment\":[1,2,3],\"sample\":1}\n{\"assignment\":[2,1,3],\"sample\":2}\n",
    )
    .unwrap();
    path
}

/// Write a minimal graph JSON file to a temp path.
fn write_temp_graph(name: &str) -> std::path::PathBuf {
    let path = unique_path(name);
    fs::write(&path, b"{\"nodes\":[0,1,2],\"adj\":[[1],[0,2],[1]]}").unwrap();
    path
}

#[test]
fn append_graph_asset_adds_graph_to_bundle() {
    use crate::io::bundle::format::AssignmentFormat;
    use crate::io::bundle::{BendlReader, BendlWriter};
    use std::io::Cursor;

    // Build a minimal finalized .bendl in memory, write to temp file.
    let mut buf: Vec<u8> = Vec::new();
    {
        let writer = BendlWriter::new(Cursor::new(&mut buf), AssignmentFormat::Ben).unwrap();
        let mut session = writer.into_stream_session().unwrap();
        session.write_all(b"STANDARD BEN FILE\x00fake").unwrap();
        let writer = session.finish_into_writer(1);
        writer.finish().unwrap();
    }
    let bendl_path = unique_path("append_graph.bendl");
    fs::write(&bendl_path, &buf).unwrap();

    let graph_path = write_temp_graph("append_graph.json");

    append_graph_asset(bendl_path.to_str().unwrap(), &graph_path).unwrap();

    // Verify the graph asset was added.
    let file = fs::File::open(&bendl_path).unwrap();
    let reader = BendlReader::open(std::io::BufReader::new(file)).unwrap();
    assert!(reader.find_asset_by_name("graph.json").is_some());

    fs::remove_file(&bendl_path).unwrap();
    fs::remove_file(&graph_path).unwrap();
}

#[test]
fn run_encode_bundle_with_graph_creates_bendl() {
    use crate::io::bundle::BendlReader;

    let jsonl = write_temp_jsonl("enc_graph_input.jsonl");
    let graph = write_temp_graph("enc_graph.json");
    let out = unique_path("enc_graph_output.bendl");

    run_encode_bundle_with_graph(&jsonl, out.to_str().unwrap(), BenVariant::Standard, &graph)
        .unwrap();

    let file = fs::File::open(&out).unwrap();
    let reader = BendlReader::open(std::io::BufReader::new(file)).unwrap();
    assert!(reader.is_finalized());
    assert!(reader.find_asset_by_name("graph.json").is_some());
    assert_eq!(reader.sample_count(), Some(2));

    fs::remove_file(&jsonl).unwrap();
    fs::remove_file(&graph).unwrap();
    fs::remove_file(&out).unwrap();
}

#[test]
fn run_xencode_bundle_with_graph_from_jsonl_creates_bendl() {
    use crate::io::bundle::BendlReader;

    let jsonl = write_temp_jsonl("xencode_graph_input.jsonl");
    let graph = write_temp_graph("xencode_graph.json");
    let out = unique_path("xencode_graph_output.bendl");

    run_xencode_bundle_with_graph(
        &jsonl,
        out.to_str().unwrap(),
        BenVariant::Standard,
        false,
        None,
        None,
        None,
        None,
        &graph,
    )
    .unwrap();

    let file = fs::File::open(&out).unwrap();
    let reader = BendlReader::open(std::io::BufReader::new(file)).unwrap();
    assert!(reader.is_finalized());
    assert!(reader.find_asset_by_name("graph.json").is_some());

    fs::remove_file(&jsonl).unwrap();
    fs::remove_file(&graph).unwrap();
    fs::remove_file(&out).unwrap();
}

#[test]
fn run_xencode_bundle_with_graph_from_ben_creates_bendl() {
    use crate::codec::encode::encode_jsonl_to_ben;
    use crate::io::bundle::BendlReader;
    use std::io::Cursor;

    // First create a BEN file from JSONL.
    let jsonl = b"{\"assignment\":[1,2],\"sample\":1}\n{\"assignment\":[2,1],\"sample\":2}\n";
    let mut ben_bytes = Vec::new();
    encode_jsonl_to_ben(Cursor::new(jsonl), &mut ben_bytes, BenVariant::Standard).unwrap();
    let ben_path = unique_path("xencode_from_ben_input.ben");
    fs::write(&ben_path, &ben_bytes).unwrap();

    let graph = write_temp_graph("xencode_from_ben_graph.json");
    let out = unique_path("xencode_from_ben_output.bendl");

    run_xencode_bundle_with_graph(
        &ben_path,
        out.to_str().unwrap(),
        BenVariant::Standard,
        true,
        None,
        None,
        None,
        None,
        &graph,
    )
    .unwrap();

    let file = fs::File::open(&out).unwrap();
    let reader = BendlReader::open(std::io::BufReader::new(file)).unwrap();
    assert!(reader.is_finalized());
    assert!(reader.find_asset_by_name("graph.json").is_some());

    fs::remove_file(&ben_path).unwrap();
    fs::remove_file(&graph).unwrap();
    fs::remove_file(&out).unwrap();
}

#[test]
fn append_graph_asset_errors_on_missing_graph_file() {
    use crate::io::bundle::format::AssignmentFormat;
    use crate::io::bundle::BendlWriter;
    use std::io::Cursor;

    let mut buf: Vec<u8> = Vec::new();
    {
        let writer = BendlWriter::new(Cursor::new(&mut buf), AssignmentFormat::Ben).unwrap();
        let mut session = writer.into_stream_session().unwrap();
        session.write_all(b"STANDARD BEN FILE\x00fake").unwrap();
        let writer = session.finish_into_writer(1);
        writer.finish().unwrap();
    }
    let bendl_path = unique_path("err_graph.bendl");
    fs::write(&bendl_path, &buf).unwrap();

    let nonexistent = unique_path("nonexistent.json");
    let err = append_graph_asset(bendl_path.to_str().unwrap(), &nonexistent).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::Other);
    assert!(err.to_string().contains("failed to read graph"));
    let _ = fs::remove_file(&bendl_path);
}

#[test]
fn run_encode_bundle_with_graph_errors_on_missing_graph() {
    let jsonl = write_temp_jsonl("err_enc_input.jsonl");
    let out = unique_path("err_enc_output.bendl");
    let nonexistent = unique_path("nonexistent.json");

    let err = run_encode_bundle_with_graph(
        &jsonl,
        out.to_str().unwrap(),
        BenVariant::Standard,
        &nonexistent,
    )
    .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::Other);
    assert!(err.to_string().contains("failed to stat graph"));
    let _ = fs::remove_file(&jsonl);
    let _ = fs::remove_file(&out);
}

#[test]
fn run_xencode_bundle_with_graph_errors_on_missing_graph() {
    let jsonl = write_temp_jsonl("err_xenc_input.jsonl");
    let out = unique_path("err_xenc_output.bendl");
    let nonexistent = unique_path("nonexistent.json");

    let err = run_xencode_bundle_with_graph(
        &jsonl,
        out.to_str().unwrap(),
        BenVariant::Standard,
        false,
        None,
        None,
        None,
        None,
        &nonexistent,
    )
    .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::Other);
    assert!(err.to_string().contains("failed to stat graph"));
    let _ = fs::remove_file(&jsonl);
    let _ = fs::remove_file(&out);
}

#[test]
fn append_graph_asset_errors_when_bundle_already_has_graph() {
    use crate::io::bundle::format::{AssignmentFormat, ASSET_TYPE_GRAPH};
    use crate::io::bundle::{AddAssetOptions, BendlWriter};
    use std::io::Cursor;

    // Build a .bendl that already contains graph.json.
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut writer = BendlWriter::new(Cursor::new(&mut buf), AssignmentFormat::Ben).unwrap();
        writer
            .add_asset(
                ASSET_TYPE_GRAPH,
                "graph.json",
                b"{}",
                AddAssetOptions::defaults().json(),
            )
            .unwrap();
        let mut session = writer.into_stream_session().unwrap();
        session.write_all(b"STANDARD BEN FILE\x00fake").unwrap();
        let writer = session.finish_into_writer(1);
        writer.finish().unwrap();
    }
    let bendl_path = unique_path("dup_graph.bendl");
    fs::write(&bendl_path, &buf).unwrap();

    // graph.json already exists — add_asset must fail with duplicate name.
    let graph_path = write_temp_graph("dup_graph.json");
    let err = append_graph_asset(bendl_path.to_str().unwrap(), &graph_path).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::Other);
    assert!(err.to_string().contains("failed to add graph asset"));

    let _ = fs::remove_file(&bendl_path);
    let _ = fs::remove_file(&graph_path);
}

#[test]
fn run_xencode_bundle_with_graph_errors_on_invalid_jsonl() {
    // from_ben=false path: encode_jsonl_to_xben fails on invalid JSONL.
    let bad_jsonl = unique_path("bad.jsonl");
    fs::write(&bad_jsonl, b"not valid json\n").unwrap();
    let graph = write_temp_graph("xenc_bad_jsonl_graph.json");
    let out = unique_path("xenc_bad_jsonl.bendl");

    let err = run_xencode_bundle_with_graph(
        &bad_jsonl,
        out.to_str().unwrap(),
        BenVariant::Standard,
        false,
        None,
        None,
        None,
        None,
        &graph,
    )
    .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);

    let _ = fs::remove_file(&bad_jsonl);
    let _ = fs::remove_file(&graph);
    let _ = fs::remove_file(&out);
}

#[test]
fn run_xencode_bundle_with_graph_errors_on_invalid_ben() {
    // from_ben=true path: encode_ben_to_xben fails on a file with no BEN banner.
    let bad_ben = unique_path("bad.ben");
    fs::write(&bad_ben, b"this is not a ben file").unwrap();
    let graph = write_temp_graph("xenc_bad_ben_graph.json");
    let out = unique_path("xenc_bad_ben.bendl");

    let err = run_xencode_bundle_with_graph(
        &bad_ben,
        out.to_str().unwrap(),
        BenVariant::Standard,
        true,
        None,
        None,
        None,
        None,
        &graph,
    )
    .unwrap_err();
    // encode_ben_to_xben fails when it can't read a valid banner.
    assert!(err.kind() != io::ErrorKind::NotFound);

    let _ = fs::remove_file(&bad_ben);
    let _ = fs::remove_file(&graph);
    let _ = fs::remove_file(&out);
}
