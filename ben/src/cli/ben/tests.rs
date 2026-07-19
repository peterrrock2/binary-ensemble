use super::args::{
    resolve_variant, CanonicalizeArgs, Cli, CliVariant, Command, Globals, LookupTarget,
    OrderingMethod, ReencodeArgs, RelabelArgs, SortGraphArgs,
};
use super::bundle::{
    append_graph_asset, run_encode_bundle_with_graph, run_xencode_bundle_with_graph,
};
use super::paths::{
    count_jsonl_lines, decode_setup, encode_setup, open_derived_writer, open_reader, open_writer,
    EncodeTarget,
};
use super::relabel_helpers::{ben_variant_name, read_node_permutation_map_file, relabeling_label};
use super::{canonicalize, reencode, relabel, sort_graph};
use crate::codec::encode::encode_jsonl_to_ben;
use crate::io::reader::BenStreamReader;
use crate::test_utils::{jsonl_from_assignments, sample_ben_bytes, unique_path};
use crate::BenVariant;
use clap::{CommandFactory, Parser};
use std::path::Path;
use std::{fs, io, io::Cursor, io::Write};

// =====================================================================
// argument parsing
// =====================================================================

#[test]
fn clap_metadata_uses_package_version() {
    let mut command = Cli::command();
    let help = command.render_long_help().to_string();

    assert_eq!(command.get_version(), Some(env!("CARGO_PKG_VERSION")));
    assert!(help.contains("Encode, decode, relabel"));
    assert!(help.contains("encode"));
    assert!(help.contains("xencode"));
    assert!(help.contains("canonicalize"));
    assert!(help.contains("reencode"));
    assert!(help.contains("pcompress"));
}

#[test]
fn parse_encode_args() {
    let cli = Cli::try_parse_from([
        "ben",
        "encode",
        "--output-file",
        "out.ben",
        "--save-all",
        "--verbose",
        "input.jsonl",
    ])
    .unwrap();

    assert_eq!(cli.globals.output_file.as_deref(), Some("out.ben"));
    assert!(cli.globals.verbose);
    match cli.command {
        Command::Encode(a) => {
            assert_eq!(a.input_file.as_deref(), Some("input.jsonl"));
            assert!(a.save_all);
        }
        other => panic!("expected encode, got {other:?}"),
    }
}

#[test]
fn global_flags_accepted_before_and_after_subcommand() {
    let before = Cli::try_parse_from(["ben", "--verbose", "encode", "input.jsonl"]).unwrap();
    assert!(before.globals.verbose);

    let after = Cli::try_parse_from(["ben", "encode", "input.jsonl", "--verbose"]).unwrap();
    assert!(after.globals.verbose);
}

#[test]
fn parse_variant_flag() {
    let cli =
        Cli::try_parse_from(["ben", "encode", "--variant", "twodelta", "input.jsonl"]).unwrap();
    match cli.command {
        Command::Encode(a) => assert_eq!(a.variant, Some(CliVariant::Twodelta)),
        other => panic!("expected encode, got {other:?}"),
    }
}

#[test]
fn parse_variant_aliases() {
    for (text, expected) in [
        ("mkv_chain", CliVariant::Mkvchain),
        ("mkv-chain", CliVariant::Mkvchain),
        ("two_delta", CliVariant::Twodelta),
        ("two-delta", CliVariant::Twodelta),
    ] {
        let cli = Cli::try_parse_from(["ben", "encode", "--variant", text, "input.jsonl"]).unwrap();
        match cli.command {
            Command::Encode(a) => assert_eq!(a.variant, Some(expected)),
            other => panic!("expected encode, got {other:?}"),
        }
    }
}

#[test]
fn parse_xencode_from_ben_flag() {
    let cli = Cli::try_parse_from(["ben", "xencode", "--from-ben", "in.ben"]).unwrap();
    match cli.command {
        Command::Xencode(a) => assert!(a.from_ben),
        other => panic!("expected xencode, got {other:?}"),
    }
}

#[test]
fn parse_decode_from_xben_flag() {
    let cli = Cli::try_parse_from(["ben", "decode", "--from-xben", "in.xben"]).unwrap();
    match cli.command {
        Command::Decode(a) => assert!(a.from_xben),
        other => panic!("expected decode, got {other:?}"),
    }
}

#[test]
fn lookup_requires_index_subcommand() {
    assert!(Cli::try_parse_from(["ben", "lookup", "x.ben"]).is_err());

    let cli = Cli::try_parse_from(["ben", "lookup", "index", "x.ben", "0"]).unwrap();
    match cli.command {
        Command::Lookup(a) => {
            let LookupTarget::Index(a) = a.target;
            assert_eq!(a.index, 0);
        }
        other => panic!("expected lookup, got {other:?}"),
    }

    assert!(Cli::try_parse_from(["ben", "lookup", "sample", "x.ben", "2"]).is_err());
}

#[test]
fn reencode_parses_with_and_without_options() {
    // The "must change something" guard is enforced at runtime, not parse time.
    assert!(Cli::try_parse_from(["ben", "reencode", "x.ben"]).is_ok());
    assert!(Cli::try_parse_from(["ben", "reencode", "x.ben", "--n-items", "5"]).is_ok());
}

#[test]
fn shape_file_is_accepted_as_hidden_alias_for_dualgraph() {
    let cli = Cli::try_parse_from([
        "ben",
        "relabel",
        "input.ben",
        "--key",
        "GEOID20",
        "--shape-file",
        "graph.json",
    ])
    .unwrap();
    match cli.command {
        Command::Relabel(a) => assert_eq!(a.dual_graph.as_deref(), Some("graph.json")),
        other => panic!("expected relabel, got {other:?}"),
    }
}

#[test]
fn unknown_subcommand_errors() {
    assert!(Cli::try_parse_from(["ben", "frobnicate", "x.ben"]).is_err());
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
    // neither means TwoDelta
    assert_eq!(resolve_variant(None, false), BenVariant::TwoDelta);
}

#[test]
fn resolve_variant_standard_arm() {
    assert_eq!(
        resolve_variant(Some(CliVariant::Standard), false),
        BenVariant::Standard
    );
}

// =====================================================================
// path derivation
// =====================================================================

#[test]
fn encode_setup_derives_extensions() {
    // A `.jsonl` source swaps its extension for the BEN-family target rather than stacking it.
    assert_eq!(
        encode_setup(
            EncodeTarget::Ben,
            "samples.jsonl".to_string(),
            None,
            true,
            false
        )
        .unwrap(),
        "samples.ben"
    );
    assert_eq!(
        encode_setup(
            EncodeTarget::Xben,
            "samples.ben".to_string(),
            None,
            true,
            false
        )
        .unwrap(),
        "samples.xben"
    );
}

#[test]
fn encode_setup_with_graph_derives_bendl_extension() {
    assert_eq!(
        encode_setup(
            EncodeTarget::Ben,
            "samples.jsonl".to_string(),
            None,
            true,
            true
        )
        .unwrap(),
        "samples.bendl"
    );
    assert_eq!(
        encode_setup(
            EncodeTarget::Xben,
            "samples.ben".to_string(),
            None,
            true,
            true
        )
        .unwrap(),
        "samples.bendl"
    );
    assert_eq!(
        encode_setup(
            EncodeTarget::Xben,
            "samples.xben".to_string(),
            None,
            true,
            true
        )
        .unwrap(),
        "samples.bendl"
    );
}

#[test]
fn encode_setup_respects_explicit_output() {
    assert_eq!(
        encode_setup(
            EncodeTarget::Ben,
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
        EncodeTarget::Ben,
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
    // A full decode lands on JSONL, so the bare stem gains a `.jsonl` extension.
    assert_eq!(
        decode_setup("samples.ben".to_string(), None, false, true).unwrap(),
        "samples.jsonl"
    );
    // x-decode stopping at BEN keeps the intermediate `.ben`.
    assert_eq!(
        decode_setup("samples.xben".to_string(), None, false, true).unwrap(),
        "samples.ben"
    );
    assert_eq!(
        decode_setup("samples.xben".to_string(), None, true, true).unwrap(),
        "samples.jsonl"
    );
}

#[test]
fn decode_setup_leaves_legacy_stacked_jsonl_names() {
    assert_eq!(
        decode_setup("samples.jsonl.ben".to_string(), None, false, true).unwrap(),
        "samples.jsonl"
    );
    assert_eq!(
        decode_setup("samples.jsonl.xben".to_string(), None, true, true).unwrap(),
        "samples.jsonl"
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
fn count_jsonl_lines_counts_nonempty_lines() {
    let path = unique_path("count.jsonl");
    fs::write(&path, b"{\"a\":1}\n\n{\"b\":2}\n").unwrap();
    let count = count_jsonl_lines(&path).unwrap();
    assert_eq!(count, 2);
    fs::remove_file(path).unwrap();
}

mod rewrite;

// =====================================================================
// bundle (.bendl) graph asset
// =====================================================================

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
    use crate::io::bundle::BendlReader;
    use std::io::Cursor;

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

    let graph_path = write_temp_graph("dup_graph.json");
    let err = append_graph_asset(bendl_path.to_str().unwrap(), &graph_path).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::Other);
    assert!(err.to_string().contains("failed to add graph asset"));

    let _ = fs::remove_file(&bendl_path);
    let _ = fs::remove_file(&graph_path);
}

#[test]
fn run_xencode_bundle_with_graph_errors_on_invalid_jsonl() {
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
    assert!(err.kind() != io::ErrorKind::NotFound);

    let _ = fs::remove_file(&bad_ben);
    let _ = fs::remove_file(&graph);
    let _ = fs::remove_file(&out);
}
