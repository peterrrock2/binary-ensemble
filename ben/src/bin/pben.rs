use ben::decode::*;
use ben::encode::*;
use ben::{logln, BenVariant};
use clap::{Parser, ValueEnum};
use pcompress;
use pipe::pipe;
use std::{
    fs::File,
    io::{self, BufRead, BufReader, BufWriter, Read, Result, Write},
};
use xz2::write::XzEncoder;

/// Defines the mode of operation.
#[derive(Parser, Debug, Clone, ValueEnum, PartialEq)]
enum Mode {
    BenToPc,
    PcToBen,
    PcToXben,
}

#[derive(Parser, Debug)]
#[command(
    name = "Conversion tool for BEN and PCOMPRESS formats",
    about = "This is a CLI tool that allows for the conversion between BEN and PCOMPRESS formats.",
    version = "0.2.0"
)]
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
}

fn main() -> Result<()> {
    let args = Args::parse();

    if args.verbose {
        std::env::set_var("RUST_LOG", "trace");
    }

    match args.mode {
        Mode::BenToPc => {
            logln!("Converting BEN to PCOMPRESS");

            let ben_reader: Box<dyn Read + Send> = match args.input_file {
                Some(file) => Box::new(BufReader::new(File::open(&file).unwrap())),
                None => Box::new(io::stdin()),
            };

            let mut pcompress_writer: BufWriter<Box<dyn io::Write>> = match args.output_file {
                Some(file) => BufWriter::new(Box::new(File::create(&file).unwrap())),
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
            logln!("Converting PCOMPRESS to BEN");

            let mut pcompress_reader: BufReader<Box<dyn Read + Send>> = match args.input_file {
                Some(file) => BufReader::new(Box::new(BufReader::new(File::open(&file).unwrap()))),
                None => BufReader::new(Box::new(io::stdin())),
            };

            let mut ben_writer: BufWriter<Box<dyn io::Write>> = match args.output_file {
                Some(file) => BufWriter::new(Box::new(File::create(&file).unwrap())),
                None => BufWriter::new(Box::new(io::stdout())),
            };

            let (pipe_reader, pipe_writer) = pipe();

            let mut buf_pipe_writer = BufWriter::new(pipe_writer);

            let _ = std::thread::spawn(move || {
                pcompress::decode::decode(&mut pcompress_reader, &mut buf_pipe_writer, 0, false)
            });

            let mut buf_pipe_reader = BufReader::new(pipe_reader);

            assignment_encode_ben(&mut buf_pipe_reader, &mut ben_writer)
        }
        Mode::PcToXben => {
            logln!("Converting PCOMPRESS to XBEN");

            let mut pcompress_reader: BufReader<Box<dyn Read + Send>> = match args.input_file {
                Some(file) => BufReader::new(Box::new(BufReader::new(File::open(&file).unwrap()))),
                None => BufReader::new(Box::new(io::stdin())),
            };

            let mut ben_writer: BufWriter<Box<dyn io::Write>> = match args.output_file {
                Some(file) => BufWriter::new(Box::new(File::create(&file).unwrap())),
                None => BufWriter::new(Box::new(io::stdout())),
            };

            let (pipe_reader, pipe_writer) = pipe();

            let mut buf_pipe_writer = BufWriter::new(pipe_writer);

            let _ = std::thread::spawn(move || {
                pcompress::decode::decode(&mut pcompress_reader, &mut buf_pipe_writer, 0, false)
            });

            let mut buf_pipe_reader = BufReader::new(pipe_reader);

            assignment_encode_xben(&mut buf_pipe_reader, &mut ben_writer)
        }
    }
}

fn assignment_decode_ben<R: Read, W: Write>(mut reader: R, mut writer: W) -> io::Result<()> {
    let ben_reader = BenDecoder::new(&mut reader)?;

    for result in ben_reader {
        match result {
            Ok(assignment) => {
                write!(writer, "{}\n", serde_json::to_string(&assignment).unwrap())?;
            }
            Err(e) => return Err(e),
        }
    }

    Ok(())
}

fn assignment_encode_ben<R: Read + BufRead, W: Write>(reader: R, writer: W) -> io::Result<()> {
    let mut ben_writer = BenEncoder::new(writer, BenVariant::MkvChain);

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

fn assignment_encode_xben<R: Read + BufRead, W: Write>(reader: R, writer: W) -> io::Result<()> {
    let encoder = XzEncoder::new(writer, 9);
    let mut xben_writer = XBenEncoder::new(encoder, BenVariant::MkvChain);

    xben_writer.write_ben_file(reader)?;
    Ok(())
}
