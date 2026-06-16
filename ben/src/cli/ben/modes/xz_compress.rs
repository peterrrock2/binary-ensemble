//! `ben xz-compress` handler.

use super::super::args::{Globals, XzCompressArgs};

use crate::cli::common::{check_overwrite, CliResult};
use crate::codec::encode::{cpus_from_signed, xz_compress};
use std::fs::File;
use std::io::{BufReader, BufWriter};

/// Execute the `xz-compress` subcommand.
pub(in crate::cli::ben) fn run(args: XzCompressArgs, g: &Globals) -> CliResult {
    tracing::info!("Running in xz compress mode");

    let reader = BufReader::new(File::open(&args.input_file)?);

    let out_file_name = match g.output_file.clone() {
        Some(name) => name,
        None => args.input_file.clone() + ".xz",
    };

    check_overwrite(&out_file_name, g.overwrite)?;
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
