use ben::decode::read::extract_assignment_ben;
use ben::decode::*;
use ben::encode::*;
use ben::{logln, BenVariant};
use clap::{Parser, ValueEnum};
use std::{
    fs::File,
    io::{self, BufReader, BufWriter, Result, Write},
    path::Path,
};
/// Defines the mode of operation.
#[derive(Parser, Debug, Clone, ValueEnum, PartialEq)]
enum Mode {
    Encode,
    XEncode,
    Decode,
    XDecode,
    Read,
    XzCompress,
    XzDecompress,
}

/// Defines the command line arguments accepted by the program.
#[derive(Parser, Debug)]
#[command(
    name = "Binary Ensemble CLI Tool",
    about = "This is a command line tool for encoding and decoding binary ensemble files.",
    version = "0.2.0"
)]
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
}

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

    if let Err(e) = check_overwrite(&out_file_name, overwrite) {
        return Err(e);
    }

    Ok(out_file_name)
}

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

    if let Err(e) = check_overwrite(&out_file_name, overwrite) {
        return Err(e);
    }

    Ok(out_file_name)
}

fn check_overwrite(file_name: &str, overwrite: bool) -> Result<()> {
    if Path::new(file_name).exists() && !overwrite {
        eprint!(
            "File {:?} already exists, do you want to overwrite it? (y/[n]): ",
            file_name
        );
        let mut user_input = String::new();
        std::io::stdin().read_line(&mut user_input).unwrap();
        eprintln!();
        if user_input.trim().to_lowercase() != "y" {
            return Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists));
        }
    }
    Ok(())
}

fn main() {
    let args = Args::parse();

    if args.verbose {
        std::env::set_var("RUST_LOG", "trace");
    }

    match args.mode {
        Mode::Encode => {
            logln!("Running in encode mode");

            let reader: Box<dyn io::BufRead>;
            let writer: Box<dyn Write>;

            match args.input_file {
                Some(in_file) => {
                    reader = Box::new(BufReader::new(File::open(&in_file).unwrap()))
                        as Box<dyn io::BufRead>;

                    if args.print {
                        writer = Box::new(BufWriter::new(io::stdout())) as Box<dyn Write>;
                    } else {
                        let out_file_name = match encode_setup(
                            args.mode,
                            in_file,
                            args.output_file,
                            args.overwrite,
                        ) {
                            Ok(name) => name,
                            Err(err) => {
                                eprintln!("Error: {:?}", err);
                                return;
                            }
                        };
                        let out_file = File::create(&out_file_name).unwrap();
                        writer = Box::new(BufWriter::new(out_file)) as Box<dyn Write>;
                    }
                }
                None => {
                    reader = Box::new(BufReader::new(io::stdin())) as Box<dyn io::BufRead>;

                    writer = if args.print {
                        Box::new(BufWriter::new(io::stdout())) as Box<dyn Write>
                    } else {
                        match args.output_file {
                            Some(name) => {
                                if let Err(e) = check_overwrite(&name, args.overwrite) {
                                    eprintln!("Error: {:?}", e);
                                    return;
                                }
                                let out_file = File::create(&name).unwrap();
                                Box::new(BufWriter::new(out_file)) as Box<dyn Write>
                            }
                            None => Box::new(BufWriter::new(io::stdout())) as Box<dyn Write>,
                        }
                    }
                }
            };

            let possible_error = if args.save_all {
                encode_jsonl_to_ben(reader, writer, BenVariant::Standard)
            } else {
                encode_jsonl_to_ben(reader, writer, BenVariant::MkvChain)
            };

            match possible_error {
                Ok(_) => {}
                Err(err) => {
                    eprintln!("Error: {:?}", err);
                }
            }
        }
        Mode::XEncode => {
            logln!("Running in xencode mode");

            let mut ben_and_xben = args.ben_and_xben;
            let mut jsonl_and_xben = args.ben_and_xben;

            let reader: Box<dyn io::BufRead>;
            let writer: Box<dyn Write>;

            match args.input_file {
                Some(in_file) => {
                    if in_file.ends_with(".ben") {
                        ben_and_xben = true;
                    } else if in_file.ends_with(".jsonl") {
                        jsonl_and_xben = true;
                    }

                    reader = Box::new(BufReader::new(File::open(&in_file).unwrap()))
                        as Box<dyn io::BufRead>;

                    writer = if args.print {
                        Box::new(BufWriter::new(io::stdout())) as Box<dyn Write>
                    } else {
                        let out_file_name = match encode_setup(
                            args.mode,
                            in_file,
                            args.output_file,
                            args.overwrite,
                        ) {
                            Ok(name) => name,
                            Err(err) => {
                                eprintln!("Error: {:?}", err);
                                return;
                            }
                        };
                        let out_file = File::create(&out_file_name).unwrap();
                        Box::new(BufWriter::new(out_file)) as Box<dyn Write>
                    };
                }
                None => {
                    reader = Box::new(BufReader::new(io::stdin())) as Box<dyn io::BufRead>;

                    writer = match args.output_file {
                        Some(name) => {
                            if let Err(e) = check_overwrite(&name, args.overwrite) {
                                eprintln!("Error: {:?}", e);
                                return;
                            }
                            let out_file = File::create(&name).unwrap();
                            Box::new(BufWriter::new(out_file)) as Box<dyn Write>
                        }
                        None => Box::new(BufWriter::new(io::stdout())) as Box<dyn Write>,
                    };
                }
            };

            if ben_and_xben {
                if let Err(err) =
                    encode_ben_to_xben(reader, writer, args.n_cpus, args.compression_level)
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
                    )
                } else {
                    encode_jsonl_to_xben(
                        reader,
                        writer,
                        BenVariant::MkvChain,
                        args.n_cpus,
                        args.compression_level,
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
            logln!("Running in decode mode");

            let mut ben_and_xben = args.ben_and_xben;
            let mut jsonl_and_ben = args.jsonl_and_ben;

            let reader: Box<dyn io::BufRead>;
            let writer: Box<dyn Write>;

            match args.input_file {
                Some(file) => {
                    if file.ends_with(".ben") {
                        jsonl_and_ben = true;
                    } else if file.ends_with(".xben") {
                        ben_and_xben = true;
                    }

                    reader = Box::new(BufReader::new(File::open(&file).unwrap()))
                        as Box<dyn io::BufRead>;

                    writer = if args.print {
                        Box::new(BufWriter::new(io::stdout())) as Box<dyn Write>
                    } else {
                        let out_file_name =
                            match decode_setup(file, args.output_file, false, args.overwrite) {
                                Ok(name) => name,
                                Err(err) => {
                                    eprintln!("Error: {:?}", err);
                                    return;
                                }
                            };
                        let out_file = File::create(&out_file_name).unwrap();
                        Box::new(BufWriter::new(out_file)) as Box<dyn Write>
                    };
                }
                None => {
                    reader = Box::new(BufReader::new(io::stdin())) as Box<dyn io::BufRead>;

                    writer = if args.print {
                        Box::new(BufWriter::new(io::stdout())) as Box<dyn Write>
                    } else {
                        match args.output_file {
                            Some(out_name) => {
                                if let Err(e) = check_overwrite(&out_name, args.overwrite) {
                                    eprintln!("Error: {:?}", e);
                                    return;
                                }
                                let out_file = File::create(&out_name).unwrap();
                                Box::new(BufWriter::new(out_file)) as Box<dyn Write>
                            }
                            None => Box::new(BufWriter::new(io::stdout())) as Box<dyn Write>,
                        }
                    }
                }
            }

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
            logln!("Running in x-decode mode");

            let reader: Box<dyn io::BufRead>;
            let writer: Box<dyn Write>;

            match args.input_file {
                Some(file) => {
                    reader = Box::new(BufReader::new(File::open(&file).unwrap()))
                        as Box<dyn io::BufRead>;

                    writer = if args.print {
                        Box::new(BufWriter::new(io::stdout())) as Box<dyn Write>
                    } else {
                        let out_file_name =
                            match decode_setup(file, args.output_file, true, args.overwrite) {
                                Ok(name) => name,
                                Err(err) => {
                                    eprintln!("Error: {:?}", err);
                                    return;
                                }
                            };
                        let out_file = File::create(&out_file_name).unwrap();
                        Box::new(BufWriter::new(out_file)) as Box<dyn Write>
                    }
                }
                None => {
                    reader = Box::new(BufReader::new(io::stdin())) as Box<dyn io::BufRead>;

                    writer = if args.print {
                        Box::new(BufWriter::new(io::stdout())) as Box<dyn Write>
                    } else {
                        match args.output_file {
                            Some(out_name) => {
                                if let Err(e) = check_overwrite(&out_name, args.overwrite) {
                                    eprintln!("Error: {:?}", e);
                                    return;
                                }
                                let out_file = File::create(&out_name).unwrap();
                                Box::new(BufWriter::new(out_file)) as Box<dyn Write>
                            }
                            None => Box::new(BufWriter::new(io::stdout())) as Box<dyn Write>,
                        }
                    }
                }
            }

            if let Err(err) = decode_xben_to_jsonl(reader, writer) {
                eprintln!("Error: {:?}", err);
            }
        }
        Mode::Read => {
            logln!("Running in read mode");
            let file: File = File::open(
                &args
                    .input_file
                    .expect("Must provide input file for read mode."),
            )
            .unwrap();
            let reader: BufReader<File> = BufReader::new(file);

            if args.sample_number.is_none() {
                eprintln!("Error: Sample number is required in read mode");
                return;
            }

            let mut writer = if args.print {
                Box::new(BufWriter::new(io::stdout())) as Box<dyn Write>
            } else {
                match args.output_file {
                    Some(name) => {
                        let file: File = File::create(name).unwrap();
                        Box::new(BufWriter::new(file)) as Box<dyn Write>
                    }
                    None => Box::new(BufWriter::new(io::stdout())) as Box<dyn Write>,
                }
            };

            args.sample_number
                .map(|n| match extract_assignment_ben(reader, n) {
                    Ok(vec) => writer.write_all(format!("{:?}\n", vec).as_bytes()).unwrap(),
                    Err(e) => eprintln!("Error: {:?}", e),
                });
        }
        Mode::XzCompress => {
            logln!("Running in xz compress mode");

            let in_file_name = args
                .input_file
                .expect("Must provide input file for xz-compress mode.");
            let in_file = File::open(&in_file_name).unwrap();
            let reader = BufReader::new(in_file);

            let out_file_name = match args.output_file {
                Some(name) => name,
                None => in_file_name + ".xz",
            };

            if Path::new(&out_file_name).exists() {
                eprint!(
                    "File {:?} already exists, do you want to overwrite it? (y/[n]): ",
                    out_file_name
                );
                let mut user_input = String::new();
                std::io::stdin().read_line(&mut user_input).unwrap();
                if user_input.trim().to_lowercase() != "y" {
                    return;
                }
                eprintln!();
            }

            let out_file = File::create(out_file_name).unwrap();
            let writer = BufWriter::new(out_file);

            if let Err(err) = xz_compress(reader, writer, args.n_cpus, args.compression_level) {
                eprintln!("Error: {:?}", err);
            }
            logln!("Done!");
        }
        Mode::XzDecompress => {
            logln!("Running in xz decompress mode");

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

            if Path::new(&output_file_name).exists() {
                eprint!(
                    "File {:?} already exists, do you want to overwrite it? (y/[n]): ",
                    output_file_name
                );
                eprintln!();
                let mut user_input = String::new();
                std::io::stdin().read_line(&mut user_input).unwrap();
                if user_input.trim().to_lowercase() != "y" {
                    return;
                }
            }

            let in_file = File::open(&in_file_name).unwrap();
            let reader = BufReader::new(in_file);

            let out_file = File::create(output_file_name).unwrap();
            let writer = BufWriter::new(out_file);

            if let Err(err) = xz_decompress(reader, writer) {
                eprintln!("Error: {:?}", err);
            }
        }
    }
}
