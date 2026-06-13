//! `ben --mode read` handler.

use super::super::args::Args;
use super::super::paths::open_writer;

use crate::cli::common::{CliError, CliResult};
use crate::ops::extract::extract_assignment_ben;
use std::fs::File;
use std::io::{BufReader, Write};

/// Execute the `lookup` sub-mode.
pub(in crate::cli::ben) fn run(args: Args) -> CliResult {
    tracing::info!("Running in lookup mode");

    let in_file = args
        .input_file
        .ok_or_else(|| CliError::other("Must provide input file for lookup mode."))?;
    let reader = BufReader::new(File::open(&in_file)?);

    let sample_number = args
        .sample_number
        .ok_or_else(|| CliError::other("Sample number is required in lookup mode"))?;

    let mut writer = open_writer(args.output_file.as_deref(), args.print, false)?;
    let vec = extract_assignment_ben(reader, sample_number)
        .map_err(|e| CliError::other(format!("{e}")))?;
    writer.write_all(format!("{:?}\n", vec).as_bytes())?;
    Ok(())
}
