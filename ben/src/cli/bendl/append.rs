use super::args::{AppendArgs, NamedAsset};
use super::helpers::append_file_asset;
use crate::io::bundle::format::{
    ASSET_TYPE_CUSTOM, ASSET_TYPE_GRAPH, ASSET_TYPE_METADATA, ASSET_TYPE_NODE_PERMUTATION_MAP,
};
use crate::io::bundle::writer::BendlAppender;
use crate::io::bundle::AddAssetOptions;
use std::fs::OpenOptions;

pub(super) fn run_append(args: AppendArgs) -> Result<(), String> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&args.input)
        .map_err(|e| format!("failed to open {:?} for read+write: {e}", args.input))?;
    let mut appender =
        BendlAppender::open(file).map_err(|e| format!("failed to open appender: {e}"))?;

    let mut added = 0usize;
    if let Some(ref path) = args.metadata {
        append_file_asset(
            &mut appender,
            ASSET_TYPE_METADATA,
            "metadata.json",
            path,
            AddAssetOptions::defaults().json(),
        )?;
        added += 1;
    }
    if let Some(ref path) = args.graph {
        let opts = if args.graph_raw {
            AddAssetOptions::defaults().json().raw()
        } else {
            AddAssetOptions::defaults().json()
        };
        append_file_asset(&mut appender, ASSET_TYPE_GRAPH, "graph.json", path, opts)?;
        added += 1;
    }
    if let Some(ref path) = args.node_permutation_map {
        append_file_asset(
            &mut appender,
            ASSET_TYPE_NODE_PERMUTATION_MAP,
            "node_permutation_map.json",
            path,
            AddAssetOptions::defaults().json(),
        )?;
        added += 1;
    }
    for NamedAsset { name, path } in &args.assets {
        append_file_asset(
            &mut appender,
            ASSET_TYPE_CUSTOM,
            name,
            path,
            AddAssetOptions::defaults(),
        )?;
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
