use crate::cli::common::{check_overwrite, set_verbose};
use crate::codec::decode::{
    decode_ben_to_jsonl, decode_xben_to_ben, decode_xben_to_jsonl, xz_decompress,
};
use crate::codec::encode::{
    encode_ben_to_xben, encode_jsonl_to_ben, encode_jsonl_to_xben, xz_compress,
};
use crate::ops::extract::extract_assignment_ben;
use crate::BenVariant;
use clap::{Parser, ValueEnum};
use std::{
    fs::File,
    io::{self, BufReader, BufWriter, Result, Write},
};

type DynReader = Box<dyn io::BufRead>;
type DynWriter = Box<dyn Write>;

#[derive(Parser, Debug, Clone, ValueEnum, PartialEq)]
/// Defines the mode of operation.
enum Mode {
    /// Encode JSONL into BEN.
    Encode,
    /// Encode JSONL or BEN into XBEN.
    XEncode,
    /// Decode BEN or XBEN into its less compressed representation.
    Decode,
    /// Fully decode XBEN into JSONL.
    XDecode,
    /// Read a single sample from a BEN file.
    Read,
    /// Compress an arbitrary stream with XZ.
    XzCompress,
    /// Decompress an `.xz` file.
    XzDecompress,
}

#[derive(Parser, Debug)]
#[command(
    name = "Binary Ensemble CLI Tool",
    about = "This is a command line tool for encoding and decoding binary ensemble files.",
    version
)]
/// Defines the command line arguments accepted by the program.
struct Args {
    /// Mode to run the program in (encode, decode, or read).
    #[arg(short, long, value_enum)]
    mode: Mode,
    /// Input file to read from.
    #[arg()]
    input_file: Option<String>,
    /// Output file to write to. Optional.
    /// If not provided, the output file will be determined
    /// based on the input file and the mode of operation.
    #[arg(short, long)]
    output_file: Option<String>,
    /// The standard behaviour is to try and derive the output file
    /// name from the input file name. If this flag is set, then this
    /// logic is ignored and the output is printed to stdout.
    /// This flag is considered a higher priority than
    /// the output_file flag, so if both are present, the output
    /// will be printed to stdout.
    #[arg(short, long)]
    print: bool,
    /// Sample number to extract. Optional.
    #[arg(short = 'n', long)]
    sample_number: Option<usize>,
    /// If input and output files are not provided,
    /// then this tells the x-encode, x-decode, and decode modes
    /// that the expected formats are BEN and XBEN
    #[arg(short = 'b', long)]
    ben_and_xben: bool,
    /// If input and output files are not provided,
    /// then this tells the x-encode and x-decode modes
    /// that the expected formats are JSONL and XBEN
    #[arg(short = 'J', long)]
    jsonl_and_xben: bool,
    /// If the input and output files are not provided,
    /// then this tells the decode mode that the expected
    /// formats are JSONL and BEN
    #[arg(short = 'j', long)]
    jsonl_and_ben: bool,
    /// When saving a file in the BEN format, the deault is to have
    /// an assignment vector saved followed by the number of repetitions
    /// of that assignment vector (this is useful for Markov chian methods
    /// like ReCom). This flag will cause the program to forgo the repetition
    /// count and just save all of the assignment vectors as they are encountered.
    #[arg(short = 'a', long)]
    save_all: bool,
    /// If the output file already exists, this flag
    /// will cause the program to overwrite it without
    /// asking the user for confirmation.
    #[arg(short = 'w', long)]
    overwrite: bool,
    /// Enables verbose printing for the CLI. Optional.
    #[arg(short, long)]
    verbose: bool,
    /// When running x-encoder, this flag will determine the number of cpus to use on the
    /// system. By default, all available cpus will be used.
    #[arg(short = 'c', long)]
    n_cpus: Option<u32>,
    /// When running x-encoder, this flag will deterimine the level of compression to use.
    /// By default, the highest level of compression will be used.
    /// Valid values are 0-9, where 0 is no compression and 9 is the highest level of compression.
    #[arg(short = 'l', long)]
    compression_level: Option<u32>,
    /// Number of TwoDelta delta frames per columnar chunk in XBEN encoding.
    /// Only affects TwoDelta variant. Larger chunks improve XZ compression.
    /// Default is 10,000.
    #[arg(long)]
    chunk_size: Option<usize>,
}

/// Derive the output path for encode-style CLI modes.
///
/// # Arguments
///
/// * `mode` - The encode-oriented CLI mode being executed.
/// * `input_file_name` - The input file path supplied by the user.
/// * `output_file_name` - An optional explicit output path.
/// * `overwrite` - Whether to skip overwrite prompting.
///
/// # Returns
///
/// Returns the resolved output path.
fn encode_setup(
    mode: Mode,
    input_file_name: String,
    output_file_name: Option<String>,
    overwrite: bool,
) -> Result<String> {
    let extension = if mode == Mode::XEncode {
        ".xben"
    } else if mode == Mode::Encode {
        ".ben"
    } else {
        ".xz"
    };

    let out_file_name = match output_file_name {
        Some(name) => name.to_owned(),
        None => {
            if input_file_name.ends_with(".ben") && extension == ".xben" {
                input_file_name.trim_end_matches(".ben").to_owned() + extension
            } else {
                input_file_name.to_string() + extension
            }
        }
    };

    check_overwrite(&out_file_name, overwrite)?;
    Ok(out_file_name)
}

/// Derive the output path for decode-style CLI modes.
///
/// # Arguments
///
/// * `in_file_name` - The input file path supplied by the user.
/// * `out_file_name` - An optional explicit output path.
/// * `full_decode` - Whether the decode should go all the way to JSONL instead
///   of stopping at BEN.
/// * `overwrite` - Whether to skip overwrite prompting.
///
/// # Returns
///
/// Returns the resolved output path.
fn decode_setup(
    in_file_name: String,
    out_file_name: Option<String>,
    full_decode: bool,
    overwrite: bool,
) -> Result<String> {
    let out_file_name = if let Some(name) = out_file_name {
        name.to_owned()
    } else if in_file_name.ends_with(".ben") {
        in_file_name.trim_end_matches(".ben").to_owned()
    } else if in_file_name.ends_with(".xben") {
        if !full_decode {
            in_file_name.trim_end_matches(".xben").to_owned() + ".ben"
        } else {
            in_file_name.trim_end_matches(".xben").to_owned()
        }
    } else if in_file_name.ends_with(".xz") {
        eprintln!(
            "Error: Unsupported file type for decode mode {:?}. Please decompress xz files with \
            either the xz command line tool or the xz-decompress mode of this tool.",
            in_file_name
        );
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
    } else {
        eprintln!(
            "Error: Unsupported file type for decode mode {:?}. Supported types are .ben and .xben.",
            in_file_name
        );
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
    };

    check_overwrite(&out_file_name, overwrite)?;
    Ok(out_file_name)
}

/// Open either the requested input file or stdin.
///
/// # Arguments
///
/// * `input_file` - An optional input file path.
///
/// # Returns
///
/// Returns a buffered reader for the requested file or stdin.
fn open_reader(input_file: Option<&str>) -> DynReader {
    match input_file {
        Some(path) => Box::new(BufReader::new(File::open(path).unwrap())),
        None => Box::new(BufReader::new(io::stdin())),
    }
}

/// Open either the requested output file or stdout.
///
/// # Arguments
///
/// * `output_file` - An optional output file path.
/// * `print` - Whether output should be forced to stdout.
/// * `overwrite` - Whether to skip overwrite prompting for file outputs.
///
/// # Returns
///
/// Returns a buffered writer for the requested file or stdout.
fn open_writer(output_file: Option<&str>, print: bool, overwrite: bool) -> Result<DynWriter> {
    if print {
        return Ok(Box::new(BufWriter::new(io::stdout())));
    }

    match output_file {
        Some(path) => {
            check_overwrite(path, overwrite)?;
            Ok(Box::new(BufWriter::new(File::create(path).unwrap())))
        }
        None => Ok(Box::new(BufWriter::new(io::stdout()))),
    }
}

/// Open a writer for a path computed by one of the setup helpers.
///
/// # Arguments
///
/// * `path` - The output path to create.
///
/// # Returns
///
/// Returns a buffered writer for `path`.
fn open_derived_writer(path: String) -> DynWriter {
    Box::new(BufWriter::new(File::create(path).unwrap()))
}

/// Parse CLI arguments and execute the selected `ben` sub-mode.
pub fn run() {
    let args = Args::parse();
    set_verbose(args.verbose);

    match args.mode {
        Mode::Encode => {
            tracing::trace!("Running in encode mode");

            let reader = open_reader(args.input_file.as_deref());
            let writer = match args.input_file.as_ref() {
                Some(in_file) if !args.print => match encode_setup(
                    args.mode,
                    in_file.clone(),
                    args.output_file.clone(),
                    args.overwrite,
                ) {
                    Ok(path) => open_derived_writer(path),
                    Err(err) => {
                        eprintln!("Error: {:?}", err);
                        return;
                    }
                },
                _ => match open_writer(args.output_file.as_deref(), args.print, args.overwrite) {
                    Ok(writer) => writer,
                    Err(err) => {
                        eprintln!("Error: {:?}", err);
                        return;
                    }
                },
            };

            let possible_error = if args.save_all {
                encode_jsonl_to_ben(reader, writer, BenVariant::Standard)
            } else {
                encode_jsonl_to_ben(reader, writer, BenVariant::MkvChain)
            };

            if let Err(err) = possible_error {
                eprintln!("Error: {:?}", err);
            }
        }
        Mode::XEncode => {
            tracing::trace!("Running in xencode mode");

            let mut ben_and_xben = args.ben_and_xben;
            let mut jsonl_and_xben = args.jsonl_and_xben;

            if let Some(in_file) = args.input_file.as_ref() {
                if in_file.ends_with(".ben") {
                    ben_and_xben = true;
                } else if in_file.ends_with(".jsonl") {
                    jsonl_and_xben = true;
                }
            }

            let reader = open_reader(args.input_file.as_deref());
            let writer = match args.input_file.as_ref() {
                Some(in_file) if !args.print => match encode_setup(
                    args.mode,
                    in_file.clone(),
                    args.output_file.clone(),
                    args.overwrite,
                ) {
                    Ok(path) => open_derived_writer(path),
                    Err(err) => {
                        eprintln!("Error: {:?}", err);
                        return;
                    }
                },
                _ => match open_writer(args.output_file.as_deref(), args.print, args.overwrite) {
                    Ok(writer) => writer,
                    Err(err) => {
                        eprintln!("Error: {:?}", err);
                        return;
                    }
                },
            };

            if ben_and_xben {
                if let Err(err) =
                    encode_ben_to_xben(reader, writer, args.n_cpus, args.compression_level, args.chunk_size)
                {
                    eprintln!("Error: {:?}", err);
                }
            } else if jsonl_and_xben {
                let possible_error = if args.save_all {
                    encode_jsonl_to_xben(
                        reader,
                        writer,
                        BenVariant::Standard,
                        args.n_cpus,
                        args.compression_level,
                        args.chunk_size,
                    )
                } else {
                    encode_jsonl_to_xben(
                        reader,
                        writer,
                        BenVariant::MkvChain,
                        args.n_cpus,
                        args.compression_level,
                        args.chunk_size,
                    )
                };
                if let Err(e) = possible_error {
                    eprintln!("Error: {:?}", e);
                }
            } else {
                eprintln!("Error: Unsupported file type(s) for xencode mode");
            }
        }
        Mode::Decode => {
            tracing::trace!("Running in decode mode");

            let mut ben_and_xben = args.ben_and_xben;
            let mut jsonl_and_ben = args.jsonl_and_ben;

            if let Some(file) = args.input_file.as_ref() {
                if file.ends_with(".ben") {
                    jsonl_and_ben = true;
                } else if file.ends_with(".xben") {
                    ben_and_xben = true;
                }
            }

            let reader = open_reader(args.input_file.as_deref());
            let writer = match args.input_file.as_ref() {
                Some(file) if !args.print => {
                    match decode_setup(
                        file.clone(),
                        args.output_file.clone(),
                        false,
                        args.overwrite,
                    ) {
                        Ok(path) => open_derived_writer(path),
                        Err(err) => {
                            eprintln!("Error: {:?}", err);
                            return;
                        }
                    }
                }
                _ => match open_writer(args.output_file.as_deref(), args.print, args.overwrite) {
                    Ok(writer) => writer,
                    Err(err) => {
                        eprintln!("Error: {:?}", err);
                        return;
                    }
                },
            };

            if ben_and_xben {
                if let Err(err) = decode_xben_to_ben(reader, writer) {
                    eprintln!("Error: {:?}", err);
                }
            } else if jsonl_and_ben {
                if let Err(err) = decode_ben_to_jsonl(reader, writer) {
                    eprintln!("Error: {:?}", err);
                }
            } else {
                eprintln!("Error: Unsupported file type(s) for decode mode");
            }
        }
        Mode::XDecode => {
            tracing::trace!("Running in x-decode mode");

            let reader = open_reader(args.input_file.as_deref());
            let writer = match args.input_file.as_ref() {
                Some(file) if !args.print => {
                    match decode_setup(file.clone(), args.output_file.clone(), true, args.overwrite)
                    {
                        Ok(path) => open_derived_writer(path),
                        Err(err) => {
                            eprintln!("Error: {:?}", err);
                            return;
                        }
                    }
                }
                _ => match open_writer(args.output_file.as_deref(), args.print, args.overwrite) {
                    Ok(writer) => writer,
                    Err(err) => {
                        eprintln!("Error: {:?}", err);
                        return;
                    }
                },
            };

            if let Err(err) = decode_xben_to_jsonl(reader, writer) {
                eprintln!("Error: {:?}", err);
            }
        }
        Mode::Read => {
            tracing::trace!("Running in read mode");
            let reader = BufReader::new(
                File::open(
                    &args
                        .input_file
                        .expect("Must provide input file for read mode."),
                )
                .unwrap(),
            );

            if args.sample_number.is_none() {
                eprintln!("Error: Sample number is required in read mode");
                return;
            }

            let mut writer = match open_writer(args.output_file.as_deref(), args.print, false) {
                Ok(writer) => writer,
                Err(err) => {
                    eprintln!("Error: {:?}", err);
                    return;
                }
            };

            args.sample_number
                .map(|n| match extract_assignment_ben(reader, n) {
                    Ok(vec) => writer.write_all(format!("{:?}\n", vec).as_bytes()).unwrap(),
                    Err(e) => eprintln!("Error: {:?}", e),
                });
        }
        Mode::XzCompress => {
            tracing::trace!("Running in xz compress mode");

            let in_file_name = args
                .input_file
                .expect("Must provide input file for xz-compress mode.");
            let reader = BufReader::new(File::open(&in_file_name).unwrap());

            let out_file_name = match args.output_file {
                Some(name) => name,
                None => in_file_name + ".xz",
            };

            if let Err(err) = check_overwrite(&out_file_name, args.overwrite) {
                eprintln!("Error: {:?}", err);
                return;
            }

            let writer = BufWriter::new(File::create(out_file_name).unwrap());

            if let Err(err) = xz_compress(reader, writer, args.n_cpus, args.compression_level) {
                eprintln!("Error: {:?}", err);
            }
            tracing::trace!("Done!");
        }
        Mode::XzDecompress => {
            tracing::trace!("Running in xz decompress mode");

            let in_file_name = args
                .input_file
                .expect("Must provide input file for xz-decompress mode.");

            if !in_file_name.ends_with(".xz") {
                eprintln!("Error: Unsupported file type for xz decompress mode");
                return;
            }

            let output_file_name = match args.output_file {
                Some(name) => name,
                None => in_file_name[..in_file_name.len() - 3].to_string(),
            };

            if let Err(err) = check_overwrite(&output_file_name, args.overwrite) {
                eprintln!("Error: {:?}", err);
                return;
            }

            let reader = BufReader::new(File::open(&in_file_name).unwrap());
            let writer = BufWriter::new(File::create(output_file_name).unwrap());

            if let Err(err) = xz_decompress(reader, writer) {
                eprintln!("Error: {:?}", err);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_path(name: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("ben-cli-ben-{name}-{nonce}"))
    }

    #[test]
    fn clap_metadata_uses_package_version() {
        let mut command = Args::command();
        let help = command.render_long_help().to_string();

        assert_eq!(command.get_version(), Some(env!("CARGO_PKG_VERSION")));
        assert!(help.contains("Binary Ensemble CLI Tool"));
        assert!(help.contains("--mode"));
        assert!(help.contains("x-encode"));
    }

    #[test]
    fn parse_encode_args() {
        let args = Args::try_parse_from([
            "ben",
            "--mode",
            "encode",
            "--output-file",
            "out.ben",
            "--save-all",
            "--verbose",
            "input.jsonl",
        ])
        .unwrap();

        assert_eq!(args.mode, Mode::Encode);
        assert_eq!(args.input_file.as_deref(), Some("input.jsonl"));
        assert_eq!(args.output_file.as_deref(), Some("out.ben"));
        assert!(args.save_all);
        assert!(args.verbose);
    }

    #[test]
    fn parse_xencode_stream_flags() {
        let args = Args::try_parse_from([
            "ben",
            "--mode",
            "x-encode",
            "--jsonl-and-xben",
            "--ben-and-xben",
            "--jsonl-and-ben",
        ])
        .unwrap();

        assert_eq!(args.mode, Mode::XEncode);
        assert!(args.jsonl_and_xben);
        assert!(args.ben_and_xben);
        assert!(args.jsonl_and_ben);
    }

    #[test]
    fn encode_setup_derives_extensions() {
        assert_eq!(
            encode_setup(Mode::Encode, "samples.jsonl".to_string(), None, true).unwrap(),
            "samples.jsonl.ben"
        );
        assert_eq!(
            encode_setup(Mode::XEncode, "samples.ben".to_string(), None, true).unwrap(),
            "samples.xben"
        );
        assert_eq!(
            encode_setup(Mode::XzCompress, "samples.jsonl".to_string(), None, true).unwrap(),
            "samples.jsonl.xz"
        );
    }

    #[test]
    fn encode_setup_respects_explicit_output() {
        assert_eq!(
            encode_setup(
                Mode::Encode,
                "ignored.jsonl".to_string(),
                Some("custom-output.ben".to_string()),
                true,
            )
            .unwrap(),
            "custom-output.ben"
        );
    }

    #[test]
    fn encode_setup_checks_overwrite() {
        let path = unique_path("existing.ben");
        fs::write(&path, "already here").unwrap();

        let err = encode_setup(
            Mode::Encode,
            "input.jsonl".to_string(),
            Some(path.to_string_lossy().into_owned()),
            true,
        );
        assert!(err.is_ok());

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn decode_setup_derives_ben_and_xben_outputs() {
        assert_eq!(
            decode_setup("samples.ben".to_string(), None, false, true).unwrap(),
            "samples"
        );
        assert_eq!(
            decode_setup("samples.xben".to_string(), None, false, true).unwrap(),
            "samples.ben"
        );
        assert_eq!(
            decode_setup("samples.xben".to_string(), None, true, true).unwrap(),
            "samples"
        );
    }

    #[test]
    fn decode_setup_rejects_xz_input() {
        let err = decode_setup("samples.xz".to_string(), None, false, true).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn decode_setup_rejects_unknown_input() {
        let err = decode_setup("samples.data".to_string(), None, false, true).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn decode_setup_respects_explicit_output() {
        assert_eq!(
            decode_setup(
                "samples.xben".to_string(),
                Some("custom.jsonl".to_string()),
                true,
                true,
            )
            .unwrap(),
            "custom.jsonl"
        );
    }

    #[test]
    fn open_reader_reads_file_contents() {
        let path = unique_path("reader.txt");
        fs::write(&path, "hello\nworld\n").unwrap();

        let mut reader = open_reader(Some(path.to_str().unwrap()));
        let mut content = String::new();
        std::io::Read::read_to_string(&mut reader, &mut content).unwrap();

        assert_eq!(content, "hello\nworld\n");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn open_reader_accepts_stdin() {
        let _reader = open_reader(None);
    }

    #[test]
    fn open_writer_creates_file_and_writes() {
        let path = unique_path("writer.txt");
        {
            let mut writer = open_writer(Some(path.to_str().unwrap()), false, true).unwrap();
            writer.write_all(b"written").unwrap();
        }

        assert_eq!(fs::read_to_string(&path).unwrap(), "written");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn open_writer_supports_stdout_and_print() {
        let mut stdout_writer = open_writer(None, false, true).unwrap();
        stdout_writer.write_all(b"").unwrap();

        let mut print_writer = open_writer(Some("ignored.txt"), true, false).unwrap();
        print_writer.write_all(b"").unwrap();
    }

    #[test]
    fn open_derived_writer_creates_file() {
        let path = unique_path("derived.txt");
        {
            let mut writer = open_derived_writer(path.to_string_lossy().into_owned());
            writer.write_all(b"derived").unwrap();
        }

        assert_eq!(fs::read_to_string(&path).unwrap(), "derived");
        fs::remove_file(path).unwrap();
    }
}
