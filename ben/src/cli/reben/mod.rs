//! `reben` CLI: relabel and canonicalize BEN files using JSON dual-graph orderings.

mod args;
mod ben_mode;
mod helpers;
mod json_mode;

#[cfg(test)]
mod tests;

use args::{Args, Mode};
use ben_mode::run_ben_mode;
use json_mode::run_json_mode;

use crate::cli::common::{set_quiet, set_verbose, CliError, CliResult};
use clap::Parser;

/// Parse CLI arguments and execute the selected `reben` mode.
pub fn run() -> CliResult {
    let args = Args::parse();
    set_verbose(args.verbose);
    set_quiet(args.quiet);
    run_with_args(args).map_err(CliError::from)
}

fn run_with_args(args: Args) -> Result<(), String> {
    match args.mode.clone() {
        Mode::Json => run_json_mode(args),
        Mode::Ben => run_ben_mode(args),
    }
}
