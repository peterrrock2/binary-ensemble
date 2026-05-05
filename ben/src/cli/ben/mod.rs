//! `ben` CLI: encode, decode, and stream-compress BEN/XBEN files.

mod args;
mod bundle;
mod modes;
mod paths;

#[cfg(test)]
mod tests;

use args::{Args, Mode};

use crate::cli::common::{set_quiet, set_verbose, CliError, CliResult};
use clap::Parser;

/// Parse CLI arguments and dispatch to the per-mode handler in [`modes`].
pub fn run() -> CliResult {
    let args = Args::parse();
    set_verbose(args.verbose);
    set_quiet(args.quiet);

    // --graph is only meaningful for the stream-producing modes.
    if args.graph.is_some() && args.mode != Mode::Encode && args.mode != Mode::XEncode {
        return Err(CliError::other(
            "--graph is only supported with --mode encode or --mode x-encode",
        ));
    }

    match args.mode {
        Mode::Encode => modes::encode::run(args),
        Mode::XEncode => modes::xencode::run(args),
        Mode::Decode => modes::decode::run(args),
        Mode::XDecode => modes::xdecode::run(args),
        Mode::Lookup => modes::lookup::run(args),
        Mode::XzCompress => modes::xz_compress::run(args),
        Mode::XzDecompress => modes::xz_decompress::run(args),
    }
}
