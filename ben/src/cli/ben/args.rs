use crate::BenVariant;
use clap::{Parser, ValueEnum};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq)]
pub(super) enum CliVariant {
    /// Store each sample independently.
    Standard,
    /// Store one frame plus a repetition count for repeated consecutive samples.
    #[value(alias = "mkv_chain")]
    Mkvchain,
    /// Store delta-encoded frames.
    #[value(alias = "two_delta")]
    Twodelta,
}

/// Resolve the BEN variant from the CLI flags.
///
/// `--variant` takes precedence over `--save-all`.
/// If neither is given, defaults to MkvChain.
pub(super) fn resolve_variant(variant: Option<CliVariant>, save_all: bool) -> BenVariant {
    match variant {
        Some(CliVariant::Standard) => BenVariant::Standard,
        Some(CliVariant::Mkvchain) => BenVariant::MkvChain,
        Some(CliVariant::Twodelta) => BenVariant::TwoDelta,
        None if save_all => BenVariant::Standard,
        None => BenVariant::MkvChain,
    }
}

#[derive(Parser, Debug, Clone, ValueEnum, PartialEq)]
/// Defines the mode of operation.
pub(super) enum Mode {
    /// Encode JSONL into BEN.
    Encode,
    /// Encode JSONL or BEN into XBEN.
    XEncode,
    /// Decode BEN or XBEN into its less compressed representation.
    Decode,
    /// Fully decode XBEN into JSONL.
    XDecode,
    /// Look up a single sample from a BEN file (random-access decode).
    Lookup,
    /// Compress an arbitrary stream with XZ.
    XzCompress,
    /// Decompress an `.xz` file.
    XzDecompress,
}

#[derive(Parser, Debug)]
#[command(
    name = "Binary Ensemble CLI Tool",
    about = "This is a command line tool for encoding and decoding binary ensemble files.",
    version
)]
/// Defines the command line arguments accepted by the program.
pub(super) struct Args {
    /// Mode to run the program in (encode, decode, or read).
    #[arg(short, long, value_enum)]
    pub mode: Mode,
    /// Input file to read from.
    #[arg()]
    pub input_file: Option<String>,
    /// Output file to write to. Optional.
    /// If not provided, the output file will be determined
    /// based on the input file and the mode of operation.
    #[arg(short, long)]
    pub output_file: Option<String>,
    /// The standard behaviour is to try and derive the output file
    /// name from the input file name. If this flag is set, then this
    /// logic is ignored and the output is printed to stdout.
    /// This flag is considered a higher priority than
    /// the output_file flag, so if both are present, the output
    /// will be printed to stdout.
    #[arg(short, long)]
    pub print: bool,
    /// Sample number to extract. Optional.
    #[arg(short = 'n', long)]
    pub sample_number: Option<usize>,
    /// If input and output files are not provided,
    /// then this tells the x-encode, x-decode, and decode modes
    /// that the expected formats are BEN and XBEN
    #[arg(short = 'b', long)]
    pub ben_and_xben: bool,
    /// If input and output files are not provided,
    /// then this tells the x-encode and x-decode modes
    /// that the expected formats are JSONL and XBEN
    #[arg(short = 'J', long)]
    pub jsonl_and_xben: bool,
    /// If the input and output files are not provided,
    /// then this tells the decode mode that the expected
    /// formats are JSONL and BEN
    #[arg(short = 'j', long)]
    pub jsonl_and_ben: bool,
    /// When saving a file in the BEN format, the deault is to have
    /// an assignment vector saved followed by the number of repetitions
    /// of that assignment vector (this is useful for Markov chian methods
    /// like ReCom). This flag will cause the program to forgo the repetition
    /// count and just save all of the assignment vectors as they are encountered.
    /// Equivalent to `--variant standard`. Ignored if `--variant` is set.
    #[arg(short = 'a', long)]
    pub save_all: bool,
    /// BEN variant to use when encoding.
    /// Possible values: standard, mkvchain, twodelta.
    /// Defaults to mkvchain if neither this nor --save-all is given.
    /// Takes precedence over --save-all when both are provided.
    #[arg(short = 't', long, value_enum)]
    pub variant: Option<CliVariant>,
    /// If the output file already exists, this flag
    /// will cause the program to overwrite it without
    /// asking the user for confirmation.
    #[arg(short = 'w', long)]
    pub overwrite: bool,
    /// Enables verbose printing for the CLI. Optional.
    #[arg(short, long)]
    pub verbose: bool,
    /// Suppress in-place progress spinners. Trace logging is unaffected.
    #[arg(short = 'q', long)]
    pub quiet: bool,
    /// Number of threads the XZ encoder may use during x-encode and
    /// xz-compress. Defaults to 1 (single-threaded). Pass an explicit
    /// value to fan compression out across worker threads; values larger
    /// than the host's available parallelism are silently clamped down.
    /// `-1` is a sentinel meaning "use every available core" (sklearn
    /// convention). See also `--xz-block-size`, which controls how much
    /// input each thread gets before it can start compressing.
    #[arg(short = 'c', long, allow_hyphen_values = true)]
    pub n_cpus: Option<i32>,
    /// When running x-encoder, this flag will deterimine the level of compression to use.
    /// By default, the highest level of compression will be used.
    /// Valid values are 0-9, where 0 is no compression and 9 is the highest level of compression.
    #[arg(short = 'l', long)]
    pub compression_level: Option<u32>,
    /// Number of TwoDelta delta frames per columnar chunk in XBEN encoding.
    /// Only affects TwoDelta variant. Larger chunks improve XZ compression.
    /// Default is 10,000.
    #[arg(long)]
    pub chunk_size: Option<usize>,
    /// Per-block size in bytes for the multithreaded XZ encoder.
    /// liblzma needs a non-zero block size to actually fan compression
    /// out across worker threads; smaller blocks scale parallelism better
    /// at a slight compression-ratio cost. Defaults to 16 MiB when
    /// `--n-cpus > 1`, or 0 (liblzma auto, ~192 MiB at preset 9) for
    /// single-thread runs.
    #[arg(long)]
    pub xz_block_size: Option<u64>,
    /// Embed a graph JSON asset alongside the assignment stream and emit
    /// the result as a `.bendl` bundle. The graph is added after the
    /// assignment stream has been fully written. Only applies to the
    /// `encode` and `x-encode` modes.
    #[arg(long)]
    pub graph: Option<PathBuf>,
}
