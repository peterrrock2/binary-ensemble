//! `pcben` CLI argument definitions.

use clap::{Parser, ValueEnum};

#[derive(Parser, Debug, Clone, ValueEnum, PartialEq)]
/// Defines the mode of operation.
pub(super) enum Mode {
    /// Convert BEN into PCOMPRESS.
    BenToPc,
    /// Convert PCOMPRESS into BEN.
    PcToBen,
    /// Convert PCOMPRESS into XBEN.
    PcToXben,
}

#[derive(Parser, Debug)]
#[command(
    name = "Conversion tool for BEN and PCOMPRESS formats",
    about = "This is a CLI tool that allows for the conversion between BEN and PCOMPRESS formats.",
    version
)]
/// Defines the command line arguments accepted by the program.
pub(super) struct Args {
    /// Mode to run the program in
    #[arg(short, long, value_enum)]
    pub(super) mode: Mode,
    /// Input file to read from.
    #[arg(short, long)]
    pub(super) input_file: Option<String>,
    /// Output file to write to. Optional.
    /// If not provided, the output file will be determined
    /// based on the input file and the mode of operation.
    #[arg(short, long)]
    pub(super) output_file: Option<String>,
    /// If the output file already exists, this flag
    /// will cause the program to overwrite it without
    /// asking the user for confirmation.
    #[arg(short = 'w', long)]
    pub(super) overwrite: bool,
    /// Enables verbose printing for the CLI. Optional.
    #[arg(short, long)]
    pub(super) verbose: bool,
    /// Suppress in-place progress spinners. Trace logging is unaffected.
    #[arg(short = 'q', long)]
    pub(super) quiet: bool,
}
