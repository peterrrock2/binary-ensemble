//! `ben --mode x-encode` handler.

use super::super::args::{resolve_variant, Args};
use super::super::bundle::run_xencode_bundle_with_graph;
use super::super::paths::{encode_setup, open_derived_writer, open_reader, open_writer};

use crate::cli::common::{CliError, CliResult};
use crate::codec::encode::{cpus_from_signed, encode_ben_to_xben, encode_jsonl_to_xben};
use std::path::Path;

/// Execute the `x-encode` sub-mode.
pub(in crate::cli::ben) fn run(args: Args) -> CliResult {
    tracing::info!("Running in xencode mode");

    let mut ben_and_xben = args.ben_and_xben;
    let mut jsonl_and_xben = args.jsonl_and_xben;

    if let Some(in_file) = args.input_file.as_ref() {
        if in_file.ends_with(".ben") {
            ben_and_xben = true;
        } else if in_file.ends_with(".jsonl") {
            jsonl_and_xben = true;
        }
    }

    // --graph path: produce a .bendl file with the XBEN stream plus a post-stream graph asset.
    if let Some(graph_path) = args.graph.as_ref() {
        let in_file = args.input_file.as_ref().ok_or_else(|| {
            CliError::other("--graph requires an input file (stdin not supported).")
        })?;
        if args.print {
            return Err(CliError::other("--graph is incompatible with --print."));
        }
        if !ben_and_xben && !jsonl_and_xben {
            return Err(CliError::other("Unsupported file type(s) for xencode mode"));
        }
        let out_path = encode_setup(
            args.mode.clone(),
            in_file.clone(),
            args.output_file.clone(),
            args.overwrite,
            true,
        )?;
        let variant = resolve_variant(args.variant, args.save_all);
        run_xencode_bundle_with_graph(
            Path::new(in_file),
            &out_path,
            variant,
            ben_and_xben,
            args.n_cpus.map(cpus_from_signed),
            args.compression_level,
            args.chunk_size,
            args.xz_block_size,
            graph_path,
        )?;
        return Ok(());
    }

    let reader = open_reader(args.input_file.as_deref())?;
    let writer = match args.input_file.as_ref() {
        Some(in_file) if !args.print => {
            let path = encode_setup(
                args.mode.clone(),
                in_file.clone(),
                args.output_file.clone(),
                args.overwrite,
                false,
            )?;
            open_derived_writer(path)?
        }
        _ => open_writer(args.output_file.as_deref(), args.print, args.overwrite)?,
    };

    if ben_and_xben {
        encode_ben_to_xben(
            reader,
            writer,
            args.n_cpus.map(cpus_from_signed),
            args.compression_level,
            args.chunk_size,
            args.xz_block_size,
        )?;
        Ok(())
    } else if jsonl_and_xben {
        let variant = resolve_variant(args.variant, args.save_all);
        encode_jsonl_to_xben(
            reader,
            writer,
            variant,
            args.n_cpus.map(cpus_from_signed),
            args.compression_level,
            args.chunk_size,
            args.xz_block_size,
        )?;
        Ok(())
    } else {
        Err(CliError::other("Unsupported file type(s) for xencode mode"))
    }
}
