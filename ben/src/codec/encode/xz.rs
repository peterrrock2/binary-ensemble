use crate::codec::encode::errors::EncodeError;
use crate::format::banners::{variant_from_banner, BANNER_LEN};
use crate::format::FormatError;
use crate::io::writer::BenStreamWriter;
use std::io::{self, BufRead, Cursor, Read, Result, Write};
use xz2::stream::{MtStreamBuilder, Stream};
use xz2::write::XzEncoder;

/// Default per-block size used by the multithreaded XZ encoder when the caller does not pass an
/// explicit `block_size`.
///
/// liblzma's `block_size = 0` means "auto" (`3 × dict_size`), which at preset 9 is ~192 MiB — far
/// too coarse for streaming inputs to fan out across worker threads. 16 MiB strikes a balance
/// between scaling thread utilization on medium ensembles and keeping per-block dictionary reuse
/// mostly intact.
pub const XZ_DEFAULT_MT_BLOCK_SIZE: u64 = 16 * 1024 * 1024;

/// Resolve `n_threads` against the host's available parallelism.
pub(crate) fn resolve_threads(n_threads: Option<u32>) -> u32 {
    n_threads.unwrap_or(1).min(host_parallelism()).max(1)
}

/// Number of cores reported by `std::thread::available_parallelism`, or `1` if the platform cannot
/// answer.
fn host_parallelism() -> u32 {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1) as u32
}

/// Convert a user-supplied signed thread count into the unsigned count the encoder expects.
///
/// CLI and Python users want sklearn-style sentinel semantics:
///
/// - `n < 0` (typically `-1`) means "use every available core".
/// - `n == 0` is treated as `1` to avoid silently disabling work.
/// - `n > 0` is the literal request, clamped to host parallelism.
///
/// Pass the result on to library APIs that take `Option<u32>`.
pub fn cpus_from_signed(n: i32) -> u32 {
    if n < 0 {
        host_parallelism()
    } else {
        (n as u32).max(1).min(host_parallelism())
    }
}

/// Build a multithreaded XZ encoder stream with the project's default `block_size` policy applied.
///
/// When `block_size` is `Some(n)`, that exact byte count is passed to liblzma. When it is `None`,
/// we default to [`XZ_DEFAULT_MT_BLOCK_SIZE`] for `n_threads > 1` and to `0` (liblzma's "auto") for
/// the single-thread case so single-thread encoding does not pay any block-overhead cost.
pub(crate) fn build_mt_stream(
    n_threads: u32,
    level: u32,
    block_size: Option<u64>,
) -> Result<Stream> {
    let resolved_block_size = match block_size {
        Some(n) => n,
        None if n_threads > 1 => XZ_DEFAULT_MT_BLOCK_SIZE,
        None => 0,
    };

    MtStreamBuilder::new()
        .threads(n_threads)
        .preset(level)
        .block_size(resolved_block_size)
        .encoder()
        .map_err(|e| io::Error::from(EncodeError::XzInit(e)))
}

/// Compress an arbitrary byte stream with XZ/LZMA2.
///
/// This is a general-purpose helper used by the XBEN tooling, but it can also be used for plain XZ
/// compression when BEN-specific framing is not needed.
///
/// # Arguments
///
/// * `reader` - The input byte stream to compress.
/// * `writer` - The destination for the compressed XZ bytes.
/// * `n_threads` - Optional XZ encoder thread count. Defaults to `1` (single-threaded) when `None`.
///   Values larger than the host's available parallelism are silently clamped down.
/// * `compression_level` - Optional XZ compression level in the range `0..=9`.
/// * `block_size` - Optional per-block size in bytes for the MT encoder. `None` defaults to
///   [`XZ_DEFAULT_MT_BLOCK_SIZE`] when threads > 1, or `0` (liblzma auto) for single-thread runs.
///   Smaller blocks improve thread fan-out at a slight compression-ratio cost.
///
/// # Returns
///
/// Returns `Ok(())` after the input stream has been fully compressed.
pub fn xz_compress<R: BufRead, W: Write>(
    mut reader: R,
    writer: W,
    n_threads: Option<u32>,
    compression_level: Option<u32>,
    block_size: Option<u64>,
) -> Result<()> {
    let mut buff = [0; 4096];

    let n_cpus = resolve_threads(n_threads);
    let level = compression_level.unwrap_or(9).clamp(0, 9);

    let mt = build_mt_stream(n_cpus, level, block_size)?;
    let mut encoder = XzEncoder::new_stream(writer, mt);

    loop {
        let count = reader.read(&mut buff)?;
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
/// The input must begin with a BEN banner so that the variant can be preserved in the compressed
/// output.
///
/// # Arguments
///
/// * `reader` - The input BEN stream, including its banner.
/// * `writer` - The destination for the compressed XBEN bytes.
/// * `n_threads` - Optional XZ encoder thread count. Defaults to `1` (single-threaded) when `None`.
///   Values larger than the host's available parallelism are silently clamped down.
/// * `compression_level` - Optional XZ compression level in the range `0..=9`.
/// * `chunk_size` - Optional TwoDelta columnar chunk size; ignored for Standard and MkvChain
///   variants.
/// * `block_size` - Optional per-block size in bytes for the MT encoder. `None` defaults to
///   [`XZ_DEFAULT_MT_BLOCK_SIZE`] when threads > 1, or `0` (liblzma auto) for single-thread runs.
///
/// # Returns
///
/// Returns `Ok(())` after the BEN stream has been translated and compressed.
pub fn encode_ben_to_xben<R: BufRead, W: Write>(
    mut reader: R,
    writer: W,
    n_threads: Option<u32>,
    compression_level: Option<u32>,
    chunk_size: Option<usize>,
    block_size: Option<u64>,
) -> Result<()> {
    let mut check_buffer = [0u8; BANNER_LEN];
    reader.read_exact(&mut check_buffer)?;

    let n_cpus = resolve_threads(n_threads);
    let level = compression_level.unwrap_or(9).clamp(0, 9);

    let mt = build_mt_stream(n_cpus, level, block_size)?;
    let encoder = XzEncoder::new_stream(writer, mt);

    let variant = variant_from_banner(&check_buffer).ok_or_else(|| {
        io::Error::from(FormatError::UnknownBanner {
            actual: check_buffer.to_vec(),
        })
    })?;
    let mut ben_encoder = BenStreamWriter::for_xben_with_encoder(encoder, variant, chunk_size)?;
    ben_encoder.ingest_ben_stream(Cursor::new(check_buffer).chain(reader))?;
    ben_encoder.finish()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpus_from_signed_negative_means_all_cores() {
        let host = host_parallelism();
        assert_eq!(cpus_from_signed(-1), host);
        assert_eq!(cpus_from_signed(-100), host);
    }

    #[test]
    fn cpus_from_signed_zero_clamps_to_one() {
        assert_eq!(cpus_from_signed(0), 1);
    }

    #[test]
    fn cpus_from_signed_positive_clamps_to_host() {
        let host = host_parallelism();
        assert_eq!(cpus_from_signed(1), 1);
        assert_eq!(cpus_from_signed(i32::MAX), host);
    }
}
