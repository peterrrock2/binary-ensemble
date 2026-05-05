use super::args::Args;
use super::helpers::{
    ben_variant_name, ordering_method_name, read_node_permutation_map_file, relabeling_label,
    to_ben_variant, to_graph_ordering,
};
use crate::json::graph::{sort_json_file_by_key, sort_json_file_by_ordering};
use crate::ops::relabel::{
    convert_ben_file, convert_ben_file_limit, relabel_ben_file, relabel_ben_file_as_variant,
    relabel_ben_file_as_variant_limit, relabel_ben_file_limit, relabel_ben_file_with_map,
    relabel_ben_file_with_map_as_variant, relabel_ben_file_with_map_as_variant_limit,
    relabel_ben_file_with_map_limit,
};
use serde_json::json;
use std::fs::File;
use std::io::{BufReader, BufWriter, Write};

pub(super) fn run_ben_mode(args: Args) -> Result<(), String> {
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
                        + "_first_seen_relabeled.jsonl.ben"
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
            .ok_or_else(|| "Provide --map-file, --key, or --ordering in BEN mode.".to_string())?
            .to_owned();
    }

    let (new_to_old_node_map, label) = read_node_permutation_map_file(&map_file_name)?;

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
