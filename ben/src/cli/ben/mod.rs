//! `ben` CLI: encode, decode, and stream-compress BEN/XBEN files.

mod args;
mod bundle;
mod paths;

#[cfg(test)]
mod tests;

use args::{resolve_variant, Args, Mode};
use bundle::{run_encode_bundle_with_graph, run_xencode_bundle_with_graph};
use paths::{decode_setup, encode_setup, open_derived_writer, open_reader, open_writer};

use crate::cli::common::{check_overwrite, set_verbose};
use crate::codec::decode::{
    decode_ben_to_jsonl, decode_xben_to_ben, decode_xben_to_jsonl, xz_decompress,
};
use crate::codec::encode::{
    encode_ben_to_xben, encode_jsonl_to_ben, encode_jsonl_to_xben, xz_compress,
};
use crate::ops::extract::extract_assignment_ben;
use clap::Parser;
use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::Path;

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
                    args.mode.clone(),
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
                    args.mode.clone(),
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
                    args.mode.clone(),
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
                    args.mode.clone(),
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
