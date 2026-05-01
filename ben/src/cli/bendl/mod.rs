//! CLI front-end for the `.bendl` bundle container.
//!
//! Exposes four subcommands:
//!
//! - `create`  — wrap a `.ben` / `.xben` assignment stream plus optional
//!   asset files into a finalized `.bendl` bundle.
//! - `inspect` — print the header and directory of a `.bendl` file.
//! - `extract` — copy the embedded stream region or a named asset out
//!   of a bundle to disk.
//! - `append`  — add new asset files to an already-finalized bundle
//!   without rewriting the stream.

mod append;
mod args;
mod create;
mod extract;
mod helpers;
mod inspect;

#[cfg(test)]
mod tests;

use append::run_append;
use args::{Args, Command};
use create::run_create;
use extract::run_extract;
use inspect::run_inspect;

use crate::cli::common::set_verbose;
use clap::Parser;

/// Parse CLI arguments and execute the selected subcommand.
pub fn run() {
    let args = Args::parse();
    set_verbose(args.verbose);

    let result = match args.command {
        Command::Create(a) => run_create(a),
        Command::Inspect(a) => run_inspect(a),
        Command::Extract(a) => run_extract(a),
        Command::Append(a) => run_append(a),
    };

    if let Err(err) = result {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}
