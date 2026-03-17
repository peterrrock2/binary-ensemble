use binary_ensemble::BenVariant;
use pyo3::exceptions::{PyIOError, PyValueError};
use pyo3::prelude::PyResult;
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::PathBuf;

pub fn parse_variant(variant: Option<&str>) -> PyResult<BenVariant> {
    match variant {
        Some("standard") => Ok(BenVariant::Standard),
        Some("mkv_chain") | Some("markov") | None => Ok(BenVariant::MkvChain),
        Some(other) => Err(PyValueError::new_err(format!(
            "Unknown variant: {other}. Supported variants are 'standard' and 'mkv_chain'."
        ))),
    }
}

pub fn validate_input_output_paths(in_file: &PathBuf, out_file: &PathBuf) -> PyResult<()> {
    if in_file == out_file {
        return Err(PyIOError::new_err("Input and output paths must differ."));
    }
    if !in_file.exists() {
        return Err(PyIOError::new_err(format!(
            "Input file {} does not exist.",
            in_file.display()
        )));
    }
    Ok(())
}

pub fn open_input(in_file: &PathBuf) -> PyResult<BufReader<File>> {
    let infile = File::open(in_file)
        .map_err(|e| PyIOError::new_err(format!("Failed to open {}: {e}", in_file.display())))?;
    Ok(BufReader::new(infile))
}

pub fn open_output(out_file: &PathBuf, overwrite: bool) -> PyResult<BufWriter<File>> {
    if out_file.exists() && !overwrite {
        return Err(PyIOError::new_err(format!(
            "Output file {} already exists (use overwrite=True to replace).",
            out_file.display()
        )));
    }

    let out_open = if overwrite {
        File::options()
            .write(true)
            .create(true)
            .truncate(true)
            .open(out_file)
    } else {
        File::options().write(true).create_new(true).open(out_file)
    };
    let outfile = out_open
        .map_err(|e| PyIOError::new_err(format!("Failed to create {}: {e}", out_file.display())))?;
    Ok(BufWriter::new(outfile))
}
