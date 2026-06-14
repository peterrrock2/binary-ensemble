//! `ben` CLI: encode, decode, relabel, and convert binary ensemble files.

mod args;
mod bundle;
mod canonicalize;
mod modes;
mod paths;
mod pcompress;
mod reencode;
mod relabel;
mod relabel_helpers;
mod sort_graph;

#[cfg(test)]
mod tests;

use args::{Cli, Command};

use crate::cli::common::{set_quiet, set_verbose, CliError, CliResult};
use clap::Parser;

/// Parse CLI arguments and dispatch to the handler for the selected subcommand.
pub fn run() -> CliResult {
    let Cli { globals, command } = Cli::parse();
    set_verbose(globals.verbose);
    set_quiet(globals.quiet);
    let g = &globals;

    match command {
        Command::Encode(a) => modes::encode::run(a, g),
        Command::Xencode(a) => modes::xencode::run(a, g),
        Command::Decode(a) => modes::decode::run(a, g),
        Command::Xdecode(a) => modes::xdecode::run(a, g),
        Command::Lookup(a) => modes::lookup::run(a, g),
        Command::XzCompress(a) => modes::xz_compress::run(a, g),
        Command::XzDecompress(a) => modes::xz_decompress::run(a, g),
        Command::Relabel(a) => relabel::run(a, g).map_err(CliError::from),
        Command::Canonicalize(a) => canonicalize::run(a, g).map_err(CliError::from),
        Command::Reencode(a) => reencode::run(a, g).map_err(CliError::from),
        Command::SortGraph(a) => sort_graph::run(a, g).map_err(CliError::from),
        Command::Pcompress(a) => pcompress::run(a, g),
    }
}
