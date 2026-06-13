use super::args::{AppendArgs, NamedAsset};
use super::helpers::{append_custom_file_asset, append_known_file_asset};
use crate::io::bundle::format::KnownAssetKind;
use crate::io::bundle::writer::BendlAppender;
use crate::io::bundle::AddAssetOptions;
use std::fs::OpenOptions;

pub(super) fn run_append(args: AppendArgs) -> Result<(), String> {
    let base_opts = || {
        let opts = AddAssetOptions::defaults();
        match args.asset_compression_level {
            Some(level) => opts.compression_level(level),
            None => opts,
        }
    };
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&args.input)
        .map_err(|e| format!("failed to open {:?} for read+write: {e}", args.input))?;
    let mut appender =
        BendlAppender::open(file).map_err(|e| format!("failed to open appender: {e}"))?;

    let mut added = 0usize;
    if let Some(ref path) = args.metadata {
        append_known_file_asset(
            &mut appender,
            KnownAssetKind::Metadata,
            path,
            base_opts().json(),
        )?;
        added += 1;
    }
    if let Some(ref path) = args.graph {
        let opts = if args.graph_raw {
            base_opts().json().raw()
        } else {
            base_opts().json()
        };
        append_known_file_asset(&mut appender, KnownAssetKind::Graph, path, opts)?;
        added += 1;
    }
    if let Some(ref path) = args.node_permutation_map {
        append_known_file_asset(
            &mut appender,
            KnownAssetKind::NodePermutationMap,
            path,
            base_opts().json(),
        )?;
        added += 1;
    }
    for NamedAsset { name, path } in &args.assets {
        append_custom_file_asset(&mut appender, name, path, base_opts())?;
        added += 1;
    }

    if added == 0 {
        // Nothing to do; leave the file untouched.
        appender.abort();
        eprintln!("No assets specified; bundle is unchanged.");
        return Ok(());
    }

    appender
        .commit()
        .map_err(|e| format!("failed to commit append: {e}"))?;
    eprintln!("Appended {added} asset(s) to {:?}", args.input);
    Ok(())
}
