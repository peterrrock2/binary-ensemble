use crate::cli::common::set_verbose;
use crate::{
    json::graph::{sort_json_file_by_key, sort_json_file_by_ordering, GraphOrderingMethod},
    ops::relabel::{
        convert_ben_file, convert_ben_file_limit, relabel_ben_file, relabel_ben_file_as_variant,
        relabel_ben_file_as_variant_limit, relabel_ben_file_limit, relabel_ben_file_with_map,
        relabel_ben_file_with_map_as_variant, relabel_ben_file_with_map_as_variant_limit,
        relabel_ben_file_with_map_limit,
    },
    BenVariant,
};
use clap::{Parser, ValueEnum};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs::File,
    io::{BufReader, BufWriter, Write},
};

#[derive(Parser, Debug, Clone, ValueEnum, PartialEq)]
/// Defines the mode of operation.
enum Mode {
    /// Sort a JSON dual graph by a key and emit a relabeling map.
    Json,
    /// Relabel or canonicalize a BEN file.
    Ben,
}

#[derive(Parser, Debug, Clone, ValueEnum, PartialEq)]
/// Topology-based ordering methods for JSON graph relabeling.
enum OrderingMethod {
    /// Recursive multilevel clustering based on local neighborhoods.
    #[clap(alias = "mlc")]
    MultiLevelCluster,
    /// Reverse Cuthill-McKee ordering.
    #[clap(alias = "rcm")]
    ReverseCuthillMckee,
}

#[derive(Parser, Debug, Clone, ValueEnum, PartialEq)]
/// BEN variants supported for BEN-mode output.
enum BenCliVariant {
    Standard,
    MkvChain,
    #[clap(alias = "twodelta")]
    TwoDelta,
}

#[derive(Parser, Debug)]
#[command(
    name = "Relabeling Binary Ensemble CLI Tool",
    about = concat!(
        "This is a command line tool for relabeling binary ensembles ",
        "to help improve compression ratios for BEN and XBEN files."
    ),
    version
)]
/// Defines the command line arguments accepted by the program.
// TODO: Change the name of shape_file to dual_graph_file.
struct Args {
    /// Input file to read from.
    #[arg()]
    input_file: String,
    /// Output file to write to.
    #[arg(short, long)]
    output_file: Option<String>,
    /// Key to sort the JSON or BEN file by.
    #[arg(short, long)]
    key: Option<String>,
    /// Topology-based ordering method to use instead of a key sort.
    #[arg(long, value_enum)]
    ordering: Option<OrderingMethod>,
    /// Shape file to use for sorting the BEN file. Only needed
    /// in BEN mode when a map is not provided.
    #[arg(short, long)]
    shape_file: Option<String>,
    /// Map file to use for relabeling the BEN file.
    #[arg(short = 'p', long)]
    map_file: Option<String>,
    /// Mode to run the program in (either JSON or BEN).
    /// The JSON mode will sort a JSON file by a given key or graph-ordering
    /// method. The BEN mode will relabel a BEN file according to a map file
    /// or a graph-ordering request (which also requires a dual-graph file). If no
    /// map file or key is provided, the BEN mode will canonicalize
    /// the assignment vectors in the BEN file.
    #[arg(short, long)]
    mode: Mode,
    /// Only relabel the first `n` expanded samples in BEN mode.
    #[arg(long)]
    n_items: Option<usize>,
    /// BEN variant to use for the BEN-mode output file.
    #[arg(long, value_enum)]
    output_variant: Option<BenCliVariant>,
    /// Rewrite the BEN stream without canonicalizing or map relabeling.
    #[arg(long)]
    convert_only: bool,
    /// Verbosity level for the program.
    #[arg(short, long)]
    verbose: bool,
}

/// Parse CLI arguments and execute the selected `reben` mode.
pub fn run() {
    let args = Args::parse();
    set_verbose(args.verbose);

    if let Err(err) = run_with_args(args) {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}

fn run_with_args(args: Args) -> Result<(), String> {
    match args.mode.clone() {
        Mode::Json => run_json_mode(args),
        Mode::Ben => run_ben_mode(args),
    }
}

fn run_json_mode(args: Args) -> Result<(), String> {
    if args.n_items.is_some() {
        return Err("--n-items is only supported in BEN mode.".to_string());
    }

    let input_file = File::open(&args.input_file)
        .map_err(|e| format!("Could not open input file {:?}: {e}", args.input_file))?;
    let reader = BufReader::new(input_file);
    let label = relabeling_label(args.key.as_deref(), args.ordering.as_ref())?;

    let output_file_name = match args.output_file {
        Some(name) => name,
        None => {
            args.input_file.trim_end_matches(".json").to_owned()
                + format!("_sorted_by_{}.json", label).as_str()
        }
    };

    let output_file = File::create(&output_file_name)
        .map_err(|e| format!("Could not create output file {output_file_name:?}: {e}"))?;
    let writer = BufWriter::new(output_file);

    let map = if let Some(key) = args.key.as_ref() {
        sort_json_file_by_key(reader, writer, key)
    } else {
        let ordering = args
            .ordering
            .as_ref()
            .ok_or_else(|| "Provide either --key or --ordering.".to_string())?;
        sort_json_file_by_ordering(reader, writer, to_graph_ordering(ordering))
    }
    .map_err(|e| format!("Could not sort input graph: {e}"))?;

    let map_file_name = args.input_file.trim_end_matches(".json").to_owned()
        + format!("_sorted_by_{}", label).as_str()
        + "_map.json";
    let map_file = File::create(&map_file_name)
        .map_err(|e| format!("Could not create map file {map_file_name:?}: {e}"))?;
    let mut map_writer = BufWriter::new(map_file);

    let map_json = json!({
        "input_file": args.input_file,
        "output_file": output_file_name,
        "key": args.key.as_ref(),
        "ordering_method": args.ordering.as_ref().map(ordering_method_name),
        "relabeling_old_to_new_nodes_map": map
    });

    map_writer
        .write_all(map_json.to_string().as_bytes())
        .map_err(|e| format!("Could not write map file {map_file_name:?}: {e}"))?;
    Ok(())
}

fn run_ben_mode(args: Args) -> Result<(), String> {
    if args.convert_only && args.output_variant.is_none() {
        return Err("--convert-only requires --output-variant.".to_string());
    }
    if args.convert_only
        && (args.map_file.is_some() || args.key.is_some() || args.ordering.is_some())
    {
        return Err("--convert-only cannot be combined with relabeling options.".to_string());
    }

    let input_file = File::open(&args.input_file)
        .map_err(|e| format!("Could not open input file {:?}: {e}", args.input_file))?;
    let reader = BufReader::new(input_file);
    let output_variant = args.output_variant.as_ref().map(to_ben_variant);

    if args.map_file.is_none() && args.key.is_none() && args.ordering.is_none() {
        if args.convert_only {
            tracing::trace!("Converting BEN file to requested variant.");
        } else {
            tracing::trace!("Canonicalizing assignment vectors in ben file.");
        }

        let output_file_name = match args.output_file {
            Some(name) => name,
            None => {
                if let Some(variant) = output_variant {
                    args.input_file.trim_end_matches(".ben").to_owned()
                        + format!("_{}.ben", ben_variant_name(variant)).as_str()
                } else {
                    args.input_file.trim_end_matches(".jsonl.ben").to_owned()
                        + "_canonicalized_assignments.jsonl.ben"
                }
            }
        };

        let output_file = File::create(&output_file_name)
            .map_err(|e| format!("Could not create output file {output_file_name:?}: {e}"))?;
        let writer = BufWriter::new(output_file);

        if args.convert_only {
            let variant = output_variant.expect("checked above");
            if let Some(limit) = args.n_items {
                convert_ben_file_limit(reader, writer, variant, limit)
            } else {
                convert_ben_file(reader, writer, variant)
            }
        } else if let Some(variant) = output_variant {
            if let Some(limit) = args.n_items {
                relabel_ben_file_as_variant_limit(reader, writer, variant, limit)
            } else {
                relabel_ben_file_as_variant(reader, writer, variant)
            }
        } else if let Some(limit) = args.n_items {
            relabel_ben_file_limit(reader, writer, limit)
        } else {
            relabel_ben_file(reader, writer)
        }
        .map_err(|e| format!("BEN relabeling failed: {e}"))?;
        return Ok(());
    }

    if args.map_file.is_some() && (args.key.is_some() || args.ordering.is_some()) {
        return Err(concat!(
            "Cannot provide both a map file and a sorting option. ",
            "Please provide either the map file or the key/ordering and the ",
            "(JSON formatted) dual-graph file needed to generate a map file."
        )
        .to_string());
    }

    let mut map_file_name = String::new();
    if args.key.is_some() || args.ordering.is_some() {
        let shape = args.shape_file.as_ref().ok_or_else(|| {
            "No shape file provided to go with the requested ordering.".to_string()
        })?;
        let label = relabeling_label(args.key.as_deref(), args.ordering.as_ref())?;
        tracing::trace!("Creating map file for ordering: {}", label);

        let output_file_name = shape.trim_end_matches(".json").to_owned()
            + format!("_sorted_by_{}.json", label).as_str();

        let output_file = File::create(&output_file_name)
            .map_err(|e| format!("Could not create output file {output_file_name:?}: {e}"))?;
        let writer = BufWriter::new(output_file);

        let shape_file =
            File::open(shape).map_err(|e| format!("Could not open shape file {shape:?}: {e}"))?;
        let shape_reader = BufReader::new(shape_file);
        let map = if let Some(key) = args.key.as_ref() {
            sort_json_file_by_key(shape_reader, writer, key)
        } else {
            let ordering = args
                .ordering
                .as_ref()
                .ok_or_else(|| "Provide either --key or --ordering.".to_string())?;
            sort_json_file_by_ordering(shape_reader, writer, to_graph_ordering(ordering))
        }
        .map_err(|e| format!("Could not sort shape file: {e}"))?;

        map_file_name = shape.trim_end_matches(".json").to_owned()
            + format!("_sorted_by_{}", label).as_str()
            + "_map.json";
        let map_file = File::create(&map_file_name)
            .map_err(|e| format!("Could not create map file {map_file_name:?}: {e}"))?;
        let mut map_writer = BufWriter::new(map_file);

        let map_json = json!({
            "input_file": args.input_file,
            "output_file": output_file_name,
            "key": args.key.as_ref(),
            "ordering_method": args.ordering.as_ref().map(ordering_method_name),
            "relabeling_old_to_new_nodes_map": map
        });

        map_writer
            .write_all(map_json.to_string().as_bytes())
            .map_err(|e| format!("Could not write map file {map_file_name:?}: {e}"))?;
    }

    if map_file_name.is_empty() {
        map_file_name = args
            .map_file
            .as_ref()
            .ok_or_else(|| "Provide --map-file, --key, or --ordering in BEN mode.".to_string())?
            .to_owned();
    }

    let (new_to_old_node_map, label) = read_relabel_map_file(&map_file_name)?;

    let output_file_name = match args.output_file {
        Some(name) => name,
        None => {
            args.input_file.trim_end_matches(".jsonl.ben").to_owned()
                + format!("_sorted_by_{}.jsonl.ben", label).as_str()
        }
    };
    let output_file = File::create(&output_file_name)
        .map_err(|e| format!("Could not create output file {output_file_name:?}: {e}"))?;
    let writer = BufWriter::new(output_file);

    tracing::trace!(
        "Relabeling ben file according to map file {}",
        map_file_name,
    );

    if let Some(variant) = output_variant {
        if let Some(limit) = args.n_items {
            relabel_ben_file_with_map_as_variant_limit(
                reader,
                writer,
                new_to_old_node_map,
                variant,
                limit,
            )
        } else {
            relabel_ben_file_with_map_as_variant(reader, writer, new_to_old_node_map, variant)
        }
    } else if let Some(limit) = args.n_items {
        relabel_ben_file_with_map_limit(reader, writer, new_to_old_node_map, limit)
    } else {
        relabel_ben_file_with_map(reader, writer, new_to_old_node_map)
    }
    .map_err(|e| format!("BEN relabeling with map {map_file_name:?} failed: {e}"))?;
    Ok(())
}

fn read_relabel_map_file(map_file_name: &str) -> Result<(HashMap<usize, usize>, String), String> {
    let map_file = File::open(map_file_name)
        .map_err(|e| format!("Could not open map file {map_file_name:?}: {e}"))?;
    let map_reader = BufReader::new(map_file);

    let data: Value = serde_json::from_reader(map_reader)
        .map_err(|e| format!("Could not parse map file {map_file_name:?} as JSON: {e}"))?;

    let map_obj = data
        .get("relabeling_old_to_new_nodes_map")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            format!(
                "Map file {map_file_name:?} must contain object field \
                 relabeling_old_to_new_nodes_map"
            )
        })?;

    let mut new_to_old_node_map = HashMap::with_capacity(map_obj.len());
    for (old_idx_text, new_idx_value) in map_obj {
        let old_idx = old_idx_text.parse::<usize>().map_err(|e| {
            format!(
                "Map file {map_file_name:?} contains invalid old node index {old_idx_text:?}: {e}"
            )
        })?;
        let new_idx = new_idx_value.as_u64().ok_or_else(|| {
            format!(
                "Map file {map_file_name:?} maps old node {old_idx} to non-integer value \
                 {new_idx_value}"
            )
        })? as usize;
        new_to_old_node_map.insert(new_idx, old_idx);
    }

    let label = data["key"]
        .as_str()
        .map(ToOwned::to_owned)
        .or_else(|| data["ordering_method"].as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| "map".to_string());

    Ok((new_to_old_node_map, label))
}

/// Convert a CLI ordering method variant to the library's graph ordering type.
///
/// # Arguments
///
/// * `ordering` - The CLI ordering method selected by the user.
///
/// # Returns
///
/// Returns the corresponding `GraphOrderingMethod`.
fn to_graph_ordering(ordering: &OrderingMethod) -> GraphOrderingMethod {
    match ordering {
        OrderingMethod::MultiLevelCluster => GraphOrderingMethod::MultiLevelCluster,
        OrderingMethod::ReverseCuthillMckee => GraphOrderingMethod::ReverseCuthillMckee,
    }
}

/// Return the kebab-case display name for an ordering method.
///
/// # Arguments
///
/// * `ordering` - The CLI ordering method variant.
///
/// # Returns
///
/// Returns a static string identifying the ordering method.
fn ordering_method_name(ordering: &OrderingMethod) -> &'static str {
    match ordering {
        OrderingMethod::MultiLevelCluster => "multi-level-cluster",
        OrderingMethod::ReverseCuthillMckee => "reverse-cuthill-mckee",
    }
}

/// Return the lowercase display name for a BEN variant.
///
/// # Arguments
///
/// * `variant` - The BEN variant to name.
///
/// # Returns
///
/// Returns a static string identifying the variant.
fn ben_variant_name(variant: BenVariant) -> &'static str {
    match variant {
        BenVariant::Standard => "standard",
        BenVariant::MkvChain => "mkvchain",
        BenVariant::TwoDelta => "twodelta",
    }
}

/// Convert a CLI BEN variant to the library's `BenVariant` type.
///
/// # Arguments
///
/// * `variant` - The CLI BEN variant selected by the user.
///
/// # Returns
///
/// Returns the corresponding `BenVariant`.
fn to_ben_variant(variant: &BenCliVariant) -> BenVariant {
    match variant {
        BenCliVariant::Standard => BenVariant::Standard,
        BenCliVariant::MkvChain => BenVariant::MkvChain,
        BenCliVariant::TwoDelta => BenVariant::TwoDelta,
    }
}

/// Derive a human-readable label from the key or ordering method for file naming.
///
/// # Arguments
///
/// * `key` - An optional JSON key used for sorting.
/// * `ordering` - An optional topology-based ordering method.
///
/// # Returns
///
/// Returns the label string, or `None` if neither option is provided.
fn relabeling_label(
    key: Option<&str>,
    ordering: Option<&OrderingMethod>,
) -> Result<String, String> {
    match (key, ordering) {
        (Some(_), Some(_)) => Err("Provide either --key or --ordering, not both.".to_string()),
        (Some(key), None) => Ok(key.to_string()),
        (None, Some(ordering)) => Ok(ordering_method_name(ordering).to_string()),
        (None, None) => Err("Provide either --key or --ordering.".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::encode::encode_jsonl_to_ben;
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
            "reben", "x.ben", "--mode", "ben", "--convert-only",
        ])
        .unwrap();
        let err = run_ben_mode(args).unwrap_err();
        assert!(err.contains("--output-variant"));
    }

    #[test]
    fn run_ben_mode_rejects_convert_only_with_relabeling() {
        let args = Args::try_parse_from([
            "reben", "x.ben", "--mode", "ben",
            "--convert-only", "--output-variant", "standard", "--key", "k",
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
        assert_eq!(to_ben_variant(&BenCliVariant::Standard), BenVariant::Standard);
    }

    #[test]
    fn relabeling_label_errors_on_both_key_and_ordering() {
        let err = relabeling_label(
            Some("k"),
            Some(&OrderingMethod::MultiLevelCluster),
        )
        .unwrap_err();
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
            "--mode", "ben",
            "--n-items", "1",
            "--output-file", out.to_str().unwrap(),
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
            "--mode", "json",
            "--ordering", "reverse-cuthill-mckee",
        ])
        .unwrap();
        let result = run_json_mode(args);
        // Clean up derived output file.
        let derived = shape.to_str().unwrap()
            .trim_end_matches(".json")
            .to_owned()
            + "_sorted_by_reverse-cuthill-mckee_map.json";
        let derived2 = shape.to_str().unwrap()
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
        use crate::codec::encode::encode_jsonl_to_ben;
        use std::io::Cursor;

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
            "--mode", "ben",
            "--map-file", map_path.to_str().unwrap(),
            "--n-items", "1",
            "--output-file", out.to_str().unwrap(),
        ])
        .unwrap();
        run_ben_mode(args).unwrap();

        for p in [&ben_path, &map_path, &out] { let _ = fs::remove_file(p); }
    }

    #[test]
    fn run_ben_mode_with_map_file_no_limit() {
        use crate::codec::encode::encode_jsonl_to_ben;
        use std::io::Cursor;

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
            "--mode", "ben",
            "--map-file", map_path.to_str().unwrap(),
            "--output-file", out.to_str().unwrap(),
        ])
        .unwrap();
        run_ben_mode(args).unwrap();

        for p in [&ben_path, &map_path, &out] { let _ = fs::remove_file(p); }
    }

    #[test]
    fn run_ben_mode_with_output_variant_and_n_items() {
        let input = write_temp_ben("var_n_items.jsonl.ben");
        let out = unique_path("var_n_items_output.jsonl.ben");
        let args = Args::try_parse_from([
            "reben", input.to_str().unwrap(),
            "--mode", "ben",
            "--output-variant", "standard",
            "--n-items", "1",
            "--output-file", out.to_str().unwrap(),
        ])
        .unwrap();
        run_ben_mode(args).unwrap();
        let _ = fs::remove_file(&input);
        let _ = fs::remove_file(&out);
    }

    #[test]
    fn run_ben_mode_with_shape_file_and_ordering() {
        // Covers the shape_file + ordering path (lines 265-269).
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
            "reben", input.to_str().unwrap(),
            "--mode", "ben",
            "--shape-file", shape.to_str().unwrap(),
            "--ordering", "reverse-cuthill-mckee",
            "--output-file", out.to_str().unwrap(),
        ])
        .unwrap();
        let result = run_ben_mode(args);
        // Clean up the map file the function derives automatically.
        let map = shape.to_str().unwrap()
            .trim_end_matches(".json")
            .to_owned()
            + "_sorted_by_reverse-cuthill-mckee_map.json";
        let sorted_json = shape.to_str().unwrap()
            .trim_end_matches(".json")
            .to_owned()
            + "_sorted_by_reverse-cuthill-mckee.json";
        let _ = fs::remove_file(&map);
        let _ = fs::remove_file(&sorted_json);
        for p in [&input, &shape, &out] { let _ = fs::remove_file(p); }
        result.unwrap();
    }

    #[test]
    fn run_ben_mode_with_map_file_and_output_variant_n_items() {
        use crate::codec::encode::encode_jsonl_to_ben;
        use std::io::Cursor;

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
            "reben", ben_path.to_str().unwrap(),
            "--mode", "ben",
            "--map-file", map_path.to_str().unwrap(),
            "--output-variant", "standard",
            "--n-items", "1",
            "--output-file", out.to_str().unwrap(),
        ])
        .unwrap();
        run_ben_mode(args).unwrap();
        for p in [&ben_path, &map_path, &out] { let _ = fs::remove_file(p); }
    }

    #[test]
    fn run_ben_mode_with_map_file_and_output_variant_no_limit() {
        use crate::codec::encode::encode_jsonl_to_ben;
        use std::io::Cursor;

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
            "reben", ben_path.to_str().unwrap(),
            "--mode", "ben",
            "--map-file", map_path.to_str().unwrap(),
            "--output-variant", "standard",
            "--output-file", out.to_str().unwrap(),
        ])
        .unwrap();
        run_ben_mode(args).unwrap();
        for p in [&ben_path, &map_path, &out] { let _ = fs::remove_file(p); }
    }

    #[test]
    fn run_ben_mode_map_file_without_output_file_derives_name() {
        // Covers the None branch of output_file (lines 306-307).
        use crate::codec::encode::encode_jsonl_to_ben;
        use std::io::Cursor;

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
            "reben", input.to_str().unwrap(),
            "--mode", "ben",
            "--map-file", map_path.to_str().unwrap(),
        ])
        .unwrap();
        let result = run_ben_mode(args);
        // Derived output: input stripped of ".jsonl.ben" + "_sorted_by_{label}.jsonl.ben"
        let derived = input.to_str().unwrap()
            .trim_end_matches(".jsonl.ben")
            .to_owned()
            + "_sorted_by_sort.jsonl.ben";
        let _ = fs::remove_file(&derived);
        for p in [&input, &map_path] { let _ = fs::remove_file(p); }
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
            "--mode", "ben",
        ])
        .unwrap();
        // run_ben_mode should succeed and write a derived output file.
        let result = run_ben_mode(args);
        // Clean up any produced files.
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
            "--mode", "ben",
            "--output-variant", "standard",
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
}
