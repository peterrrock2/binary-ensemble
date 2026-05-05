use crate::cli::common::{check_overwrite, set_quiet, set_verbose, CliError, CliResult};
use crate::io::reader::AssignmentReader;
use crate::io::writer::{AssignmentWriter, XZAssignmentWriter};
use crate::BenVariant;
use clap::{Parser, ValueEnum};
use pipe::pipe;
use serde_json::json;
use std::{
    fs::File,
    io::{self, BufRead, BufReader, BufWriter, Read, Write},
};
use xz2::write::XzEncoder;

#[derive(Parser, Debug, Clone, ValueEnum, PartialEq)]
/// Defines the mode of operation.
enum Mode {
    /// Convert BEN into PCOMPRESS.
    BenToPc,
    /// Convert PCOMPRESS into BEN.
    PcToBen,
    /// Convert PCOMPRESS into XBEN.
    PcToXben,
}

#[derive(Parser, Debug)]
#[command(
    name = "Conversion tool for BEN and PCOMPRESS formats",
    about = "This is a CLI tool that allows for the conversion between BEN and PCOMPRESS formats.",
    version
)]
/// Defines the command line arguments accepted by the program.
struct Args {
    /// Mode to run the program in
    #[arg(short, long, value_enum)]
    mode: Mode,
    /// Input file to read from.
    #[arg(short, long)]
    input_file: Option<String>,
    /// Output file to write to. Optional.
    /// If not provided, the output file will be determined
    /// based on the input file and the mode of operation.
    #[arg(short, long)]
    output_file: Option<String>,
    /// If the output file already exists, this flag
    /// will cause the program to overwrite it without
    /// asking the user for confirmation.
    #[arg(short = 'w', long)]
    overwrite: bool,
    /// Enables verbose printing for the CLI. Optional.
    #[arg(short, long)]
    verbose: bool,
    /// Suppress in-place progress spinners. Trace logging is unaffected.
    #[arg(short = 'q', long)]
    quiet: bool,
}

/// Parse CLI arguments and execute the selected `pcben` conversion.
pub fn run() -> CliResult {
    let args = Args::parse();
    set_verbose(args.verbose);
    set_quiet(args.quiet);

    match args.mode {
        Mode::BenToPc => {
            tracing::trace!("Converting BEN to PCOMPRESS");

            let ben_reader: Box<dyn Read + Send> = match args.input_file.as_ref() {
                Some(file) => Box::new(BufReader::new(File::open(file)?)),
                None => Box::new(io::stdin()),
            };

            let mut pcompress_writer: BufWriter<Box<dyn io::Write>> = match resolved_output_path(
                Mode::BenToPc,
                args.input_file.as_deref(),
                args.output_file.as_deref(),
                args.overwrite,
            )? {
                Some(file) => BufWriter::new(Box::new(File::create(file)?)),
                None => BufWriter::new(Box::new(io::stdout())),
            };

            let (pipe_reader, pipe_writer) = pipe();

            let _ = std::thread::spawn(move || -> io::Result<()> {
                assignment_decode_ben(ben_reader, pipe_writer)
            });

            let mut buf_pipe_reader = BufReader::new(pipe_reader);
            pcompress::encode::encode(&mut buf_pipe_reader, &mut pcompress_writer, false);
            Ok(())
        }
        Mode::PcToBen => {
            tracing::trace!("Converting PCOMPRESS to BEN");

            let mut pcompress_reader: BufReader<Box<dyn Read + Send>> = match args
                .input_file
                .as_ref()
            {
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
        Mode::PcToXben => {
            tracing::trace!("Converting PCOMPRESS to XBEN");

            let mut pcompress_reader: BufReader<Box<dyn Read + Send>> = match args
                .input_file
                .as_ref()
            {
                Some(file) => BufReader::new(Box::new(BufReader::new(File::open(file)?))),
                None => BufReader::new(Box::new(io::stdin())),
            };

            let mut ben_writer: BufWriter<Box<dyn io::Write>> = match resolved_output_path(
                Mode::PcToXben,
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
            assignment_encode_xben(&mut buf_pipe_reader, &mut ben_writer).map_err(CliError::from)
        }
    }
}

/// Resolve the output file path for a `pcben` mode.
fn resolved_output_path(
    mode: Mode,
    input_file: Option<&str>,
    output_file: Option<&str>,
    overwrite: bool,
) -> io::Result<Option<String>> {
    let Some(path) = output_file
        .map(ToOwned::to_owned)
        .or_else(|| input_file.map(|input| derive_output_path(mode, input)))
    else {
        return Ok(None);
    };

    check_overwrite(&path, overwrite)?;
    Ok(Some(path))
}

/// Derive the default output file name for a `pcben` conversion mode.
fn derive_output_path(mode: Mode, input_file: &str) -> String {
    match mode {
        Mode::BenToPc => input_file
            .strip_suffix(".ben")
            .map(|prefix| format!("{prefix}.pcompress"))
            .unwrap_or_else(|| format!("{input_file}.pcompress")),
        Mode::PcToBen => input_file
            .strip_suffix(".pcompress")
            .or_else(|| input_file.strip_suffix(".pc"))
            .map(|prefix| format!("{prefix}.ben"))
            .unwrap_or_else(|| format!("{input_file}.ben")),
        Mode::PcToXben => input_file
            .strip_suffix(".pcompress")
            .or_else(|| input_file.strip_suffix(".pc"))
            .map(|prefix| format!("{prefix}.xben"))
            .unwrap_or_else(|| format!("{input_file}.xben")),
    }
}

/// Decode BEN and emit one zero-based assignment vector per line for PCOMPRESS.
fn assignment_decode_ben<R: Read, W: Write>(mut reader: R, mut writer: W) -> io::Result<()> {
    let ben_reader = AssignmentReader::new(&mut reader)?;
    let mut line = String::new();

    for result in ben_reader {
        match result {
            Ok((assignment, count)) => {
                render_zero_based_assignment_line(&assignment, &mut line);
                for _ in 0..count {
                    writeln!(writer, "{line}")?;
                }
            }
            Err(e) => return Err(e),
        }
    }

    Ok(())
}

/// Render a BEN assignment vector as a zero-based JSON array for PCOMPRESS.
fn render_zero_based_assignment_line(assignment: &[u16], output: &mut String) {
    output.clear();
    output.push('[');
    for (idx, value) in assignment.iter().enumerate() {
        if idx > 0 {
            output.push(',');
        }
        output.push_str(&value.saturating_sub(1).to_string());
    }
    output.push(']');
}

/// Read zero-based assignment vectors and encode them as BEN.
fn assignment_encode_ben<R: Read + BufRead, W: Write>(reader: R, writer: W) -> io::Result<()> {
    let mut ben_writer = AssignmentWriter::new(writer, BenVariant::MkvChain)?;

    for line in reader.lines() {
        let assignment: Vec<u16> = serde_json::from_str::<Vec<u16>>(&line.unwrap())
            .unwrap()
            .into_iter()
            .map(|x| x as u16 + 1)
            .collect();
        ben_writer.write_assignment(assignment)?;
    }
    Ok(())
}

/// Read zero-based assignment vectors and encode them as XBEN.
fn assignment_encode_xben<R: Read + BufRead, W: Write>(reader: R, writer: W) -> io::Result<()> {
    let encoder = XzEncoder::new(writer, 9);
    let mut xben_writer = XZAssignmentWriter::new(encoder, BenVariant::MkvChain)?;

    for line in reader.lines() {
        let assignment: Vec<u16> = serde_json::from_str::<Vec<u16>>(&line.unwrap())
            .unwrap()
            .into_iter()
            .map(|x| x as u16 + 1)
            .collect();
        xben_writer.write_json_value(json!({ "assignment": assignment }))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests;
