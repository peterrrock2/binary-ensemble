//! `ben --mode xz-compress` handler.

use super::super::args::Args;

use crate::cli::common::{check_overwrite, CliError, CliResult};
use crate::codec::encode::{cpus_from_signed, xz_compress};
use std::fs::File;
use std::io::{BufReader, BufWriter};

/// Execute the `xz-compress` sub-mode.
pub(in crate::cli::ben) fn run(args: Args) -> CliResult {
    tracing::info!("Running in xz compress mode");

    let in_file_name = args
        .input_file
        .ok_or_else(|| CliError::other("Must provide input file for xz-compress mode."))?;
    let reader = BufReader::new(File::open(&in_file_name)?);

    let out_file_name = match args.output_file {
        Some(name) => name,
        None => in_file_name + ".xz",
    };

    check_overwrite(&out_file_name, args.overwrite)?;
    let writer = BufWriter::new(File::create(out_file_name)?);

    xz_compress(
        reader,
        writer,
        args.n_cpus.map(cpus_from_signed),
        args.compression_level,
        args.xz_block_size,
    )?;
    tracing::trace!("Done!");
    Ok(())
}
