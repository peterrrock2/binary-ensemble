use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// Parsed form of a `name=path` option such as `--asset myblob=/tmp/x`.
#[derive(Debug, Clone)]
pub(super) struct NamedAsset {
    pub name: String,
    pub path: PathBuf,
}

impl std::str::FromStr for NamedAsset {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (name, path) = s
            .split_once('=')
            .ok_or_else(|| format!("expected NAME=PATH, got {s:?}"))?;
        if name.is_empty() {
            return Err("custom asset name must be non-empty".to_string());
        }
        Ok(NamedAsset {
            name: name.to_string(),
            path: PathBuf::from(path),
        })
    }
}

/// `bendl` CLI entry point.
#[derive(Parser, Debug)]
#[command(
    name = "bendl",
    about = "Create, inspect, extract from, append to, remove assets from, and compact .bendl bundle files.",
    version
)]
pub(super) struct Args {
    /// Enable verbose tracing output.
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Suppress in-place progress spinners. Trace logging is unaffected.
    #[arg(short = 'q', long, global = true)]
    pub quiet: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub(super) enum Command {
    /// Package a `.ben` or `.xben` stream (plus optional assets) into a `.bendl`.
    Create(CreateArgs),
    /// Print the header and directory of a `.bendl` file.
    Inspect(InspectArgs),
    /// Extract the embedded stream or a named asset to a file.
    Extract(ExtractArgs),
    /// Append new assets to an already-finalized `.bendl` file.
    Append(AppendArgs),
    /// Remove named assets from a finalized `.bendl` file, compacting it afterwards so the
    /// payload bytes are actually reclaimed.
    Remove(RemoveArgs),
    /// Rewrite a `.bendl` file in place without unreferenced byte ranges (dead space left by
    /// asset removals and superseded directories).
    Compact(CompactArgs),
}

#[derive(Parser, Debug)]
pub(super) struct CreateArgs {
    /// Path to the `.ben` or `.xben` assignment stream to embed. File extension chooses the
    /// container format.
    #[arg(short = 'i', long)]
    pub input: PathBuf,
    /// Destination `.bendl` path.
    #[arg(short = 'o', long)]
    pub output: PathBuf,
    /// Optional `graph.json` asset path. Will be stored under the standardized name `graph.json`
    /// and xz-compressed by default.
    #[arg(long)]
    pub graph: Option<PathBuf>,
    /// Optional `metadata.json` asset path. Stored under standardized name.
    #[arg(long)]
    pub metadata: Option<PathBuf>,
    /// Optional `node_permutation_map.json` asset path. Stored under standardized name.
    #[arg(long)]
    pub node_permutation_map: Option<PathBuf>,
    /// Additional custom assets, specified as `NAME=PATH`. May be repeated.
    #[arg(long = "asset")]
    pub assets: Vec<NamedAsset>,
    /// Overwrite the output file if it already exists.
    #[arg(short = 'w', long)]
    pub overwrite: bool,
    /// Store `graph.json` raw instead of compressing it.
    #[arg(long)]
    pub graph_raw: bool,
    /// xz preset (0-9) used when compressing assets added by this invocation. Defaults to the
    /// writer's preset (6). Assets are write-once and read-many, so the level only trades
    /// one-time write CPU against permanent file size.
    #[arg(long, value_parser = clap::value_parser!(u32).range(0..=9))]
    pub asset_compression_level: Option<u32>,
}

#[derive(Parser, Debug)]
pub(super) struct InspectArgs {
    /// `.bendl` file to inspect.
    pub input: PathBuf,
}

#[derive(Parser, Debug)]
pub(super) struct ExtractArgs {
    /// `.bendl` file to extract from.
    pub input: PathBuf,
    /// Output file path for the extracted bytes.
    #[arg(short = 'o', long)]
    pub output: PathBuf,
    /// Extract the embedded assignment stream region verbatim. Mutually exclusive with `--asset`.
    #[arg(long, conflicts_with = "asset")]
    pub stream: bool,
    /// Allow `--stream` extraction from an unfinalized bundle. This skips stream checksum
    /// verification because an unfinalized stream checksum is not authoritative.
    #[arg(long, requires = "stream")]
    pub allow_unfinalized: bool,
    /// Name of the asset to extract (e.g. `graph.json`). If the asset is xz-compressed, the
    /// extracted file contains the decompressed bytes.
    #[arg(long)]
    pub asset: Option<String>,
    /// Overwrite the output file if it already exists.
    #[arg(short = 'w', long)]
    pub overwrite: bool,
}

#[derive(Parser, Debug)]
pub(super) struct RemoveArgs {
    /// `.bendl` file to remove assets from. Must be finalized.
    pub input: PathBuf,
    /// Name of an asset to remove (e.g. `notes.txt`). May be repeated.
    #[arg(long = "asset", required = true)]
    pub assets: Vec<String>,
}

#[derive(Parser, Debug)]
pub(super) struct CompactArgs {
    /// `.bendl` file to compact in place.
    pub input: PathBuf,
}

#[derive(Parser, Debug)]
pub(super) struct AppendArgs {
    /// `.bendl` file to append to. Must be finalized (`complete == 1`).
    pub input: PathBuf,
    /// Optional `graph.json` asset path to add.
    #[arg(long)]
    pub graph: Option<PathBuf>,
    /// Optional `metadata.json` asset path to add.
    #[arg(long)]
    pub metadata: Option<PathBuf>,
    /// Optional `node_permutation_map.json` asset path to add.
    #[arg(long)]
    pub node_permutation_map: Option<PathBuf>,
    /// Additional custom assets, specified as `NAME=PATH`. May be repeated.
    #[arg(long = "asset")]
    pub assets: Vec<NamedAsset>,
    /// Store `graph.json` raw instead of compressing it.
    #[arg(long)]
    pub graph_raw: bool,
    /// xz preset (0-9) used when compressing assets added by this invocation. Defaults to the
    /// writer's preset (6). Assets are write-once and read-many, so the level only trades
    /// one-time write CPU against permanent file size.
    #[arg(long, value_parser = clap::value_parser!(u32).range(0..=9))]
    pub asset_compression_level: Option<u32>,
}
