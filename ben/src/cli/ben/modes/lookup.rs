//! `ben lookup` handler.

use super::super::args::{Globals, LookupArgs};
use super::super::paths::open_writer;

use crate::cli::common::{CliError, CliResult};
use crate::ops::extract::extract_assignment_ben;
use std::fs::File;
use std::io::{BufReader, Write};

/// Execute the `lookup` subcommand.
pub(in crate::cli::ben) fn run(args: LookupArgs, g: &Globals) -> CliResult {
    tracing::info!("Running in lookup mode");

    let reader = BufReader::new(File::open(&args.input_file)?);

    let mut writer = open_writer(g.output_file.as_deref(), g.print, false)?;
    let vec = extract_assignment_ben(reader, args.sample_number)
        .map_err(|e| CliError::other(format!("{e}")))?;
    writer.write_all(format!("{:?}\n", vec).as_bytes())?;
    Ok(())
}
