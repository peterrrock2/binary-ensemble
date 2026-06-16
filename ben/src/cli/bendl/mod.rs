//! CLI front-end for the `.bendl` file container.
//!
//! Exposes six subcommands:
//!
//! - `create`: wrap a `.ben` / `.xben` assignment stream plus optional asset files into a finalized
//!   `.bendl` file.
//! - `inspect`: print the header and directory of a `.bendl` file.
//! - `extract`: copy the embedded stream region or a named asset out of a bundle to disk.
//! - `append`: add new asset files to an already-finalized bundle without rewriting the stream.
//! - `remove`: drop named assets from a finalized bundle and compact it, so the payload bytes are
//!   actually reclaimed.
//! - `compact`: rewrite a bundle in place without unreferenced byte ranges.

mod append;
mod args;
mod create;
mod extract;
mod helpers;
mod inspect;
mod remove;

#[cfg(test)]
mod tests;

use append::run_append;
use args::{Args, Command};
use create::run_create;
use extract::run_extract;
use inspect::run_inspect;
use remove::{run_compact, run_remove};

use crate::cli::common::{set_quiet, set_verbose, CliError, CliResult};
use clap::Parser;

/// Parse CLI arguments and execute the selected subcommand.
pub fn run() -> CliResult {
    let args = Args::parse();
    set_verbose(args.verbose);
    set_quiet(args.quiet);

    match args.command {
        Command::Create(a) => run_create(a),
        Command::Inspect(a) => run_inspect(a),
        Command::Extract(a) => run_extract(a),
        Command::Append(a) => run_append(a),
        Command::Remove(a) => run_remove(a),
        Command::Compact(a) => run_compact(a),
    }
    .map_err(CliError::from)
}
