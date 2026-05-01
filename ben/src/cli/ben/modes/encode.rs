//! `ben --mode encode` handler.

use super::super::args::{resolve_variant, Args};
use super::super::bundle::run_encode_bundle_with_graph;
use super::super::paths::{encode_setup, open_derived_writer, open_reader, open_writer};

use crate::cli::common::{CliError, CliResult};
use crate::codec::encode::encode_jsonl_to_ben;
use std::path::Path;

/// Execute the `encode` sub-mode.
pub(in crate::cli::ben) fn run(args: Args) -> CliResult {
    tracing::trace!("Running in encode mode");

    // --graph path: produce a .bendl bundle with the BEN stream
    // plus a post-stream graph asset.
    if let Some(graph_path) = args.graph.as_ref() {
        let in_file = args.input_file.as_ref().ok_or_else(|| {
            CliError::other("--graph requires an input file (stdin not supported).")
        })?;
        if args.print {
            return Err(CliError::other("--graph is incompatible with --print."));
        }
        let out_path = encode_setup(
            args.mode.clone(),
            in_file.clone(),
            args.output_file.clone(),
            args.overwrite,
            true,
        )?;
        let variant = resolve_variant(args.variant, args.save_all);
        run_encode_bundle_with_graph(Path::new(in_file), &out_path, variant, graph_path)?;
        return Ok(());
    }

    let reader = open_reader(args.input_file.as_deref());
    let writer = match args.input_file.as_ref() {
        Some(in_file) if !args.print => {
            let path = encode_setup(
                args.mode.clone(),
                in_file.clone(),
                args.output_file.clone(),
                args.overwrite,
                false,
            )?;
            open_derived_writer(path)
        }
        _ => open_writer(args.output_file.as_deref(), args.print, args.overwrite)?,
    };

    let variant = resolve_variant(args.variant, args.save_all);
    encode_jsonl_to_ben(reader, writer, variant)?;
    Ok(())
}
