//! `ben relabel` handler: apply an external permutation (map, key sort, or graph ordering).

use super::args::{Globals, RelabelArgs};
use super::relabel_helpers::{
    ben_stem, ordering_method_name, read_node_permutation_map_file, relabel_in_place,
    relabeling_label, to_graph_ordering,
};
use crate::cli::common::check_overwrite;
use crate::json::graph::{sort_json_file_by_key, sort_json_file_by_ordering};
use crate::ops::relabel::{relabel_ben_file, RelabelOptions};
use serde_json::json;
use std::fs::File;
use std::io::{BufReader, BufWriter, Write};

pub(super) fn run(args: RelabelArgs, g: &Globals) -> Result<(), String> {
    if args.map_file.is_none() && args.key.is_none() && args.ordering.is_none() {
        return Err(concat!(
            "relabel needs a permutation source: provide --map-file, or --key/--ordering ",
            "together with a (JSON formatted) dual-graph file. To renumber districts in ",
            "first-seen order use `ben canonicalize`."
        )
        .to_string());
    }
    if args.map_file.is_some() && (args.key.is_some() || args.ordering.is_some()) {
        return Err(concat!(
            "Cannot provide both a map file and a sorting option. ",
            "Please provide either the map file or the key/ordering and the ",
            "(JSON formatted) dual-graph file needed to generate a map file."
        )
        .to_string());
    }

    let input_file = File::open(&args.input_file)
        .map_err(|e| format!("Could not open input file {:?}: {e}", args.input_file))?;
    let reader = BufReader::new(input_file);
    let output_variant = args.output_variant.map(|v| v.to_ben_variant());

    let mut map_file_name = String::new();
    if args.key.is_some() || args.ordering.is_some() {
        let dual_graph = args.dual_graph.as_ref().ok_or_else(|| {
            "No dual-graph file provided to go with the requested ordering.".to_string()
        })?;
        let label = relabeling_label(args.key.as_deref(), args.ordering.as_ref())?;
        tracing::info!("Creating map file for ordering: {}", label);

        let output_file_name = dual_graph.trim_end_matches(".json").to_owned()
            + format!("_sorted_by_{}.json", label).as_str();

        check_overwrite(&output_file_name, g.overwrite)
            .map_err(|e| format!("Could not use output file {output_file_name:?}: {e}"))?;
        let output_file = File::create(&output_file_name)
            .map_err(|e| format!("Could not create output file {output_file_name:?}: {e}"))?;
        let writer = BufWriter::new(output_file);

        let dual_graph_file = File::open(dual_graph)
            .map_err(|e| format!("Could not open dual-graph file {dual_graph:?}: {e}"))?;
        let dual_graph_reader = BufReader::new(dual_graph_file);
        let map = if let Some(key) = args.key.as_ref() {
            sort_json_file_by_key(dual_graph_reader, writer, key)
        } else {
            let ordering = args
                .ordering
                .as_ref()
                .ok_or_else(|| "Provide either --key or --ordering.".to_string())?;
            sort_json_file_by_ordering(dual_graph_reader, writer, to_graph_ordering(ordering))
        }
        .map_err(|e| format!("Could not sort dual-graph file: {e}"))?;

        map_file_name = dual_graph.trim_end_matches(".json").to_owned()
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
    }

    if map_file_name.is_empty() {
        map_file_name = args
            .map_file
            .as_ref()
            .ok_or_else(|| "Provide --map-file, --key, or --ordering.".to_string())?
            .to_owned();
    }

    let (new_to_old_node_map, label) = read_node_permutation_map_file(&map_file_name)?;

    if g.output_file.is_some() && args.add_suffix {
        return Err("Provide either --output-file or --add-suffix, not both.".to_string());
    }

    let output_file_name = match g.output_file.clone() {
        Some(name) => Some(name),
        None if args.add_suffix => Some(format!(
            "{}_sorted_by_{}.ben",
            ben_stem(&args.input_file),
            label
        )),
        None => None,
    };

    tracing::info!(
        "Relabeling ben file according to map file {}",
        map_file_name
    );

    let base = RelabelOptions::node_permutation(new_to_old_node_map);
    let options = if let Some(variant) = output_variant {
        base.with_target_variant(variant)
    } else {
        base
    }
    .with_max_samples_opt(args.n_items);

    match output_file_name {
        Some(name) => {
            check_overwrite(&name, g.overwrite)
                .map_err(|e| format!("Could not use output file {name:?}: {e}"))?;
            let output_file = File::create(&name)
                .map_err(|e| format!("Could not create output file {name:?}: {e}"))?;
            let writer = BufWriter::new(output_file);
            relabel_ben_file(reader, writer, options)
                .map_err(|e| format!("BEN relabeling with map {map_file_name:?} failed: {e}"))?;
        }
        None => {
            relabel_in_place(reader, &args.input_file, options)
                .map_err(|e| format!("BEN relabeling with map {map_file_name:?} failed: {e}"))?;
        }
    }
    Ok(())
}
