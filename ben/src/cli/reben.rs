use crate::cli::common::set_verbose;
use crate::{
    json::graph::sort_json_file_by_key,
    ops::relabel::{relabel_ben_file, relabel_ben_file_with_map},
};
use clap::{Parser, ValueEnum};
use serde_json::{json, Value};
use std::{
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
    /// Shape file to use for sorting the BEN file. Only needed
    /// in BEN mode when a map is not provided.
    #[arg(short, long)]
    shape_file: Option<String>,
    /// Map file to use for relabeling the BEN file.
    #[arg(short = 'p', long)]
    map_file: Option<String>,
    /// Mode to run the program in (either JSON or BEN).
    /// The JSON mode will sort a JSON file by a given key.
    /// The BEN mode will relabel a BEN file according to a map file
    /// or a key (the latter also requires a dual-graph file). If no
    /// map file or key is provided, the BEN mode will canonicalize
    /// the assignment vectors in the BEN file.
    #[arg(short, long)]
    mode: Mode,
    /// Verbosity level for the program.
    #[arg(short, long)]
    verbose: bool,
}

pub fn run() {
    let args = Args::parse();
    set_verbose(args.verbose);

    match &args.mode {
        Mode::Json => {
            let input_file = File::open(&args.input_file).expect("Could not open input file.");
            let reader = BufReader::new(input_file);

            let key = args.key.as_ref().expect("No key provided.");

            let output_file_name = match args.output_file {
                Some(name) => name,
                None => {
                    args.input_file.trim_end_matches(".json").to_owned()
                        + format!("_sorted_by_{}.json", key).as_str()
                }
            };

            let output_file =
                File::create(&output_file_name).expect("Could not create output file.");
            let writer = BufWriter::new(output_file);

            let map = sort_json_file_by_key(reader, writer, key);

            let map_file_name = args.input_file.trim_end_matches(".json").to_owned()
                + format!("_sorted_by_{}", key).as_str()
                + "_map.json";
            let map_file = File::create(map_file_name).expect("Could not create map file.");
            let mut map_writer = BufWriter::new(map_file);

            let map_json = json!({
                "input_file": args.input_file,
                "output_file": output_file_name,
                "key": key,
                "relabeling_old_to_new_nodes_map": map.unwrap()
            });

            map_writer
                .write_all(map_json.to_string().as_bytes())
                .expect("Could not write map file.");
        }
        Mode::Ben => {
            let input_file = File::open(&args.input_file).expect("Could not open input file.");
            let reader = BufReader::new(input_file);

            if args.map_file.is_none() && args.key.is_none() {
                tracing::trace!("Canonicalizing assignment vectors in ben file.");

                let output_file_name = match args.output_file {
                    Some(name) => name,
                    None => {
                        args.input_file.trim_end_matches(".jsonl.ben").to_owned()
                            + "_canonicalized_assignments.jsonl.ben"
                    }
                };

                let output_file =
                    File::create(&output_file_name).expect("Could not create output file.");
                let writer = BufWriter::new(output_file);

                relabel_ben_file(reader, writer).unwrap();
                return;
            }

            if args.map_file.is_some() && args.key.is_some() {
                panic!(concat!(
                    "Cannot provide both a map file and a key. ",
                    "Please provide either the map file or the key and the ",
                    "(JSON formatted) dual-graph file needed to generate a map file."
                ));
            }

            let mut map_file_name = String::new();
            if let Some(key) = args.key {
                if let Some(shape) = args.shape_file {
                    tracing::trace!("Creating map file for key: {}", key);

                    let output_file_name = shape.trim_end_matches(".json").to_owned()
                        + format!("_sorted_by_{}.json", key).as_str();

                    let output_file =
                        File::create(&output_file_name).expect("Could not create output file.");
                    let writer = BufWriter::new(output_file);

                    let shape_reader =
                        BufReader::new(File::open(&shape).expect("Could not open shape file."));
                    let map = sort_json_file_by_key(shape_reader, writer, &key);

                    map_file_name = shape.trim_end_matches(".json").to_owned()
                        + format!("_sorted_by_{}", key).as_str()
                        + "_map.json";
                    let map_file =
                        File::create(&map_file_name).expect("Could not create map file.");
                    let mut map_writer = BufWriter::new(map_file);

                    let map_json = json!({
                        "input_file": args.input_file,
                        "output_file": output_file_name,
                        "key": key,
                        "relabeling_old_to_new_nodes_map": map.unwrap()
                    });

                    map_writer
                        .write_all(map_json.to_string().as_bytes())
                        .expect("Could not write map file.");
                } else {
                    panic!("{}", format!("No shape file provided to go with key {:}", key));
                }
            }

            if map_file_name.is_empty() {
                map_file_name = args.map_file.as_ref().unwrap().to_owned();
            }
            let map_file = File::open(&map_file_name).expect("Could not open map file.");
            let map_reader = BufReader::new(map_file);

            let data: Value = serde_json::from_reader(map_reader).unwrap();

            let new_to_old_node_map = data["relabeling_old_to_new_nodes_map"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (v.as_u64().unwrap() as usize, k.parse::<usize>().unwrap()))
                .collect::<std::collections::HashMap<usize, usize>>();

            let key = data["key"].as_str().unwrap();

            let output_file_name = match args.output_file {
                Some(name) => name,
                None => {
                    args.input_file.trim_end_matches(".jsonl.ben").to_owned()
                        + format!("_sorted_by_{}.jsonl.ben", key).as_str()
                }
            };
            let output_file =
                File::create(&output_file_name).expect("Could not create output file.");
            let writer = BufWriter::new(output_file);

            tracing::trace!("Relabeling ben file according to map file {}", map_file_name,);

            relabel_ben_file_with_map(reader, writer, new_to_old_node_map).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};

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
}
