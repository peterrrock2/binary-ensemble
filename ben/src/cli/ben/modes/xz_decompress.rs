//! `ben xz-decompress` handler.

use super::super::args::{Globals, XzDecompressArgs};

use crate::cli::common::{check_overwrite, CliError, CliResult};
use crate::codec::decode::xz_decompress;
use std::fs::File;
use std::io::{BufReader, BufWriter};

/// Execute the `xz-decompress` subcommand.
pub(in crate::cli::ben) fn run(args: XzDecompressArgs, g: &Globals) -> CliResult {
    tracing::info!("Running in xz decompress mode");

    if !args.input_file.ends_with(".xz") {
        return Err(CliError::other(
            "Unsupported file type for xz decompress mode",
        ));
    }

    let output_file_name = match g.output_file.clone() {
        Some(name) => name,
        None => args.input_file[..args.input_file.len() - 3].to_string(),
    };

    check_overwrite(&output_file_name, g.overwrite)?;

    let reader = BufReader::new(File::open(&args.input_file)?);
    let writer = BufWriter::new(File::create(output_file_name)?);

    xz_decompress(reader, writer)?;
    Ok(())
}
