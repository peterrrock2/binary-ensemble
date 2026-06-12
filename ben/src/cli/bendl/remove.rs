//! `bendl remove` and `bendl compact`: drop assets from a bundle and reclaim dead space.
//!
//! Removal at the appender level only rewrites the directory; the payload bytes stay behind as
//! unreferenced dead space. The `remove` subcommand therefore compacts the bundle afterwards, so
//! "removed" means the bytes are actually gone from the file. `compact` is the standalone form,
//! useful after many appends (each of which leaves a superseded directory behind).

use super::args::{CompactArgs, RemoveArgs};
use crate::io::bundle::compact::{compact_bundle_in_place, Compaction};
use crate::io::bundle::writer::BendlAppender;
use std::fs::OpenOptions;

fn describe(kind: Compaction) -> &'static str {
    match kind {
        Compaction::None => "already compact",
        Compaction::TailRewrite => "tail rewrite; stream untouched",
        Compaction::FullRewrite => "full rewrite",
    }
}

pub(super) fn run_remove(args: RemoveArgs) -> Result<(), String> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&args.input)
        .map_err(|e| format!("failed to open {:?} for read+write: {e}", args.input))?;
    let mut appender =
        BendlAppender::open(file).map_err(|e| format!("failed to open appender: {e}"))?;
    for name in &args.assets {
        appender
            .remove_asset(name)
            .map_err(|e| format!("failed to remove asset: {e}"))?;
    }
    appender
        .commit()
        .map_err(|e| format!("failed to commit removal: {e}"))?;

    // Removal only rewrites the directory; compact so the payload bytes are actually gone.
    let kind = compact_bundle_in_place(&args.input)
        .map_err(|e| format!("failed to compact bundle after removal: {e}"))?;
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
