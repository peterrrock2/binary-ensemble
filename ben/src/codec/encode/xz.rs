use crate::io::writer::XBenEncoder;
use crate::BenVariant;
use std::io::{self, BufRead, Result, Write};
use xz2::stream::MtStreamBuilder;
use xz2::write::XzEncoder;

/// Compress an arbitrary byte stream with XZ/LZMA2.
///
/// This is a general-purpose helper used by the XBEN tooling, but it can also
/// be used for plain XZ compression when BEN-specific framing is not needed.
///
/// # Arguments
///
/// * `reader` - The input byte stream to compress.
/// * `writer` - The destination for the compressed XZ bytes.
/// * `n_threads` - Optional XZ encoder thread count. When omitted, a safe
///   default is chosen.
/// * `compression_level` - Optional XZ compression level in the range `0..=9`.
///
/// # Returns
///
/// Returns `Ok(())` after the input stream has been fully compressed.
pub fn xz_compress<R: BufRead, W: Write>(
    mut reader: R,
    writer: W,
    n_threads: Option<u32>,
    compression_level: Option<u32>,
) -> Result<()> {
    let mut buff = [0; 4096];

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
    let mut encoder = XzEncoder::new_stream(writer, mt);

    while let Ok(count) = reader.read(&mut buff) {
        if count == 0 {
            break;
        }
        encoder.write_all(&buff[..count])?;
    }
    drop(encoder);
    Ok(())
}

/// Convert an existing BEN stream into an XBEN stream.
///
/// The input must begin with a BEN banner so that the variant can be preserved
/// in the compressed output.
///
/// # Arguments
///
/// * `reader` - The input BEN stream, including its banner.
/// * `writer` - The destination for the compressed XBEN bytes.
/// * `n_threads` - Optional XZ encoder thread count. When omitted, a safe
///   default is chosen.
/// * `compression_level` - Optional XZ compression level in the range `0..=9`.
///
/// # Returns
///
/// Returns `Ok(())` after the BEN stream has been translated and compressed.
pub fn encode_ben_to_xben<R: BufRead, W: Write>(
    mut reader: R,
    writer: W,
    n_threads: Option<u32>,
    compression_level: Option<u32>,
) -> Result<()> {
    let mut check_buffer = [0u8; 17];
    reader.read_exact(&mut check_buffer)?;

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

    let mut ben_encoder = match &check_buffer {
        b"STANDARD BEN FILE" => XBenEncoder::new(encoder, BenVariant::Standard),
        b"MKVCHAIN BEN FILE" => XBenEncoder::new(encoder, BenVariant::MkvChain),
        b"TWODELTA BEN FILE" => XBenEncoder::new(encoder, BenVariant::TwoDelta),
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid file format",
            ));
        }
    };

    ben_encoder.write_ben_file(reader)?;

    Ok(())
}
