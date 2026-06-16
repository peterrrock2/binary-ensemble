//! `ben decode` handler.

use super::super::args::{DecodeArgs, Globals};
use super::super::paths::{decode_setup, open_derived_writer, open_reader, open_writer};

use crate::cli::common::CliResult;
use crate::codec::decode::{decode_ben_to_jsonl, decode_xben_to_ben};

/// Execute the `decode` subcommand.
pub(in crate::cli::ben) fn run(args: DecodeArgs, g: &Globals) -> CliResult {
    tracing::info!("Running in decode mode");

    // XBEN input decodes one level to BEN; BEN input decodes to JSONL. Auto-detect from the
    // extension, falling back to `--from-xben` (the only signal available for stdin input).
    let from_xben = args.from_xben
        || args
            .input_file
            .as_ref()
            .is_some_and(|f| f.ends_with(".xben"));

    let reader = open_reader(args.input_file.as_deref())?;
    let writer = match args.input_file.as_ref() {
        Some(file) if !g.print => {
            let path = decode_setup(file.clone(), g.output_file.clone(), false, g.overwrite)?;
            open_derived_writer(path)?
        }
        _ => open_writer(g.output_file.as_deref(), g.print, g.overwrite)?,
    };

    if from_xben {
        decode_xben_to_ben(reader, writer)?;
    } else {
        decode_ben_to_jsonl(reader, writer)?;
    }
    Ok(())
}
