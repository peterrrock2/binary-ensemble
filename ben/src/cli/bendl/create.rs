use super::args::{CreateArgs, NamedAsset};
use super::helpers::{add_file_asset, format_from_path, mode_str};
use crate::cli::common::check_overwrite;
use crate::io::bundle::format::{
    ASSET_TYPE_CUSTOM, ASSET_TYPE_GRAPH, ASSET_TYPE_METADATA, ASSET_TYPE_RELABEL_MAP,
};
use crate::io::bundle::{AddAssetOptions, BendlWriter};
use crate::io::reader::subsample::count_samples_from_file;
use std::fs::File;
use std::io::{self, BufReader};

pub(super) fn run_create(args: CreateArgs) -> Result<(), String> {
    let format = format_from_path(&args.input)?;
    check_overwrite(
        args.output.to_str().ok_or("non-utf8 output path")?,
        args.overwrite,
    )
    .map_err(|e| format!("{e}"))?;

    // Count samples up front so we can patch the header at finalize time.
    // This pre-scan is O(stream size); the second pass streams bytes directly.
    let sample_count: i64 = count_samples_from_file(&args.input, mode_str(format))
        .map_err(|e| format!("failed to count samples in {:?}: {e}", args.input))?
        as i64;

    let out_file = File::create(&args.output)
        .map_err(|e| format!("failed to create {:?}: {e}", args.output))?;
    let mut writer = BendlWriter::new(out_file, format)
        .map_err(|e| format!("failed to initialize bundle writer: {e}"))?;

    // Add singleton assets first, in canonical order.
    if let Some(ref path) = args.metadata {
        add_file_asset(
            &mut writer,
            ASSET_TYPE_METADATA,
            "metadata.json",
            path,
            AddAssetOptions::defaults().json(),
        )?;
    }
    if let Some(ref path) = args.graph {
        let opts = if args.graph_raw {
            AddAssetOptions::defaults().json().raw()
        } else {
            AddAssetOptions::defaults().json()
        };
        add_file_asset(&mut writer, ASSET_TYPE_GRAPH, "graph.json", path, opts)?;
    }
    if let Some(ref path) = args.relabel_map {
        add_file_asset(
            &mut writer,
            ASSET_TYPE_RELABEL_MAP,
            "relabel_map.json",
            path,
            AddAssetOptions::defaults().json(),
        )?;
    }
    for NamedAsset { name, path } in &args.assets {
        add_file_asset(
            &mut writer,
            ASSET_TYPE_CUSTOM,
            name,
            path,
            AddAssetOptions::defaults(),
        )?;
    }

    // Stream phase: copy bytes from the input file directly into the
    // bundle's stream region. This preserves the exact BEN/XBEN bytes.
    {
        let mut handle = writer
            .begin_stream()
            .map_err(|e| format!("failed to open stream region: {e}"))?;
        let mut input = BufReader::new(
            File::open(&args.input).map_err(|e| format!("failed to open {:?}: {e}", args.input))?,
        );
        io::copy(&mut input, &mut handle)
            .map_err(|e| format!("failed to copy assignment stream: {e}"))?;
        handle
            .finish(sample_count)
            .map_err(|e| format!("failed to close stream region: {e}"))?;
    }

    writer
        .finish()
        .map_err(|e| format!("failed to finalize bundle: {e}"))?;

    eprintln!(
        "Wrote {:?} ({} samples, format = {:?})",
        args.output, sample_count, format
    );
    Ok(())
}
