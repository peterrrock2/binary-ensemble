use crate::cli::common::set_verbose;
use crate::io::reader::BenDecoder;
use crate::io::writer::{BenEncoder, XBenEncoder};
use crate::BenVariant;
use clap::{Parser, ValueEnum};
use serde_json::json;
use pipe::pipe;
use std::{
    fs::File,
    io::{self, BufRead, BufReader, BufWriter, Read, Result, Write},
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
}

pub fn run() -> Result<()> {
    let args = Args::parse();
    set_verbose(args.verbose);

    match args.mode {
        Mode::BenToPc => {
            tracing::trace!("Converting BEN to PCOMPRESS");

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
            tracing::trace!("Converting PCOMPRESS to BEN");

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
            tracing::trace!("Converting PCOMPRESS to XBEN");

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
            Ok((assignment, count)) => {
                let assignment: Vec<usize> = assignment
                    .into_iter()
                    .map(|x| x.saturating_sub(1) as usize)
                    .collect();
                let line = serde_json::to_string(&assignment).unwrap();
                for _ in 0..count {
                    writeln!(writer, "{line}")?;
                }
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
mod tests {
    use super::*;
    use crate::codec::decode::{decode_ben_to_jsonl, decode_xben_to_jsonl};
    use crate::codec::encode::encode_jsonl_to_ben;
    use clap::{CommandFactory, Parser};
    use std::io::{BufReader, Cursor};

    #[test]
    fn clap_metadata_uses_package_version() {
        let mut command = Args::command();
        let help = command.render_long_help().to_string();

        assert_eq!(command.get_version(), Some(env!("CARGO_PKG_VERSION")));
        assert!(help.contains("PCOMPRESS"));
        assert!(help.contains("--mode"));
    }

    #[test]
    fn parse_pc_to_xben_args() {
        let args = Args::try_parse_from([
            "pben",
            "--mode",
            "pc-to-xben",
            "--input-file",
            "input.pc",
            "--output-file",
            "output.xben",
            "--verbose",
        ])
        .unwrap();

        assert_eq!(args.mode, Mode::PcToXben);
        assert_eq!(args.input_file.as_deref(), Some("input.pc"));
        assert_eq!(args.output_file.as_deref(), Some("output.xben"));
        assert!(args.verbose);
    }

    #[test]
    fn assignment_decode_ben_writes_json_lines() {
        let jsonl = br#"{"assignment":[1,1,2],"sample":1}
{"assignment":[2,3,3],"sample":2}
"#;
        let mut ben = Vec::new();
        encode_jsonl_to_ben(BufReader::new(&jsonl[..]), &mut ben, BenVariant::Standard).unwrap();

        let mut out = Vec::new();
        assignment_decode_ben(Cursor::new(ben), &mut out).unwrap();

        assert_eq!(String::from_utf8(out).unwrap(), "[0,0,1]\n[1,2,2]\n");
    }

    #[test]
    fn assignment_encode_ben_offsets_values_and_writes_ben() {
        let input = b"[0,0,1]\n[1,1,2]\n";
        let mut ben = Vec::new();
        assignment_encode_ben(BufReader::new(&input[..]), &mut ben).unwrap();

        let mut out = Vec::new();
        decode_ben_to_jsonl(Cursor::new(ben), &mut out).unwrap();

        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains(r#""assignment":[1,1,2]"#));
        assert!(rendered.contains(r#""assignment":[2,2,3]"#));
    }

    #[test]
    fn assignment_encode_xben_offsets_values_and_writes_xben() {
        let input = b"[0,1,1]\n[2,2,0]\n";

        let mut xben = Vec::new();
        assignment_encode_xben(BufReader::new(&input[..]), &mut xben).unwrap();

        let mut out = Vec::new();
        decode_xben_to_jsonl(Cursor::new(xben), &mut out).unwrap();

        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains(r#""assignment":[1,2,2]"#));
        assert!(rendered.contains(r#""assignment":[3,3,1]"#));
    }
}
