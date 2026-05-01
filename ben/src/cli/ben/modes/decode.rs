//! `ben --mode decode` handler.

use super::super::args::Args;
use super::super::paths::{decode_setup, open_derived_writer, open_reader, open_writer};

use crate::cli::common::{CliError, CliResult};
use crate::codec::decode::{decode_ben_to_jsonl, decode_xben_to_ben};

/// Execute the `decode` sub-mode.
pub(in crate::cli::ben) fn run(args: Args) -> CliResult {
    tracing::trace!("Running in decode mode");

    let mut ben_and_xben = args.ben_and_xben;
    let mut jsonl_and_ben = args.jsonl_and_ben;

    if let Some(file) = args.input_file.as_ref() {
        if file.ends_with(".ben") {
            jsonl_and_ben = true;
        } else if file.ends_with(".xben") {
            ben_and_xben = true;
        }
    }

    let reader = open_reader(args.input_file.as_deref());
    let writer = match args.input_file.as_ref() {
        Some(file) if !args.print => {
            let path = decode_setup(
                file.clone(),
                args.output_file.clone(),
                false,
                args.overwrite,
            )?;
            open_derived_writer(path)
        }
        _ => open_writer(args.output_file.as_deref(), args.print, args.overwrite)?,
    };

    if ben_and_xben {
        decode_xben_to_ben(reader, writer)?;
        Ok(())
    } else if jsonl_and_ben {
        decode_ben_to_jsonl(reader, writer)?;
        Ok(())
    } else {
        Err(CliError::other("Unsupported file type(s) for decode mode"))
    }
}
