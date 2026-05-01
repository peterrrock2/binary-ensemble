use super::args::Mode;
use crate::cli::common::check_overwrite;
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Result, Write};
use std::path::Path;

pub(super) type DynReader = Box<dyn io::BufRead>;
pub(super) type DynWriter = Box<dyn Write>;

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
pub(super) fn encode_setup(
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
pub(super) fn decode_setup(
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
pub(super) fn open_reader(input_file: Option<&str>) -> DynReader {
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
pub(super) fn open_writer(
    output_file: Option<&str>,
    print: bool,
    overwrite: bool,
) -> Result<DynWriter> {
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
pub(super) fn open_derived_writer(path: String) -> DynWriter {
    Box::new(BufWriter::new(File::create(path).unwrap()))
}

/// Count the number of non-empty lines in a JSONL file. Used to populate
/// the bundle header's `sample_count` when wrapping a stream encode in a
/// `.bendl` container.
pub(super) fn count_jsonl_lines(path: &Path) -> io::Result<i64> {
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
