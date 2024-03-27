use ben::decode::read::extract_assignment_ben;
use ben::decode::*;
use ben::encode::*;
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
    name = "Binary Ensamble CLI Tool",
    about = "This is a command line tool for encoding and decoding binary ensamble files.",
    version = "0.1.0"
)]
struct Args {
    /// Mode to run the program in (encode, decode, or read).
    #[arg(short, long, value_enum)]
    mode: Mode,

    /// Input file to read from.
    #[arg()]
    input_file: String,

    /// Output file to write to. Optional.
    /// If not provided, the output file will be determined
    /// based on the input file and the mode of operation.
    #[arg(short, long)]
    output_file: Option<String>,

    /// Sample number to extract. Optional.
    #[arg(short = 'n', long)]
    sample_number: Option<usize>,

    /// Print the output to the console. Optional.
    #[arg(short, long)]
    print: bool,
}

fn encode_setup(args: &Args) -> Result<String> {
    let extension = if args.mode == Mode::XEncode {
        ".xben"
    } else if args.mode == Mode::Encode {
        ".ben"
    } else {
        ".xz"
    };

    let out_file_name = match &args.output_file {
        Some(name) => name.to_owned(),
        None => {
            if args.input_file.ends_with(".ben") && extension == ".xben" {
                args.input_file.trim_end_matches(".ben").to_owned() + extension
            } else {
                args.input_file.to_string() + extension
            }
        }
    };

    if Path::new(&out_file_name).exists() {
        eprint!(
            "File {:?} already exists, do you want to overwrite it? (y/[n]): ",
            out_file_name
        );
        eprintln!();
        let mut user_input = String::new();
        std::io::stdin().read_line(&mut user_input).unwrap();
        if user_input.trim().to_lowercase() != "y" {
            return Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists));
        }
    }

    Ok(out_file_name)
}

fn decode_setup(args: &Args, full_decode: bool) -> Result<String> {
    let outfile_name = if let Some(name) = &args.output_file {
        name.to_owned()
    } else if args.input_file.ends_with(".ben") {
        args.input_file.trim_end_matches(".ben").to_owned()
    } else if args.input_file.ends_with(".xben") {
        if !full_decode {
            args.input_file.trim_end_matches(".xben").to_owned() + ".ben"
        } else {
            args.input_file.trim_end_matches(".xben").to_owned()
        }
    } else if args.input_file.ends_with(".xz") {
        eprintln!(
            "Error: Unsupported file type for decode mode {:?}. Please decompress xz files with \
            either the xz command line tool or the xz-decompress mode of this tool.",
            args.input_file
        );
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
    } else {
        eprintln!(
            "Error: Unsupported file type for decode mode {:?}. Supported types are .ben and .xben.",
            args.input_file
        );
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
    };

    if Path::new(&outfile_name).exists() {
        eprint!(
            "File {:?} already exists, do you want to overwrite it? (y/[n]): ",
            outfile_name
        );
        let mut user_input = String::new();
        std::io::stdin().read_line(&mut user_input).unwrap();
        if user_input.trim().to_lowercase() != "y" {
            return Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists));
        }
        eprintln!();
    }

    Ok(outfile_name)
}

fn main() {
    let args = Args::parse();

    match args.mode {
        Mode::Encode => {
            eprintln!("Running in encode mode");
            let in_file = File::open(&args.input_file).unwrap();
            let reader = BufReader::new(in_file);

            let mut out_file: Box<dyn Write> = if args.print {
                Box::new(io::stdout())
            } else {
                match encode_setup(&args) {
                    Ok(name) => match File::create(&name) {
                        Ok(file) => Box::new(file),
                        Err(err) => {
                            eprintln!("Error creating file: {:?}", err);
                            return;
                        }
                    },
                    Err(err) => {
                        if err.kind() == std::io::ErrorKind::AlreadyExists {
                            return;
                        }
                        eprintln!("Error: {:?}", err);
                        return;
                    }
                }
            };

            let writer = BufWriter::new(&mut out_file);

            if let Err(err) = jsonl_encode_ben(reader, writer) {
                eprintln!("Error: {:?}", err);
            }
        }
        Mode::XEncode => {
            eprintln!("Running in xencode mode");
            let in_file = File::open(&args.input_file).unwrap();
            let reader = BufReader::new(in_file);

            let mut out_file: Box<dyn Write> = if args.print {
                Box::new(io::stdout())
            } else {
                match encode_setup(&args) {
                    Ok(name) => match File::create(&name) {
                        Ok(file) => Box::new(file),
                        Err(err) => {
                            eprintln!("Error creating file: {:?}", err);
                            return;
                        }
                    },
                    Err(err) => {
                        if err.kind() == std::io::ErrorKind::AlreadyExists {
                            return;
                        }
                        eprintln!("Error: {:?}", err);
                        return;
                    }
                }
            };

            let writer = BufWriter::new(&mut out_file);

            if args.input_file.ends_with(".ben") {
                if let Err(err) = encode_ben_to_xben(reader, writer) {
                    eprintln!("Error: {:?}", err);
                }
            } else {
                if let Err(err) = jsonl_encode_xben(reader, writer) {
                    eprintln!("Error: {:?}", err);
                }
            }
        }
        Mode::Decode => {
            eprintln!("Running in decode mode");
            let file = File::open(&args.input_file).unwrap();
            let reader = BufReader::new(file);

            let xben = args.input_file.ends_with(".xben");

            let mut out_file: Box<dyn Write> = if args.print {
                Box::new(io::stdout())
            } else {
                match decode_setup(&args, false) {
                    Ok(name) => match File::create(&name) {
                        Ok(file) => Box::new(file),
                        Err(err) => {
                            eprintln!("Error creating file: {:?}", err);
                            return;
                        }
                    },
                    Err(err) => {
                        if err.kind() == std::io::ErrorKind::AlreadyExists {
                            return;
                        }
                        eprintln!("Error: {:?}", err);
                        return;
                    }
                }
            };

            let writer = BufWriter::new(&mut out_file);

            if xben {
                eprintln!("Decoding xben file to ben file");

                if let Err(err) = decode_xben_to_ben(reader, writer) {
                    eprintln!("Error: {:?}", err);
                }
            } else {
                eprintln!("Decoding ben file to jsonl file");

                if let Err(err) = jsonl_decode_ben(reader, writer) {
                    eprintln!("Error: {:?}", err);
                }
            }
        }
        Mode::XDecode => {
            eprintln!("Running in xdecode mode");
            let file = File::open(&args.input_file).unwrap();
            let reader = BufReader::new(file);

            let mut out_file: Box<dyn Write> = if args.print {
                Box::new(io::stdout())
            } else {
                match decode_setup(&args, false) {
                    Ok(name) => match File::create(&name) {
                        Ok(file) => Box::new(file),
                        Err(err) => {
                            eprintln!("Error creating file: {:?}", err);
                            return;
                        }
                    },
                    Err(err) => {
                        if err.kind() == std::io::ErrorKind::AlreadyExists {
                            return;
                        }
                        eprintln!("Error: {:?}", err);
                        return;
                    }
                }
            };

            let writer = BufWriter::new(&mut out_file);

            if let Err(err) = jsonl_decode_xben(reader, writer) {
                eprintln!("Error: {:?}", err);
            }
        }
        Mode::Read => {
            eprintln!("Running in read mode");
            let file: File = File::open(&args.input_file).unwrap();
            let reader: BufReader<File> = BufReader::new(file);

            if args.sample_number.is_none() {
                eprintln!("Error: Sample number is required in read mode");
                return;
            }

            let stdout: std::io::Stdout = std::io::stdout();
            let mut writer: BufWriter<std::io::StdoutLock<'_>> = BufWriter::new(stdout.lock());

            args.sample_number
                .map(|n| match extract_assignment_ben(reader, n) {
                    Ok(vec) => writer.write_all(format!("{:?}\n", vec).as_bytes()).unwrap(),
                    Err(e) => eprintln!("Error: {:?}", e),
                });
        }
        Mode::XzCompress => {
            eprintln!("Running in xz compress mode");

            let in_file = File::open(&args.input_file).unwrap();
            let reader = BufReader::new(in_file);

            let out_file_name = match args.output_file {
                Some(name) => name,
                None => args.input_file + ".xz",
            };

            if Path::new(&out_file_name).exists() {
                eprint!(
                    "File {:?} already exists, do you want to overwrite it? (y/[n]): ",
                    out_file_name
                );
                eprintln!();
                let mut user_input = String::new();
                std::io::stdin().read_line(&mut user_input).unwrap();
                if user_input.trim().to_lowercase() != "y" {
                    return;
                }
            }

            let out_file = File::create(out_file_name).unwrap();
            let writer = BufWriter::new(out_file);

            if let Err(err) = xz_compress(reader, writer) {
                eprintln!("Error: {:?}", err);
            }
            eprintln!("Done!");
        }
        Mode::XzDecompress => {
            eprintln!("Running in xz decompress mode");

            if !args.input_file.ends_with(".xz") {
                eprintln!("Error: Unsupported file type for xz decompress mode");
                return;
            }

            let output_file_name = match args.output_file {
                Some(name) => name,
                None => args.input_file[..args.input_file.len() - 3].to_string(),
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

            let in_file = File::open(&args.input_file).unwrap();
            let reader = BufReader::new(in_file);

            let out_file = File::create(output_file_name).unwrap();
            let writer = BufWriter::new(out_file);

            if let Err(err) = xz_decompress(reader, writer) {
                eprintln!("Error: {:?}", err);
            }
        }
    }
}
