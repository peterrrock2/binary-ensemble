use super::args::{Args, BenCliVariant, Mode, OrderingMethod};
use super::ben_mode::run_ben_mode;
use super::helpers::{
    ben_variant_name, read_relabel_map_file, relabeling_label, to_ben_variant,
};
use super::json_mode::run_json_mode;
use crate::codec::encode::encode_jsonl_to_ben;
use crate::BenVariant;
use clap::{CommandFactory, Parser};
use std::{
    fs,
    io::Cursor,
    time::{SystemTime, UNIX_EPOCH},
};

fn unique_path(name: &str) -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("reben-{name}-{nonce}"))
}

/// Write a minimal Standard BEN file to a temp path and return the path.
fn write_temp_ben(name: &str) -> std::path::PathBuf {
    let path = unique_path(name);
    let jsonl = b"{\"assignment\":[1,2,3],\"sample\":1}\n{\"assignment\":[2,1,3],\"sample\":2}\n";
    let mut ben = Vec::new();
    encode_jsonl_to_ben(Cursor::new(jsonl), &mut ben, BenVariant::Standard).unwrap();
    fs::write(&path, &ben).unwrap();
    path
}

#[test]
fn clap_metadata_uses_package_version() {
    let mut command = Args::command();
    let help = command.render_long_help().to_string();

    assert_eq!(command.get_version(), Some(env!("CARGO_PKG_VERSION")));
    assert!(help.contains("Relabeling Binary Ensemble CLI Tool"));
    assert!(help.contains("--shape-file"));
    assert!(help.contains("canonicalize"));
}

#[test]
fn parse_json_mode_args() {
    let args = Args::try_parse_from([
        "reben",
        "dual_graph.json",
        "--mode",
        "json",
        "--key",
        "GEOID20",
        "--output-file",
        "sorted.json",
        "--verbose",
    ])
    .unwrap();

    assert_eq!(args.mode, Mode::Json);
    assert_eq!(args.input_file, "dual_graph.json");
    assert_eq!(args.key.as_deref(), Some("GEOID20"));
    assert_eq!(args.output_file.as_deref(), Some("sorted.json"));
    assert!(args.verbose);
}

#[test]
fn parse_json_mode_ordering_args() {
    let args = Args::try_parse_from([
        "reben",
        "dual_graph.json",
        "--mode",
        "json",
        "--ordering",
        "multi-level-cluster",
    ])
    .unwrap();

    assert_eq!(args.mode, Mode::Json);
    assert_eq!(args.ordering, Some(OrderingMethod::MultiLevelCluster));
    assert!(args.key.is_none());
}

#[test]
fn parse_ben_mode_n_items_args() {
    let args = Args::try_parse_from([
        "reben",
        "samples.jsonl.ben",
        "--mode",
        "ben",
        "--n-items",
        "25",
    ])
    .unwrap();

    assert_eq!(args.mode, Mode::Ben);
    assert_eq!(args.n_items, Some(25));
}

#[test]
fn parse_ben_mode_output_variant_args() {
    let args = Args::try_parse_from([
        "reben",
        "samples.jsonl.ben",
        "--mode",
        "ben",
        "--output-variant",
        "twodelta",
        "--convert-only",
    ])
    .unwrap();

    assert_eq!(args.mode, Mode::Ben);
    assert_eq!(args.output_variant, Some(BenCliVariant::TwoDelta));
    assert!(args.convert_only);
}

#[test]
fn run_json_mode_rejects_n_items() {
    let args = Args::try_parse_from([
        "reben", "x.json", "--mode", "json", "--key", "k", "--n-items", "5",
    ])
    .unwrap();
    let err = run_json_mode(args).unwrap_err();
    assert!(err.contains("--n-items"));
}

#[test]
fn run_ben_mode_rejects_convert_only_without_variant() {
    let args = Args::try_parse_from([
        "reben",
        "x.ben",
        "--mode",
        "ben",
        "--convert-only",
    ])
    .unwrap();
    let err = run_ben_mode(args).unwrap_err();
    assert!(err.contains("--output-variant"));
}

#[test]
fn run_ben_mode_rejects_convert_only_with_relabeling() {
    let args = Args::try_parse_from([
        "reben",
        "x.ben",
        "--mode",
        "ben",
        "--convert-only",
        "--output-variant",
        "standard",
        "--key",
        "k",
    ])
    .unwrap();
    let err = run_ben_mode(args).unwrap_err();
    assert!(err.contains("--convert-only cannot be combined"));
}

#[test]
fn ben_variant_name_covers_all_variants() {
    assert_eq!(ben_variant_name(BenVariant::Standard), "standard");
    assert_eq!(ben_variant_name(BenVariant::MkvChain), "mkvchain");
    assert_eq!(ben_variant_name(BenVariant::TwoDelta), "twodelta");
}

#[test]
fn to_ben_variant_covers_standard() {
    assert_eq!(
        to_ben_variant(&BenCliVariant::Standard),
        BenVariant::Standard
    );
}

#[test]
fn relabeling_label_errors_on_both_key_and_ordering() {
    let err =
        relabeling_label(Some("k"), Some(&OrderingMethod::MultiLevelCluster)).unwrap_err();
    assert!(err.contains("not both"));
}

#[test]
fn relabeling_label_errors_on_neither() {
    let err = relabeling_label(None, None).unwrap_err();
    assert!(err.contains("either"));
}

#[test]
fn run_ben_mode_with_n_items_limit() {
    let input = write_temp_ben("n_items_input.jsonl.ben");
    let out = unique_path("n_items_output.jsonl.ben");
    let args = Args::try_parse_from([
        "reben",
        input.to_str().unwrap(),
        "--mode",
        "ben",
        "--n-items",
        "1",
        "--output-file",
        out.to_str().unwrap(),
    ])
    .unwrap();
    run_ben_mode(args).unwrap();
    let _ = fs::remove_file(&input);
    let _ = fs::remove_file(&out);
}

#[test]
fn run_json_mode_with_ordering_derives_output_name() {
    // Create a minimal graph JSON file.
    let shape = unique_path("ordering_shape.json");
    fs::write(
        &shape,
        br#"{"nodes":[{"id":0},{"id":1},{"id":2}],"adjacency":[[{"id":1}],[{"id":0},{"id":2}],[{"id":1}]]}"#,
    )
    .unwrap();
    let args = Args::try_parse_from([
        "reben",
        shape.to_str().unwrap(),
        "--mode",
        "json",
        "--ordering",
        "reverse-cuthill-mckee",
    ])
    .unwrap();
    let result = run_json_mode(args);
    // Clean up derived output file.
    let derived = shape
        .to_str()
        .unwrap()
        .trim_end_matches(".json")
        .to_owned()
        + "_sorted_by_reverse-cuthill-mckee_map.json";
    let derived2 = shape
        .to_str()
        .unwrap()
        .trim_end_matches(".json")
        .to_owned()
        + "_sorted_by_reverse-cuthill-mckee.jsonl.ben";
    let _ = fs::remove_file(&derived);
    let _ = fs::remove_file(&derived2);
    let _ = fs::remove_file(&shape);
    result.unwrap();
}

#[test]
fn run_ben_mode_with_map_file_and_n_items() {
    // Build a 3-node BEN file.
    let jsonl = b"{\"assignment\":[1,2,3],\"sample\":1}\n{\"assignment\":[2,1,3],\"sample\":2}\n";
    let mut ben = Vec::new();
    encode_jsonl_to_ben(Cursor::new(jsonl), &mut ben, BenVariant::Standard).unwrap();
    let ben_path = unique_path("map_n_items.jsonl.ben");
    fs::write(&ben_path, &ben).unwrap();

    let map_path = unique_path("map_n_items_map.json");
    fs::write(
        &map_path,
        b"{\"relabeling_old_to_new_nodes_map\":{\"0\":2,\"1\":0,\"2\":1}}",
    )
    .unwrap();

    let out = unique_path("map_n_items_output.jsonl.ben");
    let args = Args::try_parse_from([
        "reben",
        ben_path.to_str().unwrap(),
        "--mode",
        "ben",
        "--map-file",
        map_path.to_str().unwrap(),
        "--n-items",
        "1",
        "--output-file",
        out.to_str().unwrap(),
    ])
    .unwrap();
    run_ben_mode(args).unwrap();

    for p in [&ben_path, &map_path, &out] {
        let _ = fs::remove_file(p);
    }
}

#[test]
fn run_ben_mode_with_map_file_no_limit() {
    let jsonl = b"{\"assignment\":[1,2,3],\"sample\":1}\n{\"assignment\":[2,1,3],\"sample\":2}\n";
    let mut ben = Vec::new();
    encode_jsonl_to_ben(Cursor::new(jsonl), &mut ben, BenVariant::Standard).unwrap();
    let ben_path = unique_path("map_nolimit.jsonl.ben");
    fs::write(&ben_path, &ben).unwrap();

    let map_path = unique_path("map_nolimit_map.json");
    fs::write(
        &map_path,
        b"{\"relabeling_old_to_new_nodes_map\":{\"0\":2,\"1\":0,\"2\":1}}",
    )
    .unwrap();

    let out = unique_path("map_nolimit_output.jsonl.ben");
    let args = Args::try_parse_from([
        "reben",
        ben_path.to_str().unwrap(),
        "--mode",
        "ben",
        "--map-file",
        map_path.to_str().unwrap(),
        "--output-file",
        out.to_str().unwrap(),
    ])
    .unwrap();
    run_ben_mode(args).unwrap();

    for p in [&ben_path, &map_path, &out] {
        let _ = fs::remove_file(p);
    }
}

#[test]
fn run_ben_mode_with_output_variant_and_n_items() {
    let input = write_temp_ben("var_n_items.jsonl.ben");
    let out = unique_path("var_n_items_output.jsonl.ben");
    let args = Args::try_parse_from([
        "reben",
        input.to_str().unwrap(),
        "--mode",
        "ben",
        "--output-variant",
        "standard",
        "--n-items",
        "1",
        "--output-file",
        out.to_str().unwrap(),
    ])
    .unwrap();
    run_ben_mode(args).unwrap();
    let _ = fs::remove_file(&input);
    let _ = fs::remove_file(&out);
}

#[test]
fn run_ben_mode_with_shape_file_and_ordering() {
    // Covers the shape_file + ordering path.
    // Creates a map from the shape file ordering, then relabels the BEN.
    let input = write_temp_ben("shape_order_input.jsonl.ben");
    let shape = unique_path("shape_order_shape.json");
    fs::write(
        &shape,
        br#"{"nodes":[{"id":0},{"id":1},{"id":2}],"adjacency":[[{"id":1}],[{"id":0},{"id":2}],[{"id":1}]]}"#,
    )
    .unwrap();
    let out = unique_path("shape_order_output.jsonl.ben");
    let args = Args::try_parse_from([
        "reben",
        input.to_str().unwrap(),
        "--mode",
        "ben",
        "--shape-file",
        shape.to_str().unwrap(),
        "--ordering",
        "reverse-cuthill-mckee",
        "--output-file",
        out.to_str().unwrap(),
    ])
    .unwrap();
    let result = run_ben_mode(args);
    // Clean up the map file the function derives automatically.
    let map = shape
        .to_str()
        .unwrap()
        .trim_end_matches(".json")
        .to_owned()
        + "_sorted_by_reverse-cuthill-mckee_map.json";
    let sorted_json = shape
        .to_str()
        .unwrap()
        .trim_end_matches(".json")
        .to_owned()
        + "_sorted_by_reverse-cuthill-mckee.json";
    let _ = fs::remove_file(&map);
    let _ = fs::remove_file(&sorted_json);
    for p in [&input, &shape, &out] {
        let _ = fs::remove_file(p);
    }
    result.unwrap();
}

#[test]
fn run_ben_mode_with_map_file_and_output_variant_n_items() {
    let jsonl = b"{\"assignment\":[1,2,3],\"sample\":1}\n{\"assignment\":[2,1,3],\"sample\":2}\n";
    let mut ben = Vec::new();
    encode_jsonl_to_ben(Cursor::new(jsonl), &mut ben, BenVariant::Standard).unwrap();
    let ben_path = unique_path("map_var_n.jsonl.ben");
    fs::write(&ben_path, &ben).unwrap();

    let map_path = unique_path("map_var_n_map.json");
    fs::write(
        &map_path,
        b"{\"relabeling_old_to_new_nodes_map\":{\"0\":2,\"1\":0,\"2\":1}}",
    )
    .unwrap();
    let out = unique_path("map_var_n_output.jsonl.ben");
    let args = Args::try_parse_from([
        "reben",
        ben_path.to_str().unwrap(),
        "--mode",
        "ben",
        "--map-file",
        map_path.to_str().unwrap(),
        "--output-variant",
        "standard",
        "--n-items",
        "1",
        "--output-file",
        out.to_str().unwrap(),
    ])
    .unwrap();
    run_ben_mode(args).unwrap();
    for p in [&ben_path, &map_path, &out] {
        let _ = fs::remove_file(p);
    }
}

#[test]
fn run_ben_mode_with_map_file_and_output_variant_no_limit() {
    let jsonl = b"{\"assignment\":[1,2,3],\"sample\":1}\n";
    let mut ben = Vec::new();
    encode_jsonl_to_ben(Cursor::new(jsonl), &mut ben, BenVariant::Standard).unwrap();
    let ben_path = unique_path("map_var_nolim.jsonl.ben");
    fs::write(&ben_path, &ben).unwrap();

    let map_path = unique_path("map_var_nolim_map.json");
    fs::write(
        &map_path,
        b"{\"relabeling_old_to_new_nodes_map\":{\"0\":2,\"1\":0,\"2\":1}}",
    )
    .unwrap();
    let out = unique_path("map_var_nolim_output.jsonl.ben");
    let args = Args::try_parse_from([
        "reben",
        ben_path.to_str().unwrap(),
        "--mode",
        "ben",
        "--map-file",
        map_path.to_str().unwrap(),
        "--output-variant",
        "standard",
        "--output-file",
        out.to_str().unwrap(),
    ])
    .unwrap();
    run_ben_mode(args).unwrap();
    for p in [&ben_path, &map_path, &out] {
        let _ = fs::remove_file(p);
    }
}

#[test]
fn run_ben_mode_map_file_without_output_file_derives_name() {
    // Covers the None branch of output_file.
    let jsonl = b"{\"assignment\":[1,2,3],\"sample\":1}\n";
    let mut ben = Vec::new();
    encode_jsonl_to_ben(Cursor::new(jsonl), &mut ben, BenVariant::Standard).unwrap();
    let input = unique_path("map_derive.jsonl.ben");
    fs::write(&input, &ben).unwrap();

    let map_path = unique_path("map_derive_map.json");
    fs::write(
        &map_path,
        b"{\"relabeling_old_to_new_nodes_map\":{\"0\":2,\"1\":0,\"2\":1},\"key\":\"sort\"}",
    )
    .unwrap();
    let args = Args::try_parse_from([
        "reben",
        input.to_str().unwrap(),
        "--mode",
        "ben",
        "--map-file",
        map_path.to_str().unwrap(),
    ])
    .unwrap();
    let result = run_ben_mode(args);
    // Derived output: input stripped of ".jsonl.ben" + "_sorted_by_{label}.jsonl.ben"
    let derived = input
        .to_str()
        .unwrap()
        .trim_end_matches(".jsonl.ben")
        .to_owned()
        + "_sorted_by_sort.jsonl.ben";
    let _ = fs::remove_file(&derived);
    for p in [&input, &map_path] {
        let _ = fs::remove_file(p);
    }
    result.unwrap();
}

#[test]
fn read_relabel_map_file_rejects_non_integer_index() {
    let map_path = unique_path("bad_index_map.json");
    fs::write(
        &map_path,
        b"{\"relabeling_old_to_new_nodes_map\":{\"not_a_number\":0}}",
    )
    .unwrap();
    let err = read_relabel_map_file(map_path.to_str().unwrap()).unwrap_err();
    assert!(err.contains("invalid old node index"));
    let _ = fs::remove_file(&map_path);
}

#[test]
fn read_relabel_map_file_rejects_non_integer_value() {
    let map_path = unique_path("bad_value_map.json");
    fs::write(
        &map_path,
        b"{\"relabeling_old_to_new_nodes_map\":{\"0\":\"not_a_number\"}}",
    )
    .unwrap();
    let err = read_relabel_map_file(map_path.to_str().unwrap()).unwrap_err();
    assert!(err.contains("non-integer"));
    let _ = fs::remove_file(&map_path);
}

#[test]
fn run_ben_mode_canonicalize_derives_output_name() {
    let input = write_temp_ben("canon.jsonl.ben");
    let args = Args::try_parse_from([
        "reben",
        input.to_str().unwrap(),
        "--mode",
        "ben",
    ])
    .unwrap();
    let result = run_ben_mode(args);
    let derived = input
        .to_str()
        .unwrap()
        .trim_end_matches(".jsonl.ben")
        .to_owned()
        + "_canonicalized_assignments.jsonl.ben";
    let _ = fs::remove_file(&derived);
    fs::remove_file(&input).unwrap();
    result.unwrap();
}

#[test]
fn run_ben_mode_with_output_variant_derives_name() {
    let input = write_temp_ben("variant.ben");
    let args = Args::try_parse_from([
        "reben",
        input.to_str().unwrap(),
        "--mode",
        "ben",
        "--output-variant",
        "standard",
    ])
    .unwrap();
    let result = run_ben_mode(args);
    let derived = input
        .to_str()
        .unwrap()
        .trim_end_matches(".ben")
        .to_owned()
        + "_standard.ben";
    let _ = fs::remove_file(&derived);
    fs::remove_file(&input).unwrap();
    result.unwrap();
}
