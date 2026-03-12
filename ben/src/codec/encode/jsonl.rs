use crate::io::writer::{BenEncoder, XBenEncoder};
use crate::{log, logln, BenVariant};
use serde_json::Value;
use std::io::{BufRead, Result, Write};
use xz2::stream::MtStreamBuilder;
use xz2::write::XzEncoder;

pub fn encode_jsonl_to_xben<R: BufRead, W: Write>(
    reader: R,
    writer: W,
    variant: BenVariant,
    n_threads: Option<u32>,
    compression_level: Option<u32>,
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

    let mut line_num = 1;

    for line_result in reader.lines() {
        log!("Encoding line: {}\r", line_num);
        line_num += 1;
        let line = line_result?;
        let data: Value = serde_json::from_str(&line).expect("Error parsing JSON from line");

        ben_encoder.write_json_value(data)?;
    }

    logln!();
    logln!("Done!");

    Ok(())
}

pub fn encode_jsonl_to_ben<R: BufRead, W: Write>(
    reader: R,
    writer: W,
    variant: BenVariant,
) -> Result<()> {
    let mut line_num = 1;
    let mut ben_encoder = BenEncoder::new(writer, variant);
    for line_result in reader.lines() {
        log!("Encoding line: {}\r", line_num);
        line_num += 1;
        let line = line_result?;
        let data: Value = serde_json::from_str(&line).expect("Error parsing JSON from line");

        ben_encoder.write_json_value(data)?;
    }
    logln!();
    logln!("Done!");
    Ok(())
}
