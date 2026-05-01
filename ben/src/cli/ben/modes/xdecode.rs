//! `ben --mode x-decode` handler.

use super::super::args::Args;
use super::super::paths::{decode_setup, open_derived_writer, open_reader, open_writer};

use crate::cli::common::CliResult;
use crate::codec::decode::decode_xben_to_jsonl;

/// Execute the `x-decode` sub-mode.
pub(in crate::cli::ben) fn run(args: Args) -> CliResult {
    tracing::trace!("Running in x-decode mode");

    let reader = open_reader(args.input_file.as_deref());
    let writer = match args.input_file.as_ref() {
        Some(file) if !args.print => {
            let path =
                decode_setup(file.clone(), args.output_file.clone(), true, args.overwrite)?;
            open_derived_writer(path)
        }
        _ => open_writer(args.output_file.as_deref(), args.print, args.overwrite)?,
    };

    decode_xben_to_jsonl(reader, writer)?;
    Ok(())
}
