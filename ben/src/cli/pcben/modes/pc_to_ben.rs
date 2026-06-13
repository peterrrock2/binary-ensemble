//! `pcben --mode pc-to-ben` handler.

use super::super::args::{Args, Mode};
use super::super::paths::resolved_output_path;
use super::super::translate::assignment_encode_ben;

use crate::cli::common::{CliError, CliResult};
use pipe::pipe;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read};

/// Execute the `pc-to-ben` sub-mode.
pub(in crate::cli::pcben) fn run(args: Args) -> CliResult {
    tracing::info!("Converting PCOMPRESS to BEN");

    let mut pcompress_reader: BufReader<Box<dyn Read + Send>> = match args.input_file.as_ref() {
        Some(file) => BufReader::new(Box::new(BufReader::new(File::open(file)?))),
        None => BufReader::new(Box::new(io::stdin())),
    };

    let mut ben_writer: BufWriter<Box<dyn io::Write>> = match resolved_output_path(
        Mode::PcToBen,
        args.input_file.as_deref(),
        args.output_file.as_deref(),
        args.overwrite,
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
    assignment_encode_ben(&mut buf_pipe_reader, &mut ben_writer).map_err(CliError::from)
}
