//! `ben sort-graph` handler: sort a dual-graph JSON, emitting a sorted graph and a relabeling map.

use super::args::{Globals, SortGraphArgs};
use super::relabel_helpers::{ordering_method_name, relabeling_label, to_graph_ordering};
use crate::cli::common::check_overwrite;
use crate::json::graph::{sort_json_file_by_key, sort_json_file_by_ordering};
use serde_json::json;
use std::fs::File;
use std::io::{BufReader, BufWriter, Write};

pub(super) fn run(args: SortGraphArgs, g: &Globals) -> Result<(), String> {
    let input_file = File::open(&args.input_file)
        .map_err(|e| format!("Could not open input file {:?}: {e}", args.input_file))?;
    let reader = BufReader::new(input_file);
    let label = relabeling_label(args.key.as_deref(), args.ordering.as_ref())?;

    let output_file_name = match g.output_file.clone() {
        Some(name) => name,
        None => {
            args.input_file.trim_end_matches(".json").to_owned()
                + format!("_sorted_by_{}.json", label).as_str()
        }
    };

    check_overwrite(&output_file_name, g.overwrite)
        .map_err(|e| format!("Could not use output file {output_file_name:?}: {e}"))?;
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
    check_overwrite(&map_file_name, g.overwrite)
        .map_err(|e| format!("Could not use map file {map_file_name:?}: {e}"))?;
    let map_file = File::create(&map_file_name)
        .map_err(|e| format!("Could not create map file {map_file_name:?}: {e}"))?;
    let mut map_writer = BufWriter::new(map_file);

    let map_json = json!({
        "input_file": args.input_file,
        "output_file": output_file_name,
        "key": args.key.as_ref(),
        "ordering_method": args.ordering.as_ref().map(ordering_method_name),
        "node_permutation_old_to_new": map
    });

    map_writer
        .write_all(map_json.to_string().as_bytes())
        .map_err(|e| format!("Could not write map file {map_file_name:?}: {e}"))?;
    Ok(())
}
