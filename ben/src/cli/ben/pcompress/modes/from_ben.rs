//! `ben pcompress from-ben` handler: BEN -> PCOMPRESS.

use super::super::super::args::{Globals, PcompressIoArgs};
use super::super::paths::{resolved_output_path, PcDirection};
use super::super::translate::assignment_decode_ben;

use crate::cli::common::{CliError, CliResult};
use pipe::pipe;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read};

/// Execute the `from-ben` direction.
pub(in crate::cli::ben::pcompress) fn run(args: PcompressIoArgs, g: &Globals) -> CliResult {
    tracing::info!("Converting BEN to PCOMPRESS");

    let ben_reader: Box<dyn Read + Send> = match args.input_file.as_ref() {
        Some(file) => Box::new(BufReader::new(File::open(file)?)),
        None => Box::new(io::stdin()),
    };

    let mut pcompress_writer: BufWriter<Box<dyn io::Write>> = match resolved_output_path(
        PcDirection::FromBen,
        args.input_file.as_deref(),
        g.output_file.as_deref(),
        g.overwrite,
    )? {
        Some(file) => BufWriter::new(Box::new(File::create(file)?)),
        None => BufWriter::new(Box::new(io::stdout())),
    };

    let (pipe_reader, pipe_writer) = pipe();

    let decode_thread = std::thread::spawn(move || assignment_decode_ben(ben_reader, pipe_writer));

    let mut buf_pipe_reader = BufReader::new(pipe_reader);
    pcompress::encode::encode(&mut buf_pipe_reader, &mut pcompress_writer, false);

    // Surface a translation failure (e.g. malformed BEN input) rather than letting the closed pipe
    // end the encode quietly with a truncated output file.
    match decode_thread.join() {
        Ok(result) => result.map_err(CliError::from),
        Err(_) => Err(CliError::other("BEN-to-PCOMPRESS decode thread panicked")),
    }
}
