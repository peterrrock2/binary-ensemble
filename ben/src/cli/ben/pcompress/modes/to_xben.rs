//! `ben pcompress to-xben` handler: PCOMPRESS -> XBEN.

use super::super::super::args::{Globals, PcompressIoArgs};
use super::super::paths::{resolved_output_path, PcDirection};
use super::super::translate::assignment_encode_xben;

use crate::cli::common::{CliError, CliResult};
use pipe::pipe;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read};

/// Execute the `to-xben` direction.
pub(in crate::cli::ben::pcompress) fn run(args: PcompressIoArgs, g: &Globals) -> CliResult {
    tracing::info!("Converting PCOMPRESS to XBEN");

    let mut pcompress_reader: BufReader<Box<dyn Read + Send>> = match args.input_file.as_ref() {
        Some(file) => BufReader::new(Box::new(BufReader::new(File::open(file)?))),
        None => BufReader::new(Box::new(io::stdin())),
    };

    let mut ben_writer: BufWriter<Box<dyn io::Write>> = match resolved_output_path(
        PcDirection::ToXben,
        args.input_file.as_deref(),
        g.output_file.as_deref(),
        g.overwrite,
    )? {
        Some(file) => BufWriter::new(Box::new(File::create(file)?)),
        None => BufWriter::new(Box::new(io::stdout())),
    };

    let (pipe_reader, pipe_writer) = pipe();
    let mut buf_pipe_writer = BufWriter::new(pipe_writer);

    let _ = std::thread::spawn(move || {
        pcompress::decode::decode(&mut pcompress_reader, &mut buf_pipe_writer, 0, false)
    });

    let mut buf_pipe_reader = BufReader::new(pipe_reader);
    assignment_encode_xben(&mut buf_pipe_reader, &mut ben_writer).map_err(CliError::from)
}
