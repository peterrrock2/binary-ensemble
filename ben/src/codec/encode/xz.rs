use crate::io::writer::XBenEncoder;
use crate::BenVariant;
use std::io::{self, BufRead, Result, Write};
use xz2::stream::MtStreamBuilder;
use xz2::write::XzEncoder;

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
