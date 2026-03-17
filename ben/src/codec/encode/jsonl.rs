use crate::io::writer::{BenEncoder, XBenEncoder};
use crate::{progress, BenVariant};
use serde_json::Value;
use std::io::{self, BufRead, Result, Write};
use xz2::stream::MtStreamBuilder;
use xz2::write::XzEncoder;

/// Encode JSONL assignment records directly into an XBEN stream.
///
/// Each input line must be a JSON object with an `assignment` array. The output
/// stream begins with the standard BEN banner inside the compressed payload and
/// then stores each assignment in ben32 form.
///
/// # Arguments
///
/// * `reader` - A JSONL input stream with one assignment record per line.
/// * `writer` - The destination for the compressed XBEN bytes.
/// * `variant` - The BEN variant to use inside the XBEN payload.
/// * `n_threads` - Optional XZ encoder thread count. When omitted, a safe
///   default is chosen.
/// * `compression_level` - Optional XZ compression level in the range `0..=9`.
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
) -> Result<()> {
    let mut n_cpus: u32 = n_threads.unwrap_or(1);
    n_cpus = n_cpus
        .min(
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1) as u32,
        )
        .max(1);

    let level = compression_level.unwrap_or(9).clamp(0, 9);

    let mt = MtStreamBuilder::new()
        .threads(n_cpus)
        .preset(level)
        .block_size(0)
        .encoder()
        .expect("init MT encoder");
    let encoder = XzEncoder::new_stream(writer, mt);
    let mut ben_encoder = XBenEncoder::new(encoder, variant);
    if let Some(cs) = chunk_size {
        ben_encoder = ben_encoder.with_chunk_size(cs);
    }

    let mut line_num = 1;

    for line_result in reader.lines() {
        progress!("Encoding line: {}\r", line_num);
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

    tracing::trace!("");
    tracing::trace!("Done!");

    Ok(())
}

/// Encode JSONL assignment records into an uncompressed BEN file.
///
/// The input is expected to contain one JSON object per line with an
/// `assignment` array. The `sample` field is ignored because BEN sample order is
/// determined by the stream position.
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
    let mut line_num = 1;
    let mut ben_encoder = BenEncoder::new(writer, variant);
    for line_result in reader.lines() {
        progress!("Encoding line: {}\r", line_num);
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
    tracing::trace!("");
    tracing::trace!("Done!");
    Ok(())
}
