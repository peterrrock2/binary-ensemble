use super::*;

// =====================================================================
// relabel / canonicalize / reencode / sort-graph helpers
// =====================================================================

/// Minimal 3-node adjacency-style graph JSON, matching the shape `sort_json_file_by_*` accepts.
const SHAPE_JSON: &[u8] = br#"{"nodes":[{"id":0,"GEOID20":"B"},{"id":1,"GEOID20":"A"},{"id":2,"GEOID20":"C"}],"adjacency":[[{"id":1}],[{"id":0},{"id":2}],[{"id":1}]]}"#;

fn g_out(out: &Path) -> Globals {
    Globals {
        output_file: Some(out.to_string_lossy().into_owned()),
        overwrite: true,
        ..Default::default()
    }
}

fn g_none() -> Globals {
    Globals {
        overwrite: true,
        ..Default::default()
    }
}

/// Write a minimal Standard BEN file to a temp path and return the path.
fn write_temp_ben(name: &str) -> std::path::PathBuf {
    write_temp_ben_with(name, &[vec![1, 2, 3], vec![2, 1, 3]])
}

/// Write a Standard BEN file holding the given assignments (which may contain `0` and
/// non-consecutive labels) and return the path.
fn write_temp_ben_with(name: &str, assignments: &[Vec<u16>]) -> std::path::PathBuf {
    let path = unique_path(name);
    let jsonl = jsonl_from_assignments(assignments);
    let ben = sample_ben_bytes(&jsonl, BenVariant::Standard);
    fs::write(&path, &ben).unwrap();
    path
}

/// Decode every sample (repetitions expanded) from a BEN file on disk.
fn decode_ben_file(path: &Path) -> Vec<Vec<u16>> {
    let bytes = fs::read(path).unwrap();
    BenStreamReader::from_ben(Cursor::new(bytes))
        .unwrap()
        .silent(true)
        .flat_map(|r| {
            let (a, c) = r.unwrap();
            std::iter::repeat_n(a, c as usize)
        })
        .collect()
}

fn relabel_args(input: &Path) -> RelabelArgs {
    RelabelArgs {
        input_file: input.to_string_lossy().into_owned(),
        key: None,
        ordering: None,
        dual_graph: None,
        map_file: None,
        n_items: None,
        output_variant: None,
        add_suffix: false,
    }
}

fn canon_args(input: &Path) -> CanonicalizeArgs {
    CanonicalizeArgs {
        input_file: input.to_string_lossy().into_owned(),
        n_items: None,
        output_variant: None,
        add_suffix: false,
    }
}

fn reencode_args(input: &Path) -> ReencodeArgs {
    ReencodeArgs {
        input_file: input.to_string_lossy().into_owned(),
        output_variant: None,
        collapse_runs: false,
        n_items: None,
        add_suffix: false,
    }
}

// --- canonicalize ----------------------------------------------------

#[test]
fn canonicalize_relabels_first_seen_zero_based() {
    let samples = vec![vec![0u16, 5, 5, 3], vec![3u16, 3, 0, 5]];
    let input = write_temp_ben_with("canon_input.ben", &samples);
    let out = unique_path("canon_output.ben");

    canonicalize::run(canon_args(&input), &g_out(&out)).unwrap();

    // Per sample: [0,5,5,3] -> 0->0,5->1,3->2; [3,3,0,5] -> 3->0,0->1,5->2.
    assert_eq!(
        decode_ben_file(&out),
        vec![vec![0u16, 1, 1, 2], vec![0u16, 0, 1, 2]]
    );
    let _ = fs::remove_file(&input);
    let _ = fs::remove_file(&out);
}

#[test]
fn canonicalize_in_place_default() {
    let samples = vec![vec![2u16, 1, 3]];
    let input = write_temp_ben_with("canon_in_place.ben", &samples);

    canonicalize::run(canon_args(&input), &g_none()).unwrap();

    assert!(input.exists(), "input must remain after in-place replace");
    // First-seen 0-based relabel of [2,1,3] is [0,1,2].
    assert_eq!(decode_ben_file(&input), vec![vec![0u16, 1, 2]]);
    fs::remove_file(&input).unwrap();
}

#[test]
fn canonicalize_add_suffix_derives_name() {
    let input = write_temp_ben("canon_suffix.ben");
    let mut args = canon_args(&input);
    args.add_suffix = true;

    canonicalize::run(args, &g_none()).unwrap();

    let derived =
        input.to_str().unwrap().trim_end_matches(".ben").to_owned() + "_first_seen_relabeled.ben";
    let exists = Path::new(&derived).exists();
    let _ = fs::remove_file(&derived);
    fs::remove_file(&input).unwrap();
    assert!(exists, "canonicalize --add-suffix must derive {derived}");
}

#[test]
fn canonicalize_rejects_output_file_with_add_suffix() {
    let input = write_temp_ben("canon_conflict.ben");
    let mut args = canon_args(&input);
    args.add_suffix = true;
    let err = canonicalize::run(args, &g_out(Path::new("out.ben"))).unwrap_err();
    let _ = fs::remove_file(&input);
    assert!(err.contains("--output-file or --add-suffix"), "got: {err}");
}

// --- reencode --------------------------------------------------------

#[test]
fn reencode_rejects_no_op() {
    let err = reencode::run(reencode_args(Path::new("nope.ben")), &g_none()).unwrap_err();
    assert!(err.contains("nothing to do"), "got: {err}");
}

#[test]
fn reencode_default_preserves_arbitrary_labels() {
    // Variant change with no relabel must preserve `0` and non-consecutive ids verbatim.
    let samples = vec![vec![0u16, 5, 5, 3], vec![3u16, 3, 0, 5]];
    let input = write_temp_ben_with("reencode_preserve.ben", &samples);
    let out = unique_path("reencode_preserve_out.ben");

    let mut args = reencode_args(&input);
    args.output_variant = Some(CliVariant::Standard);
    reencode::run(args, &g_out(&out)).unwrap();

    assert_eq!(decode_ben_file(&out), samples);
    let _ = fs::remove_file(&input);
    let _ = fs::remove_file(&out);
}

#[test]
fn reencode_collapse_runs_round_trips() {
    let samples = vec![vec![1u16, 2, 3], vec![1u16, 2, 3], vec![2u16, 1, 3]];
    let input = write_temp_ben_with("reencode_collapse.ben", &samples);
    let out = unique_path("reencode_collapse_out.ben");

    let mut args = reencode_args(&input);
    args.collapse_runs = true;
    reencode::run(args, &g_out(&out)).unwrap();

    // Collapsing adjacent equal assignments must not change the expanded sample sequence.
    assert_eq!(decode_ben_file(&out), samples);
    let _ = fs::remove_file(&input);
    let _ = fs::remove_file(&out);
}

#[test]
fn reencode_in_place_default() {
    let samples = vec![vec![2u16, 1, 3]];
    let input = write_temp_ben_with("reencode_in_place.ben", &samples);
    let sibling = input.to_str().unwrap().trim_end_matches(".ben").to_owned() + "_standard.ben";

    let mut args = reencode_args(&input);
    args.output_variant = Some(CliVariant::Standard);
    reencode::run(args, &g_none()).unwrap();

    assert!(input.exists(), "input must remain after in-place replace");
    assert!(
        !Path::new(&sibling).exists(),
        "no suffixed sibling should be created by default"
    );
    // Labels preserved (distinguishes reencode from canonicalize, which would relabel to [0,1,2]).
    assert_eq!(decode_ben_file(&input), samples);
    fs::remove_file(&input).unwrap();
}

#[test]
fn reencode_add_suffix_variant_name() {
    let input = write_temp_ben("reencode_suffix_variant.ben");
    let mut args = reencode_args(&input);
    args.output_variant = Some(CliVariant::Standard);
    args.add_suffix = true;
    reencode::run(args, &g_none()).unwrap();

    let derived = input.to_str().unwrap().trim_end_matches(".ben").to_owned() + "_standard.ben";
    let exists = Path::new(&derived).exists();
    let _ = fs::remove_file(&derived);
    fs::remove_file(&input).unwrap();
    assert!(
        exists,
        "reencode --output-variant --add-suffix must derive {derived}"
    );
}

#[test]
fn reencode_add_suffix_reencode_name_without_variant() {
    let input = write_temp_ben("reencode_suffix_plain.ben");
    let mut args = reencode_args(&input);
    args.collapse_runs = true;
    args.add_suffix = true;
    reencode::run(args, &g_none()).unwrap();

    let derived = input.to_str().unwrap().trim_end_matches(".ben").to_owned() + "_reencode.ben";
    let exists = Path::new(&derived).exists();
    let _ = fs::remove_file(&derived);
    fs::remove_file(&input).unwrap();
    assert!(
        exists,
        "reencode --collapse-runs --add-suffix must derive {derived}"
    );
}

// --- relabel ---------------------------------------------------------

#[test]
fn relabel_with_map_file() {
    let input = write_temp_ben("relabel_map_input.ben");
    let map_path = unique_path("relabel_map.json");
    fs::write(
        &map_path,
        b"{\"node_permutation_old_to_new\":{\"0\":2,\"1\":0,\"2\":1}}",
    )
    .unwrap();
    let out = unique_path("relabel_map_out.ben");

    let mut args = relabel_args(&input);
    args.map_file = Some(map_path.to_string_lossy().into_owned());
    relabel::run(args, &g_out(&out)).unwrap();

    assert!(out.exists());
    for p in [&input, &map_path, &out] {
        let _ = fs::remove_file(p);
    }
}

#[test]
fn relabel_in_place_default() {
    let input = write_temp_ben("relabel_in_place.ben");
    let original = fs::read(&input).unwrap();
    let map_path = unique_path("relabel_in_place_map.json");
    fs::write(
        &map_path,
        b"{\"node_permutation_old_to_new\":{\"0\":2,\"1\":0,\"2\":1}}",
    )
    .unwrap();

    let mut args = relabel_args(&input);
    args.map_file = Some(map_path.to_string_lossy().into_owned());
    relabel::run(args, &g_none()).unwrap();

    assert!(input.exists(), "input must remain after in-place replace");
    assert_ne!(fs::read(&input).unwrap(), original);
    let _ = fs::remove_file(&input);
    let _ = fs::remove_file(&map_path);
}

#[test]
fn relabel_add_suffix_derives_sorted_name() {
    let input = write_temp_ben("relabel_suffix.ben");
    let map_path = unique_path("relabel_suffix_map.json");
    fs::write(
        &map_path,
        b"{\"key\":\"map\",\"node_permutation_old_to_new\":{\"0\":2,\"1\":0,\"2\":1}}",
    )
    .unwrap();

    let mut args = relabel_args(&input);
    args.map_file = Some(map_path.to_string_lossy().into_owned());
    args.add_suffix = true;
    relabel::run(args, &g_none()).unwrap();

    let derived =
        input.to_str().unwrap().trim_end_matches(".ben").to_owned() + "_sorted_by_map.ben";
    let exists = Path::new(&derived).exists();
    let _ = fs::remove_file(&derived);
    let _ = fs::remove_file(&input);
    let _ = fs::remove_file(&map_path);
    assert!(exists, "relabel --add-suffix must derive {derived}");
}

#[test]
fn relabel_rejects_output_file_with_add_suffix() {
    let input = write_temp_ben("relabel_conflict.ben");
    let map_path = unique_path("relabel_conflict_map.json");
    fs::write(
        &map_path,
        b"{\"node_permutation_old_to_new\":{\"0\":2,\"1\":0,\"2\":1}}",
    )
    .unwrap();

    let mut args = relabel_args(&input);
    args.map_file = Some(map_path.to_string_lossy().into_owned());
    args.add_suffix = true;
    let err = relabel::run(args, &g_out(Path::new("out.ben"))).unwrap_err();
    let _ = fs::remove_file(&input);
    let _ = fs::remove_file(&map_path);
    assert!(err.contains("--output-file or --add-suffix"), "got: {err}");
}

#[test]
fn relabel_with_key_and_dualgraph() {
    let input = write_temp_ben("relabel_key_input.ben");
    let shape = unique_path("relabel_key_shape.json");
    fs::write(&shape, SHAPE_JSON).unwrap();
    let out = unique_path("relabel_key_out.ben");

    let mut args = relabel_args(&input);
    args.key = Some("GEOID20".to_string());
    args.dual_graph = Some(shape.to_string_lossy().into_owned());
    relabel::run(args, &g_out(&out)).unwrap();

    let shape_stem = shape.to_str().unwrap().trim_end_matches(".json").to_owned();
    let _ = fs::remove_file(shape_stem.clone() + "_sorted_by_GEOID20_map.json");
    let _ = fs::remove_file(shape_stem + "_sorted_by_GEOID20.json");
    for p in [&input, &shape, &out] {
        let _ = fs::remove_file(p);
    }
}

#[test]
fn relabel_rejects_no_permutation_source() {
    let err = relabel::run(relabel_args(Path::new("x.ben")), &g_none()).unwrap_err();
    assert!(err.contains("permutation source"), "got: {err}");
}

#[test]
fn relabel_rejects_map_file_combined_with_key() {
    let mut args = relabel_args(Path::new("x.ben"));
    args.map_file = Some("m.json".to_string());
    args.key = Some("k".to_string());
    let err = relabel::run(args, &g_none()).unwrap_err();
    assert!(
        err.contains("map file") || err.contains("sorting option"),
        "got: {err}"
    );
}

#[test]
fn relabel_rejects_key_without_dual_graph() {
    let input = write_temp_ben("relabel_key_no_graph.ben");
    let mut args = relabel_args(&input);
    args.key = Some("GEOID20".to_string());
    let err = relabel::run(args, &g_none()).unwrap_err();
    let _ = fs::remove_file(&input);
    assert!(err.contains("dual-graph file"), "got: {err}");
}

// --- sort-graph ------------------------------------------------------

#[test]
fn sort_graph_with_key_derives_outputs() {
    let shape = unique_path("sort_graph_key.json");
    fs::write(&shape, SHAPE_JSON).unwrap();

    let args = SortGraphArgs {
        input_file: shape.to_string_lossy().into_owned(),
        key: Some("GEOID20".to_string()),
        ordering: None,
    };
    sort_graph::run(args, &g_none()).unwrap();

    let stem = shape.to_str().unwrap().trim_end_matches(".json").to_owned();
    let sorted = stem.clone() + "_sorted_by_GEOID20.json";
    let map = stem + "_sorted_by_GEOID20_map.json";
    assert!(Path::new(&sorted).exists());
    assert!(Path::new(&map).exists());
    let _ = fs::remove_file(&sorted);
    let _ = fs::remove_file(&map);
    let _ = fs::remove_file(&shape);
}

#[test]
fn sort_graph_with_ordering_derives_outputs() {
    let shape = unique_path("sort_graph_ord.json");
    fs::write(&shape, SHAPE_JSON).unwrap();

    let args = SortGraphArgs {
        input_file: shape.to_string_lossy().into_owned(),
        key: None,
        ordering: Some(OrderingMethod::ReverseCuthillMckee),
    };
    sort_graph::run(args, &g_none()).unwrap();

    let stem = shape.to_str().unwrap().trim_end_matches(".json").to_owned();
    let sorted = stem.clone() + "_sorted_by_reverse-cuthill-mckee.json";
    let map = stem + "_sorted_by_reverse-cuthill-mckee_map.json";
    assert!(Path::new(&sorted).exists());
    assert!(Path::new(&map).exists());
    let _ = fs::remove_file(&sorted);
    let _ = fs::remove_file(&map);
    let _ = fs::remove_file(&shape);
}

// --- helpers ---------------------------------------------------------

#[test]
fn ben_variant_name_covers_all_variants() {
    assert_eq!(ben_variant_name(BenVariant::Standard), "standard");
    assert_eq!(ben_variant_name(BenVariant::MkvChain), "mkvchain");
    assert_eq!(ben_variant_name(BenVariant::TwoDelta), "twodelta");
}

#[test]
fn relabeling_label_errors_on_both_key_and_ordering() {
    let err = relabeling_label(Some("k"), Some(&OrderingMethod::ReverseCuthillMckee)).unwrap_err();
    assert!(err.contains("either --key or --ordering"));
}

#[test]
fn relabeling_label_errors_on_neither() {
    let err = relabeling_label(None, None).unwrap_err();
    assert!(err.contains("either --key or --ordering"));
}

#[test]
fn read_node_permutation_map_file_rejects_non_integer_index() {
    let map_path = unique_path("bad_index_map.json");
    fs::write(&map_path, b"{\"node_permutation_old_to_new\":{\"x\":0}}").unwrap();
    let err = read_node_permutation_map_file(map_path.to_str().unwrap()).unwrap_err();
    let _ = fs::remove_file(&map_path);
    assert!(err.contains("invalid old node index"), "got: {err}");
}
