use crate::codec::encode::xz::{build_mt_stream, resolve_threads};
use crate::io::writer::BenStreamWriter;
use crate::progress::Spinner;
use crate::BenVariant;
use serde_json::Value;
use std::io::{self, BufRead, Result, Write};
use xz2::write::XzEncoder;

/// Encode JSONL assignment records directly into an XBEN stream.
///
/// Each input line must be a JSON object with an `assignment` array. The output stream begins with
/// the standard BEN banner inside the compressed payload and then stores each assignment in ben32
/// form.
///
/// # Arguments
///
/// * `reader` - A JSONL input stream with one assignment record per line.
/// * `writer` - The destination for the compressed XBEN bytes.
/// * `variant` - The BEN variant to use inside the XBEN payload.
/// * `n_threads` - Optional XZ encoder thread count. Defaults to `1` (single-threaded) when `None`.
///   Values larger than the host's available parallelism are silently clamped down.
/// * `compression_level` - Optional XZ compression level in the range `0..=9`.
/// * `chunk_size` - Optional TwoDelta columnar chunk size; ignored for Standard and MkvChain
///   variants.
/// * `block_size` - Optional per-block size in bytes for the MT encoder. `None` defaults to
///   [`crate::codec::encode::xz::XZ_DEFAULT_MT_BLOCK_SIZE`] when threads > 1, or `0` (liblzma auto)
///   for single-thread runs.
///
/// # Returns
///
/// Returns `Ok(())` after all JSONL lines have been encoded and written.
pub fn encode_jsonl_to_xben<R: BufRead, W: Write>(
    reader: R,
    writer: W,
    variant: BenVariant,
    n_threads: Option<u32>,
    compression_level: Option<u32>,
    chunk_size: Option<usize>,
    block_size: Option<u64>,
) -> Result<()> {
    let n_cpus = resolve_threads(n_threads);
    let level = compression_level.unwrap_or(9).clamp(0, 9);
    let mt = build_mt_stream(n_cpus, level, block_size)?;
    let encoder = XzEncoder::new_stream(writer, mt);

    let mut ben_encoder = BenStreamWriter::for_xben_with_encoder(encoder, variant, chunk_size)?;

    let mut line_num = 1u64;
    let spinner = Spinner::new("Encoding line");

    for line_result in reader.lines() {
        spinner.set_count(line_num);
        line_num += 1;
        let line = line_result?;
        let data: Value = serde_json::from_str(&line).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Error parsing JSON from line: {e}"),
            )
        })?;

        ben_encoder.write_json_value(data)?;
    }

    ben_encoder.finish()?;
    Ok(())
}

/// Encode JSONL assignment records into an uncompressed BEN file.
///
/// The input is expected to contain one JSON object per line with an `assignment` array. The
/// `sample` field is ignored because BEN sample order is determined by the stream position.
///
/// # Arguments
///
/// * `reader` - A JSONL input stream with one assignment record per line.
/// * `writer` - The destination for the BEN bytes.
/// * `variant` - The BEN variant to use when writing the output stream.
///
/// # Returns
///
/// Returns `Ok(())` after all JSONL lines have been encoded and written.
pub fn encode_jsonl_to_ben<R: BufRead, W: Write>(
    reader: R,
    writer: W,
    variant: BenVariant,
) -> Result<()> {
    let mut line_num = 1u64;
    let spinner = Spinner::new("Encoding line");
    let mut ben_encoder = BenStreamWriter::for_ben(writer, variant)?;
    for line_result in reader.lines() {
        spinner.set_count(line_num);
        line_num += 1;
        let line = line_result?;
        let data: Value = serde_json::from_str(&line).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Error parsing JSON from line: {e}"),
            )
        })?;

        ben_encoder.write_json_value(data)?;
    }
    ben_encoder.finish()?;
    Ok(())
}
