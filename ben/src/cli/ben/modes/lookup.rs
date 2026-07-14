//! `ben lookup` handler.

use super::super::args::{Globals, LookupArgs, LookupTarget};
use super::super::paths::open_writer;

use crate::cli::common::{CliError, CliResult};
use crate::ops::extract::extract_assignment_ben_seek;
use crate::progress::Spinner;
use std::fs::File;
use std::io::{BufReader, Write};

/// Execute the `lookup` subcommand.
pub(in crate::cli::ben) fn run(args: LookupArgs, g: &Globals) -> CliResult {
    tracing::info!("Running in lookup mode");

    let (input_file, sample_number) = match args.target {
        LookupTarget::Index(args) => (
            args.input_file,
            args.index
                .checked_add(1)
                .ok_or_else(|| CliError::other("lookup index is too large"))?,
        ),
        LookupTarget::Sample(args) => (args.input_file, args.sample_number),
    };

    let reader = BufReader::new(File::open(input_file)?);

    let mut writer = open_writer(g.output_file.as_deref(), g.print, false)?;
    // A single random-access lookup has no meaningful running count; show an indeterminate spinner
    // while the seek-and-replay runs, and clear it before writing the result.
    let vec = {
        let _spinner = Spinner::message("Finding plan");
        extract_assignment_ben_seek(reader, sample_number)
            .map_err(|e| CliError::other(format!("{e}")))?
    };
    writer.write_all(format!("{:?}\n", vec).as_bytes())?;
    Ok(())
}
