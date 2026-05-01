//! `ben --mode xz-decompress` handler.

use super::super::args::Args;

use crate::cli::common::{check_overwrite, CliError, CliResult};
use crate::codec::decode::xz_decompress;
use std::fs::File;
use std::io::{BufReader, BufWriter};

/// Execute the `xz-decompress` sub-mode.
pub(in crate::cli::ben) fn run(args: Args) -> CliResult {
    tracing::trace!("Running in xz decompress mode");

    let in_file_name = args
        .input_file
        .ok_or_else(|| CliError::other("Must provide input file for xz-decompress mode."))?;

    if !in_file_name.ends_with(".xz") {
        return Err(CliError::other(
            "Unsupported file type for xz decompress mode",
        ));
    }

    let output_file_name = match args.output_file {
        Some(name) => name,
        None => in_file_name[..in_file_name.len() - 3].to_string(),
    };

    check_overwrite(&output_file_name, args.overwrite)?;

    let reader = BufReader::new(File::open(&in_file_name)?);
    let writer = BufWriter::new(File::create(output_file_name)?);

    xz_decompress(reader, writer)?;
    Ok(())
}
