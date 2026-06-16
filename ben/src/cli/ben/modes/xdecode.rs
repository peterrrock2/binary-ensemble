//! `ben xdecode` handler.

use super::super::args::{Globals, XdecodeArgs};
use super::super::paths::{decode_setup, open_derived_writer, open_reader, open_writer};

use crate::cli::common::CliResult;
use crate::codec::decode::decode_xben_to_jsonl;

/// Execute the `xdecode` subcommand.
pub(in crate::cli::ben) fn run(args: XdecodeArgs, g: &Globals) -> CliResult {
    tracing::info!("Running in x-decode mode");

    let reader = open_reader(args.input_file.as_deref())?;
    let writer = match args.input_file.as_ref() {
        Some(file) if !g.print => {
            let path = decode_setup(file.clone(), g.output_file.clone(), true, g.overwrite)?;
            open_derived_writer(path)?
        }
        _ => open_writer(g.output_file.as_deref(), g.print, g.overwrite)?,
    };

    decode_xben_to_jsonl(reader, writer)?;
    Ok(())
}
