//! `ben xencode` handler.

use super::super::args::{resolve_variant, Globals, XencodeArgs};
use super::super::bundle::run_xencode_bundle_with_graph;
use super::super::paths::{
    encode_setup, open_derived_writer, open_reader, open_writer, EncodeTarget,
};

use crate::cli::common::{CliError, CliResult};
use crate::codec::encode::{cpus_from_signed, encode_ben_to_xben, encode_jsonl_to_xben};
use std::path::Path;

/// Execute the `xencode` subcommand.
pub(in crate::cli::ben) fn run(args: XencodeArgs, g: &Globals) -> CliResult {
    tracing::info!("Running in xencode mode");

    // BEN input recompresses to XBEN; JSONL input encodes to XBEN. Auto-detect from the extension,
    // falling back to the `--from-ben` flag (the only signal available for stdin input).
    let from_ben = args.from_ben
        || args
            .input_file
            .as_ref()
            .is_some_and(|f| f.ends_with(".ben"));

    // --graph path: produce a .bendl file with the XBEN stream plus a post-stream graph asset.
    if let Some(graph_path) = args.graph.as_ref() {
        let in_file = args.input_file.as_ref().ok_or_else(|| {
            CliError::other("--graph requires an input file (stdin not supported).")
        })?;
        if g.print {
            return Err(CliError::other("--graph is incompatible with --print."));
        }
        let out_path = encode_setup(
            EncodeTarget::Xben,
            in_file.clone(),
            g.output_file.clone(),
            g.overwrite,
            true,
        )?;
        let variant = resolve_variant(args.variant, args.save_all);
        run_xencode_bundle_with_graph(
            Path::new(in_file),
            &out_path,
            variant,
            from_ben,
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
        Some(in_file) if !g.print => {
            let path = encode_setup(
                EncodeTarget::Xben,
                in_file.clone(),
                g.output_file.clone(),
                g.overwrite,
                false,
            )?;
            open_derived_writer(path)?
        }
        _ => open_writer(g.output_file.as_deref(), g.print, g.overwrite)?,
    };

    if from_ben {
        encode_ben_to_xben(
            reader,
            writer,
            args.n_cpus.map(cpus_from_signed),
            args.compression_level,
            args.chunk_size,
            args.xz_block_size,
        )?;
    } else {
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
    }
    Ok(())
}
