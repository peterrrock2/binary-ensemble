//! Streaming-invariant soak tests.
//!
//! "Streaming, not slurping" is a core invariant of this workspace: ensembles are too large to
//! hold in memory, so every encode/decode path must process samples incrementally. Nothing else
//! in the suite *measures* that; every other harness uses small inputs, so an accidental
//! buffer-everything regression would pass the entire suite and only surface as an OOM on a real
//! multi-gigabyte ensemble.
//!
//! These tests pin the invariant directly: an encoder thread streams a multi-gigabyte *logical*
//! ensemble through an OS pipe (64 KiB of kernel backpressure) into a decoder, and the process's
//! peak RSS (`VmHWM`) must stay bounded. A slurping regression on either side would buffer
//! gigabytes and blow the bound unmissably.
//!
//! Linux-only (peak RSS is read from `/proc/self/status`) and `#[ignore]`-gated into the
//! slow/stress suite: multi-gigabyte logical streams take a few seconds.

#![cfg(target_os = "linux")]

use binary_ensemble::io::reader::BenStreamReader;
use binary_ensemble::io::writer::{BenStreamWriter, XzEncodeOptions};
use binary_ensemble::BenVariant;
use std::io::{BufReader, BufWriter};

/// Samples streamed per test.
const N_SAMPLES: usize = 200_000;
/// Nodes per assignment. 200k samples x 5k nodes x 2 bytes = 2 GB of logical assignment data.
const ASSIGNMENT_LEN: usize = 5_000;
/// Peak-RSS budget. True streaming peaks well under 100 MB; a slurping regression buffers the
/// 2 GB logical stream and exceeds this bound by an order of magnitude.
const MAX_PEAK_RSS_KB: u64 = 256 * 1024;

/// The process's lifetime peak resident set size in kilobytes, from `/proc/self/status` `VmHWM`.
fn peak_rss_kb() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").expect("read /proc/self/status");
    let line = status
        .lines()
        .find(|l| l.starts_with("VmHWM:"))
        .expect("VmHWM present in /proc/self/status");
    line.split_whitespace()
        .nth(1)
        .expect("VmHWM value field")
        .parse()
        .expect("VmHWM parses as kB")
}

/// A few distinct assignment templates so consecutive samples differ (no count-merging shortcut)
/// while staying cheap to produce. Runs of 50 keep each frame's RLE small, so the on-wire stream
/// is modest even though the decoded stream is gigabytes.
fn templates() -> Vec<Vec<u16>> {
    (0..4u16)
        .map(|k| {
            (0..ASSIGNMENT_LEN)
                .map(|j| ((j / 50) as u16 + k) % 40 + 1)
                .collect()
        })
        .collect()
}

/// Drive `N_SAMPLES` through an encoder thread, an OS pipe, and `decode`, asserting the decoded
/// totals and the peak-RSS bound.
fn assert_streaming_round_trip(
    encode: impl FnOnce(std::io::PipeWriter) + Send + 'static,
    decode: impl FnOnce(std::io::PipeReader) -> (usize, u64),
) {
    let (reader, writer) = std::io::pipe().expect("create pipe");

    let encoder_thread = std::thread::spawn(move || encode(writer));
    let (total_samples, total_nodes) = decode(reader);
    encoder_thread.join().expect("encoder thread");

    assert_eq!(total_samples, N_SAMPLES);
    assert_eq!(total_nodes, (N_SAMPLES * ASSIGNMENT_LEN) as u64);

    let peak = peak_rss_kb();
    assert!(
        peak < MAX_PEAK_RSS_KB,
        "peak RSS {peak} kB breaches the {MAX_PEAK_RSS_KB} kB streaming bound; \
         some encode/decode path is buffering the stream instead of streaming it"
    );
}

fn decode_counting<R: std::io::Read>(reader: R, from_xben: bool) -> (usize, u64) {
    let mut decoder = if from_xben {
        BenStreamReader::from_xben(reader).expect("open xben stream")
    } else {
        BenStreamReader::from_ben(reader).expect("open ben stream")
    }
    .silent(true);

    let mut total_samples = 0usize;
    let mut total_nodes = 0u64;
    decoder
        .for_each_assignment(|assignment, count| {
            total_samples += count as usize;
            total_nodes += assignment.len() as u64 * u64::from(count);
            Ok(true)
        })
        .expect("decode stream");
    (total_samples, total_nodes)
}

#[test]
#[ignore = "streaming soak: multi-gigabyte logical stream; run via the slow/stress gate"]
fn plain_ben_round_trip_streams_without_slurping() {
    let templates = templates();
    assert_streaming_round_trip(
        move |writer| {
            let mut encoder =
                BenStreamWriter::for_ben(BufWriter::new(writer), BenVariant::Standard)
                    .expect("open ben writer");
            for i in 0..N_SAMPLES {
                encoder
                    .write_assignment(templates[i % templates.len()].clone())
                    .expect("write assignment");
            }
            encoder.finish().expect("finish ben stream");
        },
        |reader| decode_counting(BufReader::new(reader), false),
    );
}

#[test]
#[ignore = "streaming soak: multi-gigabyte logical stream; run via the slow/stress gate"]
fn xben_round_trip_streams_without_slurping() {
    let templates = templates();
    assert_streaming_round_trip(
        move |writer| {
            // Compression level 1 keeps the xz dictionary near 1 MiB; the default level's 64 MiB
            // dictionary would dominate the RSS measurement and mask a slurping regression.
            let options = XzEncodeOptions::new()
                .with_n_threads(1)
                .with_compression_level(1);
            let mut encoder =
                BenStreamWriter::for_xben(BufWriter::new(writer), BenVariant::Standard, options)
                    .expect("open xben writer");
            for i in 0..N_SAMPLES {
                encoder
                    .write_assignment(templates[i % templates.len()].clone())
                    .expect("write assignment");
            }
            encoder.finish().expect("finish xben stream");
        },
        |reader| decode_counting(BufReader::new(reader), true),
    );
}
