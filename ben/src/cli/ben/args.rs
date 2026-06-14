//! `ben` CLI argument definitions.
//!
//! The CLI is a clap subcommand tree (`ben encode`, `ben relabel`, ...). Flags shared by every
//! subcommand live on [`Globals`], flattened onto the top-level command and marked `global = true`
//! so they parse before or after the subcommand name. Each subcommand carries only the options
//! that apply to it.

use crate::BenVariant;
use clap::{Args as ClapArgs, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

/// BEN variant selector shared by the encode and relabel/reencode subcommands.
#[derive(Debug, Clone, Copy, ValueEnum, PartialEq)]
pub(super) enum CliVariant {
    /// Store each sample independently.
    Standard,
    /// Store one frame plus a repetition count for repeated consecutive samples.
    #[value(alias = "mkv_chain", alias = "mkv-chain")]
    Mkvchain,
    /// Store delta-encoded frames.
    #[value(alias = "two_delta", alias = "two-delta")]
    Twodelta,
}

impl CliVariant {
    /// Map the CLI selector to the library's [`BenVariant`].
    pub(super) fn to_ben_variant(self) -> BenVariant {
        match self {
            CliVariant::Standard => BenVariant::Standard,
            CliVariant::Mkvchain => BenVariant::MkvChain,
            CliVariant::Twodelta => BenVariant::TwoDelta,
        }
    }
}

/// Resolve the BEN variant for an encode.
///
/// `--variant` takes precedence over `--save-all`. If neither is given, defaults to MkvChain.
pub(super) fn resolve_variant(variant: Option<CliVariant>, save_all: bool) -> BenVariant {
    match variant {
        Some(v) => v.to_ben_variant(),
        None if save_all => BenVariant::Standard,
        None => BenVariant::MkvChain,
    }
}

/// Topology-based ordering methods for dual-graph relabeling.
#[derive(Debug, Clone, Copy, ValueEnum, PartialEq)]
pub(super) enum OrderingMethod {
    /// Recursive multilevel clustering based on local neighborhoods.
    #[value(alias = "mlc")]
    MultiLevelCluster,
    /// Reverse Cuthill-McKee ordering.
    #[value(alias = "rcm")]
    ReverseCuthillMckee,
}

/// Top-level `ben` CLI.
#[derive(Parser, Debug)]
#[command(
    name = "ben",
    about = "Encode, decode, relabel, and convert binary ensemble files.",
    version
)]
pub(super) struct Cli {
    #[command(flatten)]
    pub globals: Globals,
    #[command(subcommand)]
    pub command: Command,
}

/// Flags shared by every subcommand. Marked `global = true` so they may appear before or after the
/// subcommand name.
#[derive(ClapArgs, Debug, Default)]
pub(super) struct Globals {
    /// Output file to write to. If omitted, the path is derived from the input file and
    /// subcommand.
    #[arg(short, long, global = true)]
    pub output_file: Option<String>,
    /// Write to stdout instead of a derived or explicit file. Higher priority than
    /// `--output-file`.
    #[arg(short, long, global = true)]
    pub print: bool,
    /// Overwrite an existing output file without prompting.
    #[arg(short = 'w', long, global = true)]
    pub overwrite: bool,
    /// Enable info-level logging. An explicit `RUST_LOG` still wins.
    #[arg(short, long, global = true)]
    pub verbose: bool,
    /// Suppress in-place progress spinners. Trace logging is unaffected.
    #[arg(short = 'q', long, global = true)]
    pub quiet: bool,
}

/// The `ben` subcommands.
#[derive(Subcommand, Debug)]
pub(super) enum Command {
    /// Encode JSONL into BEN.
    Encode(EncodeArgs),
    /// Encode JSONL or BEN into XBEN.
    Xencode(XencodeArgs),
    /// Decode BEN into JSONL, or XBEN one level into BEN.
    Decode(DecodeArgs),
    /// Fully decode XBEN into JSONL.
    Xdecode(XdecodeArgs),
    /// Look up a single sample from a BEN file (random-access decode).
    Lookup(LookupArgs),
    /// Compress an arbitrary stream with XZ.
    XzCompress(XzCompressArgs),
    /// Decompress an `.xz` file.
    XzDecompress(XzDecompressArgs),
    /// Relabel a BEN file by a permutation map, key sort, or graph ordering.
    Relabel(RelabelArgs),
    /// Canonicalize a BEN file: relabel districts in first-seen order, starting at 0.
    Canonicalize(CanonicalizeArgs),
    /// Re-encode a BEN file: change variant and/or collapse runs.
    Reencode(ReencodeArgs),
    /// Sort a dual-graph JSON by a key or ordering and emit a relabeling map.
    SortGraph(SortGraphArgs),
    /// Convert between BEN/XBEN and the PCOMPRESS format.
    Pcompress(PcompressArgs),
}

/// `ben encode` options.
#[derive(ClapArgs, Debug)]
pub(super) struct EncodeArgs {
    /// Input JSONL file. Reads stdin when omitted.
    pub input_file: Option<String>,
    /// BEN variant to encode into. Defaults to mkvchain. Takes precedence over `--save-all`.
    #[arg(short = 't', long, value_enum)]
    pub variant: Option<CliVariant>,
    /// Store every assignment vector without run-length repetition counts. Equivalent to
    /// `--variant standard`. Ignored if `--variant` is set.
    #[arg(short = 'a', long)]
    pub save_all: bool,
    /// Embed a graph JSON asset alongside the assignment stream and emit a `.bendl` bundle. The
    /// graph is added after the assignment stream has been fully written.
    #[arg(long)]
    pub graph: Option<PathBuf>,
}

/// `ben xencode` options.
#[derive(ClapArgs, Debug)]
pub(super) struct XencodeArgs {
    /// Input JSONL or BEN file. Reads stdin when omitted.
    pub input_file: Option<String>,
    /// BEN variant to encode into (JSONL input only). Defaults to mkvchain.
    #[arg(short = 't', long, value_enum)]
    pub variant: Option<CliVariant>,
    /// Store every assignment vector without run-length repetition counts (JSONL input only).
    #[arg(short = 'a', long)]
    pub save_all: bool,
    /// Embed a graph JSON asset and emit a `.bendl` bundle.
    #[arg(long)]
    pub graph: Option<PathBuf>,
    /// Treat the input as BEN (recompress to XBEN) rather than JSONL. Auto-detected from a `.ben`
    /// extension; needed only when reading BEN from stdin.
    #[arg(long)]
    pub from_ben: bool,
    /// Number of threads the XZ encoder may use. Defaults to 1. `-1` means every available core
    /// (sklearn convention); values above the host's parallelism are clamped down.
    #[arg(short = 'c', long, allow_hyphen_values = true)]
    pub n_cpus: Option<i32>,
    /// XZ compression level, 0-9. Defaults to the highest level.
    #[arg(short = 'l', long)]
    pub compression_level: Option<u32>,
    /// Number of TwoDelta delta frames per columnar chunk. Only affects the TwoDelta variant.
    /// Larger chunks improve XZ compression. Default is 10,000.
    #[arg(long)]
    pub chunk_size: Option<usize>,
    /// Per-block size in bytes for the multithreaded XZ encoder. liblzma needs a non-zero block
    /// size to fan compression out across threads. Defaults to 16 MiB when `--n-cpus > 1`, or 0
    /// (liblzma auto) for single-thread runs.
    #[arg(long)]
    pub xz_block_size: Option<u64>,
}

/// `ben decode` options.
#[derive(ClapArgs, Debug)]
pub(super) struct DecodeArgs {
    /// Input BEN or XBEN file. Reads stdin when omitted.
    pub input_file: Option<String>,
    /// Treat the input as XBEN (decode one level into BEN) rather than BEN (decode into JSONL).
    /// Auto-detected from a `.xben` extension; needed only when reading from stdin.
    #[arg(long)]
    pub from_xben: bool,
}

/// `ben xdecode` options.
#[derive(ClapArgs, Debug)]
pub(super) struct XdecodeArgs {
    /// Input XBEN file. Reads stdin when omitted.
    pub input_file: Option<String>,
}

/// `ben lookup` options.
#[derive(ClapArgs, Debug)]
pub(super) struct LookupArgs {
    /// Input BEN file.
    pub input_file: String,
    /// Sample number to extract.
    #[arg(short = 'n', long)]
    pub sample_number: usize,
}

/// `ben xz-compress` options.
#[derive(ClapArgs, Debug)]
pub(super) struct XzCompressArgs {
    /// Input file to compress.
    pub input_file: String,
    /// Number of threads the XZ encoder may use. Defaults to 1. `-1` means every available core.
    #[arg(short = 'c', long, allow_hyphen_values = true)]
    pub n_cpus: Option<i32>,
    /// XZ compression level, 0-9. Defaults to the highest level.
    #[arg(short = 'l', long)]
    pub compression_level: Option<u32>,
    /// Per-block size in bytes for the multithreaded XZ encoder.
    #[arg(long)]
    pub xz_block_size: Option<u64>,
}

/// `ben xz-decompress` options.
#[derive(ClapArgs, Debug)]
pub(super) struct XzDecompressArgs {
    /// Input `.xz` file.
    pub input_file: String,
}

/// `ben relabel` options: apply an external permutation (map, key sort, or graph ordering).
#[derive(ClapArgs, Debug)]
pub(super) struct RelabelArgs {
    /// Input BEN file to relabel.
    pub input_file: String,
    /// Key to sort the dual graph by, deriving the permutation.
    #[arg(short, long)]
    pub key: Option<String>,
    /// Topology-based ordering method to use instead of a key sort.
    #[arg(long, value_enum)]
    pub ordering: Option<OrderingMethod>,
    /// Dual-graph (JSON) file used to derive a permutation from `--key` or `--ordering`.
    #[arg(short = 'd', long = "dualgraph", alias = "shape-file")]
    pub dual_graph: Option<String>,
    /// Precomputed permutation map file to relabel by.
    #[arg(long)]
    pub map_file: Option<String>,
    /// Only relabel the first `n` expanded samples.
    #[arg(long)]
    pub n_items: Option<usize>,
    /// BEN variant for the output file.
    #[arg(long, value_enum)]
    pub output_variant: Option<CliVariant>,
    /// Write a suffixed sibling file instead of replacing the input in place.
    #[arg(long)]
    pub add_suffix: bool,
}

/// `ben canonicalize` options: relabel districts first-seen, 0-based.
#[derive(ClapArgs, Debug)]
pub(super) struct CanonicalizeArgs {
    /// Input BEN file to canonicalize.
    pub input_file: String,
    /// Only canonicalize the first `n` expanded samples.
    #[arg(long)]
    pub n_items: Option<usize>,
    /// BEN variant for the output file.
    #[arg(long, value_enum)]
    pub output_variant: Option<CliVariant>,
    /// Write a suffixed sibling file instead of replacing the input in place.
    #[arg(long)]
    pub add_suffix: bool,
}

/// `ben reencode` options: change the encoding without relabeling.
///
/// At least one of `--output-variant`, `--collapse-runs`, or `--n-items` must be set; a re-encode
/// that changes nothing is rejected rather than emitting an identical copy.
#[derive(ClapArgs, Debug)]
pub(super) struct ReencodeArgs {
    /// Input BEN file to re-encode.
    pub input_file: String,
    /// BEN variant for the output file.
    #[arg(long, value_enum)]
    pub output_variant: Option<CliVariant>,
    /// Collapse adjacent equal assignments, matching the historical conversion run policy. Without
    /// this, frame boundaries are preserved exactly.
    #[arg(long)]
    pub collapse_runs: bool,
    /// Only re-encode the first `n` expanded samples.
    #[arg(long)]
    pub n_items: Option<usize>,
    /// Write a suffixed sibling file instead of replacing the input in place.
    #[arg(long)]
    pub add_suffix: bool,
}

/// `ben sort-graph` options: sort a dual-graph JSON, emitting a sorted graph and a relabeling map.
#[derive(ClapArgs, Debug)]
pub(super) struct SortGraphArgs {
    /// Input dual-graph JSON file.
    pub input_file: String,
    /// Key to sort the graph by.
    #[arg(short, long)]
    pub key: Option<String>,
    /// Topology-based ordering method to use instead of a key sort.
    #[arg(long, value_enum)]
    pub ordering: Option<OrderingMethod>,
}

/// `ben pcompress` group: BEN/XBEN <-> PCOMPRESS conversions.
#[derive(ClapArgs, Debug)]
pub(super) struct PcompressArgs {
    #[command(subcommand)]
    pub direction: PcompressDirection,
}

/// Conversion direction for the `pcompress` subcommand.
#[derive(Subcommand, Debug)]
pub(super) enum PcompressDirection {
    /// Convert BEN into PCOMPRESS.
    FromBen(PcompressIoArgs),
    /// Convert PCOMPRESS into BEN.
    ToBen(PcompressIoArgs),
    /// Convert PCOMPRESS into XBEN.
    ToXben(PcompressIoArgs),
}

/// Shared input options for the `pcompress` directions.
#[derive(ClapArgs, Debug)]
pub(super) struct PcompressIoArgs {
    /// Input file to read from. Reads stdin when omitted.
    pub input_file: Option<String>,
}
