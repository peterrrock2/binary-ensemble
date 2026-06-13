//! `bendl remove` and `bendl compact`: drop assets from a bundle and reclaim dead space.
//!
//! Removal goes through [`remove_assets_in_place`], which drops the directory entries and
//! reclaims their bytes as one operation, so "removed" means the bytes are actually gone from
//! the file, and a failure partway leaves the bundle untouched, assets still present. `compact`
//! is the standalone reclamation form, useful after many appends (each of which leaves a
//! superseded directory behind).

use super::args::{CompactArgs, RemoveArgs};
use crate::io::bundle::compact::{compact_bundle_in_place, remove_assets_in_place, Compaction};

fn describe(kind: Compaction) -> &'static str {
    match kind {
        Compaction::None => "already compact",
        Compaction::TailRewrite => "tail rewrite; stream untouched",
        Compaction::FullRewrite => "full rewrite",
    }
}

pub(super) fn run_remove(args: RemoveArgs) -> Result<(), String> {
    let names: Vec<&str> = args.assets.iter().map(String::as_str).collect();
    let kind = remove_assets_in_place(&args.input, &names)
        .map_err(|e| format!("failed to remove asset(s): {e}"))?;
    eprintln!(
        "Removed {} asset(s) from {:?} and compacted it ({})",
        args.assets.len(),
        args.input,
        describe(kind)
    );
    Ok(())
}

pub(super) fn run_compact(args: CompactArgs) -> Result<(), String> {
    let kind = compact_bundle_in_place(&args.input)
        .map_err(|e| format!("failed to compact bundle: {e}"))?;
    eprintln!("Compacted {:?} ({})", args.input, describe(kind));
    Ok(())
}
