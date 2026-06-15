//! `ben pcompress to-ben` handler: PCOMPRESS -> BEN.

use super::super::super::args::{Globals, PcompressIoArgs};
use super::super::paths::{resolved_output_path, PcDirection};
use super::super::translate::assignment_encode_ben;

use crate::cli::common::{CliError, CliResult};
use pipe::pipe;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read};

/// Execute the `to-ben` direction.
pub(in crate::cli::ben::pcompress) fn run(args: PcompressIoArgs, g: &Globals) -> CliResult {
    tracing::info!("Converting PCOMPRESS to BEN");

    let mut pcompress_reader: BufReader<Box<dyn Read + Send>> = match args.input_file.as_ref() {
        Some(file) => BufReader::new(Box::new(BufReader::new(File::open(file)?))),
        None => BufReader::new(Box::new(io::stdin())),
    };

    let mut ben_writer: BufWriter<Box<dyn io::Write>> = match resolved_output_path(
        PcDirection::ToBen,
        args.input_file.as_deref(),
        g.output_file.as_deref(),
        g.overwrite,
    )? {
        Some(file) => BufWriter::new(Box::new(File::create(file)?)),
        None => BufWriter::new(Box::new(io::stdout())),
    };

    let (pipe_reader, pipe_writer) = pipe();
    let mut buf_pipe_writer = BufWriter::new(pipe_writer);

    let decode_thread = std::thread::spawn(move || {
        pcompress::decode::decode(&mut pcompress_reader, &mut buf_pipe_writer, 0, false)
    });

    let mut buf_pipe_reader = BufReader::new(pipe_reader);
    let encode_result = assignment_encode_ben(&mut buf_pipe_reader, &mut ben_writer);

    // `pcompress::decode::decode` panics on some malformed inputs. The pipe reader can still see a
    // clean EOF and emit a prefix BEN stream, so the producer thread must be joined even when the
    // foreground encode appears successful.
    match decode_thread.join() {
        Ok(()) => encode_result.map_err(CliError::from),
        Err(_) => Err(CliError::other("PCOMPRESS-to-BEN decode thread panicked")),
    }
}
