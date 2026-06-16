//! `ben encode` handler.

use super::super::args::{resolve_variant, EncodeArgs, Globals};
use super::super::bundle::run_encode_bundle_with_graph;
use super::super::paths::{
    encode_setup, open_derived_writer, open_reader, open_writer, EncodeTarget,
};

use crate::cli::common::{CliError, CliResult};
use crate::codec::encode::encode_jsonl_to_ben;
use std::path::Path;

/// Execute the `encode` subcommand.
pub(in crate::cli::ben) fn run(args: EncodeArgs, g: &Globals) -> CliResult {
    tracing::info!("Running in encode mode");

    // --graph path: produce a .bendl file with the BEN stream plus a post-stream graph asset.
    if let Some(graph_path) = args.graph.as_ref() {
        let in_file = args.input_file.as_ref().ok_or_else(|| {
            CliError::other("--graph requires an input file (stdin not supported).")
        })?;
        if g.print {
            return Err(CliError::other("--graph is incompatible with --print."));
        }
        let out_path = encode_setup(
            EncodeTarget::Ben,
            in_file.clone(),
            g.output_file.clone(),
            g.overwrite,
            true,
        )?;
        let variant = resolve_variant(args.variant, args.save_all);
        run_encode_bundle_with_graph(Path::new(in_file), &out_path, variant, graph_path)?;
        return Ok(());
    }

    let reader = open_reader(args.input_file.as_deref())?;
    let writer = match args.input_file.as_ref() {
        Some(in_file) if !g.print => {
            let path = encode_setup(
                EncodeTarget::Ben,
                in_file.clone(),
                g.output_file.clone(),
                g.overwrite,
                false,
            )?;
            open_derived_writer(path)?
        }
        _ => open_writer(g.output_file.as_deref(), g.print, g.overwrite)?,
    };

    let variant = resolve_variant(args.variant, args.save_all);
    encode_jsonl_to_ben(reader, writer, variant)?;
    Ok(())
}
