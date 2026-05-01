use crate::cli::common::{check_overwrite, set_verbose};
use crate::io::reader::AssignmentReader;
use crate::io::writer::{AssignmentWriter, XZAssignmentWriter};
use crate::BenVariant;
use clap::{Parser, ValueEnum};
use pipe::pipe;
use serde_json::json;
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

/// Parse CLI arguments and execute the selected `pben` conversion.
pub fn run() -> Result<()> {
    let args = Args::parse();
    set_verbose(args.verbose);

    match args.mode {
        Mode::BenToPc => {
            tracing::trace!("Converting BEN to PCOMPRESS");

            let ben_reader: Box<dyn Read + Send> = match args.input_file.as_ref() {
                Some(file) => Box::new(BufReader::new(File::open(file).unwrap())),
                None => Box::new(io::stdin()),
            };

            let mut pcompress_writer: BufWriter<Box<dyn io::Write>> = match resolved_output_path(
                Mode::BenToPc,
                args.input_file.as_deref(),
                args.output_file.as_deref(),
                args.overwrite,
            )? {
                Some(file) => BufWriter::new(Box::new(File::create(file).unwrap())),
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
                Some(file) => BufReader::new(Box::new(BufReader::new(File::open(file).unwrap()))),
                None => BufReader::new(Box::new(io::stdin())),
            };

            let mut ben_writer: BufWriter<Box<dyn io::Write>> = match resolved_output_path(
                Mode::PcToBen,
                args.input_file.as_deref(),
                args.output_file.as_deref(),
                args.overwrite,
            )? {
                Some(file) => BufWriter::new(Box::new(File::create(file).unwrap())),
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

            let mut pcompress_reader: BufReader<Box<dyn Read + Send>> = match args
                .input_file
                .as_ref()
            {
                Some(file) => BufReader::new(Box::new(BufReader::new(File::open(file).unwrap()))),
                None => BufReader::new(Box::new(io::stdin())),
            };

            let mut ben_writer: BufWriter<Box<dyn io::Write>> = match resolved_output_path(
                Mode::PcToXben,
                args.input_file.as_deref(),
                args.output_file.as_deref(),
                args.overwrite,
            )? {
                Some(file) => BufWriter::new(Box::new(File::create(file).unwrap())),
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

/// Resolve the output file path for a `pben` mode.
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

/// Derive the default output file name for a `pben` conversion mode.
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
    fn derive_output_path_replaces_expected_suffixes() {
        assert_eq!(
            derive_output_path(Mode::BenToPc, "plans.ben"),
            "plans.pcompress"
        );
        assert_eq!(
            derive_output_path(Mode::PcToBen, "plans.pcompress"),
            "plans.ben"
        );
        assert_eq!(derive_output_path(Mode::PcToXben, "plans.pc"), "plans.xben");
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
    fn resolved_output_path_returns_none_when_both_paths_absent() {
        // When neither output_file nor input_file is given, stdout mode: Ok(None).
        let result = resolved_output_path(Mode::BenToPc, None, None, false).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn assignment_decode_ben_propagates_read_error() {
        // assignment_decode_ben propagates I/O errors from the BEN reader.
        struct AlwaysErrors;
        impl io::Read for AlwaysErrors {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "broken"))
            }
        }
        let mut out = Vec::new();
        let err = assignment_decode_ben(AlwaysErrors, &mut out).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
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

    #[test]
    fn assignment_decode_ben_iterator_error_propagates() {
        // Provides a valid BEN banner so AssignmentReader::new succeeds,
        // then returns a non-EOF error on the next read so the iterator
        // fires the Err(e) => return Err(e) arm (line 204).
        use std::io::Read;
        use crate::format::banners::STANDARD_BEN_BANNER;

        struct BannerThenError {
            banner: &'static [u8],
            pos: usize,
        }
        impl Read for BannerThenError {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                if self.pos < self.banner.len() {
                    let n = buf.len().min(self.banner.len() - self.pos);
                    buf[..n].copy_from_slice(&self.banner[self.pos..self.pos + n]);
                    self.pos += n;
                    Ok(n)
                } else {
                    Err(io::Error::new(io::ErrorKind::BrokenPipe, "broken"))
                }
            }
        }

        let reader = BannerThenError { banner: STANDARD_BEN_BANNER, pos: 0 };
        let mut out = Vec::new();
        let err = assignment_decode_ben(reader, &mut out).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    }
}
