use crate::cli::common::{check_overwrite, set_verbose};
use crate::codec::decode::{
    decode_ben_to_jsonl, decode_xben_to_ben, decode_xben_to_jsonl, xz_decompress,
};
use crate::codec::encode::{
    encode_ben_to_xben, encode_jsonl_to_ben, encode_jsonl_to_xben, xz_compress,
};
use crate::io::bundle::format::{AssignmentFormat, ASSET_TYPE_GRAPH, CANONICAL_NAME_GRAPH};
use crate::io::bundle::writer::BendlAppender;
use crate::io::bundle::{AddAssetOptions, BendlWriter};
use crate::io::reader::subsample::count_samples_from_file;
use crate::ops::extract::extract_assignment_ben;
use crate::BenVariant;
use clap::{Parser, ValueEnum};
use std::{
    fs::{File, OpenOptions},
    io::{self, BufRead, BufReader, BufWriter, Result, Write},
    path::{Path, PathBuf},
};

type DynReader = Box<dyn io::BufRead>;
type DynWriter = Box<dyn Write>;

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq)]
enum CliVariant {
    /// Store each sample independently.
    Standard,
    /// Store one frame plus a repetition count for repeated consecutive samples.
    #[value(alias = "mkv_chain")]
    Mkvchain,
    /// Store delta-encoded frames.
    #[value(alias = "two_delta")]
    Twodelta,
}

/// Resolve the BEN variant from the CLI flags.
///
/// `--variant` takes precedence over `--save-all`.
/// If neither is given, defaults to MkvChain.
fn resolve_variant(variant: Option<CliVariant>, save_all: bool) -> BenVariant {
    match variant {
        Some(CliVariant::Standard) => BenVariant::Standard,
        Some(CliVariant::Mkvchain) => BenVariant::MkvChain,
        Some(CliVariant::Twodelta) => BenVariant::TwoDelta,
        None if save_all => BenVariant::Standard,
        None => BenVariant::MkvChain,
    }
}

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
    /// Equivalent to `--variant standard`. Ignored if `--variant` is set.
    #[arg(short = 'a', long)]
    save_all: bool,
    /// BEN variant to use when encoding.
    /// Possible values: standard, mkvchain, twodelta.
    /// Defaults to mkvchain if neither this nor --save-all is given.
    /// Takes precedence over --save-all when both are provided.
    #[arg(short = 't', long, value_enum)]
    variant: Option<CliVariant>,
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
    /// Embed a graph JSON asset alongside the assignment stream and emit
    /// the result as a `.bendl` bundle. The graph is added after the
    /// assignment stream has been fully written. Only applies to the
    /// `encode` and `x-encode` modes.
    #[arg(long)]
    graph: Option<PathBuf>,
}

/// Derive the output path for encode-style CLI modes.
///
/// # Arguments
///
/// * `mode` - The encode-oriented CLI mode being executed.
/// * `input_file_name` - The input file path supplied by the user.
/// * `output_file_name` - An optional explicit output path.
/// * `overwrite` - Whether to skip overwrite prompting.
/// * `with_graph` - When true, the output is a `.bendl` bundle instead
///   of a bare `.ben`/`.xben` stream, so the derived extension is
///   `.bendl` regardless of `mode`.
///
/// # Returns
///
/// Returns the resolved output path.
fn encode_setup(
    mode: Mode,
    input_file_name: String,
    output_file_name: Option<String>,
    overwrite: bool,
    with_graph: bool,
) -> Result<String> {
    let extension = if with_graph {
        ".bendl"
    } else if mode == Mode::XEncode {
        ".xben"
    } else if mode == Mode::Encode {
        ".ben"
    } else {
        ".xz"
    };

    let out_file_name = match output_file_name {
        Some(name) => name.to_owned(),
        None => {
            let stripped_ben = input_file_name.ends_with(".ben")
                && (extension == ".xben" || extension == ".bendl");
            let stripped_xben = input_file_name.ends_with(".xben") && extension == ".bendl";
            if stripped_ben {
                input_file_name.trim_end_matches(".ben").to_owned() + extension
            } else if stripped_xben {
                input_file_name.trim_end_matches(".xben").to_owned() + extension
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

/// Count the number of non-empty lines in a JSONL file. Used to populate
/// the bundle header's `sample_count` when wrapping a stream encode in a
/// `.bendl` container.
fn count_jsonl_lines(path: &Path) -> io::Result<i64> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut n: i64 = 0;
    for line in reader.lines() {
        let line = line?;
        if !line.is_empty() {
            n += 1;
        }
    }
    Ok(n)
}

/// After a finalized `.bendl` has been written, reopen it in append mode
/// and attach the graph asset in-place. This runs *after* the stream has
/// finished, which is why we print "Adding graph..." at this point.
fn append_graph_asset(out_path: &str, graph_path: &Path) -> Result<()> {
    eprintln!("Adding graph...");
    let graph_bytes = std::fs::read(graph_path).map_err(|e| {
        io::Error::new(
            io::ErrorKind::Other,
            format!("failed to read graph {graph_path:?}: {e}"),
        )
    })?;

    let file = OpenOptions::new().read(true).write(true).open(out_path)?;
    let mut appender = BendlAppender::open(file)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{e}")))?;
    appender
        .add_asset(
            ASSET_TYPE_GRAPH,
            CANONICAL_NAME_GRAPH,
            &graph_bytes,
            AddAssetOptions::defaults().json(),
        )
        .map_err(|e| {
            io::Error::new(
                io::ErrorKind::Other,
                format!("failed to add graph asset: {e}"),
            )
        })?;
    appender
        .commit()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{e}")))?;
    Ok(())
}

/// Encode `input_path` (JSONL) to BEN inside a fresh `.bendl` bundle at
/// `out_path` and then append the graph as a post-stream asset.
fn run_encode_bundle_with_graph(
    input_path: &Path,
    out_path: &str,
    variant: BenVariant,
    graph_path: &Path,
) -> Result<()> {
    // Validate the graph file is readable before we do any real work,
    // so a bad --graph path doesn't leave a half-written bundle behind.
    std::fs::metadata(graph_path).map_err(|e| {
        io::Error::new(
            io::ErrorKind::Other,
            format!("failed to stat graph {graph_path:?}: {e}"),
        )
    })?;

    let sample_count = count_jsonl_lines(input_path)?;

    let out_file = File::create(out_path)?;
    let mut bendl_writer = BendlWriter::new(out_file, AssignmentFormat::Ben)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{e}")))?;
    {
        let mut handle = bendl_writer
            .begin_stream()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{e}")))?;
        let reader = BufReader::new(File::open(input_path)?);
        encode_jsonl_to_ben(reader, &mut handle, variant)?;
        handle
            .finish(sample_count)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{e}")))?;
    }
    bendl_writer
        .finish()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{e}")))?;

    append_graph_asset(out_path, graph_path)
}

/// Encode `input_path` (JSONL or `.ben`) to XBEN inside a fresh `.bendl`
/// bundle at `out_path` and then append the graph as a post-stream asset.
#[allow(clippy::too_many_arguments)]
fn run_xencode_bundle_with_graph(
    input_path: &Path,
    out_path: &str,
    variant: BenVariant,
    from_ben: bool,
    n_threads: Option<u32>,
    compression_level: Option<u32>,
    chunk_size: Option<usize>,
    graph_path: &Path,
) -> Result<()> {
    std::fs::metadata(graph_path).map_err(|e| {
        io::Error::new(
            io::ErrorKind::Other,
            format!("failed to stat graph {graph_path:?}: {e}"),
        )
    })?;

    let sample_count: i64 = if from_ben {
        count_samples_from_file(input_path, "ben")? as i64
    } else {
        count_jsonl_lines(input_path)?
    };

    let out_file = File::create(out_path)?;
    let mut bendl_writer = BendlWriter::new(out_file, AssignmentFormat::Xben)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{e}")))?;
    {
        let mut handle = bendl_writer
            .begin_stream()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{e}")))?;
        let reader = BufReader::new(File::open(input_path)?);
        if from_ben {
            encode_ben_to_xben(
                reader,
                &mut handle,
                n_threads,
                compression_level,
                chunk_size,
            )?;
        } else {
            encode_jsonl_to_xben(
                reader,
                &mut handle,
                variant,
                n_threads,
                compression_level,
                chunk_size,
            )?;
        }
        handle
            .finish(sample_count)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{e}")))?;
    }
    bendl_writer
        .finish()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{e}")))?;

    append_graph_asset(out_path, graph_path)
}

/// Parse CLI arguments and execute the selected `ben` sub-mode.
pub fn run() {
    let args = Args::parse();
    set_verbose(args.verbose);

    // --graph is only meaningful for the stream-producing modes.
    if args.graph.is_some() && args.mode != Mode::Encode && args.mode != Mode::XEncode {
        eprintln!("Error: --graph is only supported with --mode encode or --mode x-encode");
        return;
    }

    match args.mode {
        Mode::Encode => {
            tracing::trace!("Running in encode mode");

            // --graph path: produce a .bendl bundle with the BEN stream
            // plus a post-stream graph asset.
            if let Some(graph_path) = args.graph.as_ref() {
                let in_file = match args.input_file.as_ref() {
                    Some(f) => f,
                    None => {
                        eprintln!("Error: --graph requires an input file (stdin not supported).");
                        return;
                    }
                };
                if args.print {
                    eprintln!("Error: --graph is incompatible with --print.");
                    return;
                }
                let out_path = match encode_setup(
                    args.mode,
                    in_file.clone(),
                    args.output_file.clone(),
                    args.overwrite,
                    true,
                ) {
                    Ok(path) => path,
                    Err(err) => {
                        eprintln!("Error: {:?}", err);
                        return;
                    }
                };
                let variant = resolve_variant(args.variant, args.save_all);
                if let Err(err) =
                    run_encode_bundle_with_graph(Path::new(in_file), &out_path, variant, graph_path)
                {
                    eprintln!("Error: {:?}", err);
                }
                return;
            }

            let reader = open_reader(args.input_file.as_deref());
            let writer = match args.input_file.as_ref() {
                Some(in_file) if !args.print => match encode_setup(
                    args.mode,
                    in_file.clone(),
                    args.output_file.clone(),
                    args.overwrite,
                    false,
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

            let variant = resolve_variant(args.variant, args.save_all);
            if let Err(err) = encode_jsonl_to_ben(reader, writer, variant) {
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

            // --graph path: produce a .bendl bundle with the XBEN stream
            // plus a post-stream graph asset.
            if let Some(graph_path) = args.graph.as_ref() {
                let in_file = match args.input_file.as_ref() {
                    Some(f) => f,
                    None => {
                        eprintln!("Error: --graph requires an input file (stdin not supported).");
                        return;
                    }
                };
                if args.print {
                    eprintln!("Error: --graph is incompatible with --print.");
                    return;
                }
                if !ben_and_xben && !jsonl_and_xben {
                    eprintln!("Error: Unsupported file type(s) for xencode mode");
                    return;
                }
                let out_path = match encode_setup(
                    args.mode,
                    in_file.clone(),
                    args.output_file.clone(),
                    args.overwrite,
                    true,
                ) {
                    Ok(path) => path,
                    Err(err) => {
                        eprintln!("Error: {:?}", err);
                        return;
                    }
                };
                let variant = resolve_variant(args.variant, args.save_all);
                if let Err(err) = run_xencode_bundle_with_graph(
                    Path::new(in_file),
                    &out_path,
                    variant,
                    ben_and_xben,
                    args.n_cpus,
                    args.compression_level,
                    args.chunk_size,
                    graph_path,
                ) {
                    eprintln!("Error: {:?}", err);
                }
                return;
            }

            let reader = open_reader(args.input_file.as_deref());
            let writer = match args.input_file.as_ref() {
                Some(in_file) if !args.print => match encode_setup(
                    args.mode,
                    in_file.clone(),
                    args.output_file.clone(),
                    args.overwrite,
                    false,
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
                if let Err(err) = encode_ben_to_xben(
                    reader,
                    writer,
                    args.n_cpus,
                    args.compression_level,
                    args.chunk_size,
                ) {
                    eprintln!("Error: {:?}", err);
                }
            } else if jsonl_and_xben {
                let variant = resolve_variant(args.variant, args.save_all);
                if let Err(e) = encode_jsonl_to_xben(
                    reader,
                    writer,
                    variant,
                    args.n_cpus,
                    args.compression_level,
                    args.chunk_size,
                ) {
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
    fn parse_variant_flag() {
        let args = Args::try_parse_from([
            "ben",
            "--mode",
            "encode",
            "--variant",
            "twodelta",
            "input.jsonl",
        ])
        .unwrap();

        assert_eq!(args.variant, Some(CliVariant::Twodelta));
    }

    #[test]
    fn parse_variant_aliases() {
        let args = Args::try_parse_from([
            "ben",
            "--mode",
            "encode",
            "--variant",
            "mkv_chain",
            "input.jsonl",
        ])
        .unwrap();
        assert_eq!(args.variant, Some(CliVariant::Mkvchain));

        let args = Args::try_parse_from([
            "ben",
            "--mode",
            "encode",
            "--variant",
            "two_delta",
            "input.jsonl",
        ])
        .unwrap();
        assert_eq!(args.variant, Some(CliVariant::Twodelta));
    }

    #[test]
    fn resolve_variant_precedence() {
        // --variant takes precedence over --save-all
        assert_eq!(
            resolve_variant(Some(CliVariant::Twodelta), true),
            BenVariant::TwoDelta
        );
        assert_eq!(
            resolve_variant(Some(CliVariant::Mkvchain), true),
            BenVariant::MkvChain
        );
        // --save-all alone means Standard
        assert_eq!(resolve_variant(None, true), BenVariant::Standard);
        // neither means MkvChain
        assert_eq!(resolve_variant(None, false), BenVariant::MkvChain);
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
            encode_setup(Mode::Encode, "samples.jsonl".to_string(), None, true, false).unwrap(),
            "samples.jsonl.ben"
        );
        assert_eq!(
            encode_setup(Mode::XEncode, "samples.ben".to_string(), None, true, false).unwrap(),
            "samples.xben"
        );
        assert_eq!(
            encode_setup(
                Mode::XzCompress,
                "samples.jsonl".to_string(),
                None,
                true,
                false
            )
            .unwrap(),
            "samples.jsonl.xz"
        );
    }

    #[test]
    fn encode_setup_with_graph_derives_bendl_extension() {
        // JSONL + encode + graph → .bendl
        assert_eq!(
            encode_setup(Mode::Encode, "samples.jsonl".to_string(), None, true, true).unwrap(),
            "samples.jsonl.bendl"
        );
        // .ben input to x-encode with graph trims the .ben suffix
        assert_eq!(
            encode_setup(Mode::XEncode, "samples.ben".to_string(), None, true, true).unwrap(),
            "samples.bendl"
        );
        // .xben input to x-encode with graph trims the .xben suffix
        assert_eq!(
            encode_setup(Mode::XEncode, "samples.xben".to_string(), None, true, true).unwrap(),
            "samples.bendl"
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
                false,
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
            false,
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

    #[test]
    fn resolve_variant_standard_arm() {
        assert_eq!(
            resolve_variant(Some(CliVariant::Standard), false),
            BenVariant::Standard
        );
    }

    #[test]
    fn count_jsonl_lines_counts_nonempty_lines() {
        let path = unique_path("count.jsonl");
        fs::write(&path, b"{\"a\":1}\n\n{\"b\":2}\n").unwrap();
        let count = count_jsonl_lines(&path).unwrap();
        assert_eq!(count, 2);
        fs::remove_file(path).unwrap();
    }

    /// Write a two-sample Standard BEN JSONL file to a temp path.
    fn write_temp_jsonl(name: &str) -> std::path::PathBuf {
        let path = unique_path(name);
        fs::write(
            &path,
            b"{\"assignment\":[1,2,3],\"sample\":1}\n{\"assignment\":[2,1,3],\"sample\":2}\n",
        )
        .unwrap();
        path
    }

    /// Write a minimal graph JSON file to a temp path.
    fn write_temp_graph(name: &str) -> std::path::PathBuf {
        let path = unique_path(name);
        fs::write(&path, b"{\"nodes\":[0,1,2],\"adj\":[[1],[0,2],[1]]}").unwrap();
        path
    }

    #[test]
    fn append_graph_asset_adds_graph_to_bundle() {
        use crate::io::bundle::{AddAssetOptions, BendlReader, BendlWriter};
        use crate::io::bundle::format::{AssignmentFormat, ASSET_TYPE_GRAPH};
        use std::io::Cursor;

        // Build a minimal finalized .bendl in memory, write to temp file.
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut writer = BendlWriter::new(Cursor::new(&mut buf), AssignmentFormat::Ben).unwrap();
            writer.write_stream_bytes(b"STANDARD BEN FILE\x00fake", 1).unwrap();
            writer.finish().unwrap();
        }
        let bendl_path = unique_path("append_graph.bendl");
        fs::write(&bendl_path, &buf).unwrap();

        let graph_path = write_temp_graph("append_graph.json");

        append_graph_asset(bendl_path.to_str().unwrap(), &graph_path).unwrap();

        // Verify the graph asset was added.
        let file = fs::File::open(&bendl_path).unwrap();
        let reader = BendlReader::open(std::io::BufReader::new(file)).unwrap();
        assert!(reader.find_asset_by_name("graph.json").is_some());

        fs::remove_file(&bendl_path).unwrap();
        fs::remove_file(&graph_path).unwrap();
    }

    #[test]
    fn run_encode_bundle_with_graph_creates_bendl() {
        use crate::io::bundle::BendlReader;

        let jsonl = write_temp_jsonl("enc_graph_input.jsonl");
        let graph = write_temp_graph("enc_graph.json");
        let out = unique_path("enc_graph_output.bendl");

        run_encode_bundle_with_graph(&jsonl, out.to_str().unwrap(), BenVariant::Standard, &graph)
            .unwrap();

        let file = fs::File::open(&out).unwrap();
        let reader = BendlReader::open(std::io::BufReader::new(file)).unwrap();
        assert!(reader.is_complete());
        assert!(reader.find_asset_by_name("graph.json").is_some());
        assert_eq!(reader.sample_count(), Some(2));

        fs::remove_file(&jsonl).unwrap();
        fs::remove_file(&graph).unwrap();
        fs::remove_file(&out).unwrap();
    }

    #[test]
    fn run_xencode_bundle_with_graph_from_jsonl_creates_bendl() {
        use crate::io::bundle::BendlReader;

        let jsonl = write_temp_jsonl("xencode_graph_input.jsonl");
        let graph = write_temp_graph("xencode_graph.json");
        let out = unique_path("xencode_graph_output.bendl");

        run_xencode_bundle_with_graph(
            &jsonl,
            out.to_str().unwrap(),
            BenVariant::Standard,
            false,
            None,
            None,
            None,
            &graph,
        )
        .unwrap();

        let file = fs::File::open(&out).unwrap();
        let reader = BendlReader::open(std::io::BufReader::new(file)).unwrap();
        assert!(reader.is_complete());
        assert!(reader.find_asset_by_name("graph.json").is_some());

        fs::remove_file(&jsonl).unwrap();
        fs::remove_file(&graph).unwrap();
        fs::remove_file(&out).unwrap();
    }

    #[test]
    fn run_xencode_bundle_with_graph_from_ben_creates_bendl() {
        use crate::codec::encode::encode_jsonl_to_ben;
        use crate::io::bundle::BendlReader;
        use std::io::Cursor;

        // First create a BEN file from JSONL.
        let jsonl = b"{\"assignment\":[1,2],\"sample\":1}\n{\"assignment\":[2,1],\"sample\":2}\n";
        let mut ben_bytes = Vec::new();
        encode_jsonl_to_ben(Cursor::new(jsonl), &mut ben_bytes, BenVariant::Standard).unwrap();
        let ben_path = unique_path("xencode_from_ben_input.ben");
        fs::write(&ben_path, &ben_bytes).unwrap();

        let graph = write_temp_graph("xencode_from_ben_graph.json");
        let out = unique_path("xencode_from_ben_output.bendl");

        run_xencode_bundle_with_graph(
            &ben_path,
            out.to_str().unwrap(),
            BenVariant::Standard,
            true,
            None,
            None,
            None,
            &graph,
        )
        .unwrap();

        let file = fs::File::open(&out).unwrap();
        let reader = BendlReader::open(std::io::BufReader::new(file)).unwrap();
        assert!(reader.is_complete());
        assert!(reader.find_asset_by_name("graph.json").is_some());

        fs::remove_file(&ben_path).unwrap();
        fs::remove_file(&graph).unwrap();
        fs::remove_file(&out).unwrap();
    }

    #[test]
    fn append_graph_asset_errors_on_missing_graph_file() {
        use crate::io::bundle::{BendlWriter};
        use crate::io::bundle::format::AssignmentFormat;
        use std::io::Cursor;

        let mut buf: Vec<u8> = Vec::new();
        {
            let mut writer = BendlWriter::new(Cursor::new(&mut buf), AssignmentFormat::Ben).unwrap();
            writer.write_stream_bytes(b"STANDARD BEN FILE\x00fake", 1).unwrap();
            writer.finish().unwrap();
        }
        let bendl_path = unique_path("err_graph.bendl");
        fs::write(&bendl_path, &buf).unwrap();

        let nonexistent = unique_path("nonexistent.json");
        let err = append_graph_asset(bendl_path.to_str().unwrap(), &nonexistent).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
        assert!(err.to_string().contains("failed to read graph"));
        let _ = fs::remove_file(&bendl_path);
    }

    #[test]
    fn run_encode_bundle_with_graph_errors_on_missing_graph() {
        let jsonl = write_temp_jsonl("err_enc_input.jsonl");
        let out = unique_path("err_enc_output.bendl");
        let nonexistent = unique_path("nonexistent.json");

        let err = run_encode_bundle_with_graph(
            &jsonl, out.to_str().unwrap(), BenVariant::Standard, &nonexistent,
        )
        .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
        assert!(err.to_string().contains("failed to stat graph"));
        let _ = fs::remove_file(&jsonl);
        let _ = fs::remove_file(&out);
    }

    #[test]
    fn run_xencode_bundle_with_graph_errors_on_missing_graph() {
        let jsonl = write_temp_jsonl("err_xenc_input.jsonl");
        let out = unique_path("err_xenc_output.bendl");
        let nonexistent = unique_path("nonexistent.json");

        let err = run_xencode_bundle_with_graph(
            &jsonl, out.to_str().unwrap(), BenVariant::Standard, false,
            None, None, None, &nonexistent,
        )
        .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
        assert!(err.to_string().contains("failed to stat graph"));
        let _ = fs::remove_file(&jsonl);
        let _ = fs::remove_file(&out);
    }
}
