//! `pcben` CLI: convert between BEN/XBEN and the foreign PCOMPRESS format.

mod args;
mod modes;
mod paths;
mod translate;

#[cfg(test)]
mod tests;

use args::{Args, Mode};

use crate::cli::common::{set_quiet, set_verbose, CliResult};
use clap::Parser;

/// Parse CLI arguments and execute the selected `pcben` conversion.
pub fn run() -> CliResult {
    let args = Args::parse();
    set_verbose(args.verbose);
    set_quiet(args.quiet);

    match args.mode {
        Mode::BenToPc => modes::ben_to_pc::run(args),
        Mode::PcToBen => modes::pc_to_ben::run(args),
        Mode::PcToXben => modes::pc_to_xben::run(args),
    }
}
