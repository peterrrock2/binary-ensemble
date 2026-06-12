use super::paths::count_jsonl_lines;
use crate::codec::encode::{encode_ben_to_xben, encode_jsonl_to_ben, encode_jsonl_to_xben};
use crate::io::bundle::format::{AssignmentFormat, ASSET_TYPE_GRAPH, STANDARDIZED_NAME_GRAPH};
use crate::io::bundle::writer::BendlAppender;
use crate::io::bundle::{AddAssetOptions, BendlWriter};
use crate::io::reader::subsample::count_samples_from_file;
use crate::io::reader::BenWireFormat;
use crate::BenVariant;
use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, Result};
use std::path::Path;

/// After a finalized `.bendl` has been written, reopen it in append mode and attach the graph asset
/// in-place. This runs *after* the stream has finished, which is why we print "Adding graph..." at
/// this point.
pub(super) fn append_graph_asset(out_path: &str, graph_path: &Path) -> Result<()> {
    eprintln!("Adding graph...");
    let graph_bytes = std::fs::read(graph_path)
        .map_err(|e| io::Error::other(format!("failed to read graph {graph_path:?}: {e}")))?;

    let file = OpenOptions::new().read(true).write(true).open(out_path)?;
    let mut appender = BendlAppender::open(file).map_err(|e| io::Error::other(format!("{e}")))?;
    appender
        .add_asset(
            ASSET_TYPE_GRAPH,
            STANDARDIZED_NAME_GRAPH,
            &graph_bytes,
            AddAssetOptions::defaults().json(),
        )
        .map_err(|e| io::Error::other(format!("failed to add graph asset: {e}")))?;
    appender
        .commit()
        .map_err(|e| io::Error::other(format!("{e}")))?;
    Ok(())
}

/// Encode `input_path` (JSONL) to BEN inside a fresh `.bendl` file at `out_path` and then append
/// the graph as a post-stream asset.
pub(super) fn run_encode_bundle_with_graph(
    input_path: &Path,
    out_path: &str,
    variant: BenVariant,
    graph_path: &Path,
) -> Result<()> {
    // Validate the graph file is readable before we do any real work, so a bad --graph path doesn't
    // leave a half-written bundle behind.
    std::fs::metadata(graph_path)
        .map_err(|e| io::Error::other(format!("failed to stat graph {graph_path:?}: {e}")))?;

    let sample_count = count_jsonl_lines(input_path)?;

    let out_file = File::create(out_path)?;
    let bendl_writer = BendlWriter::new(out_file, AssignmentFormat::Ben)
        .map_err(|e| io::Error::other(format!("{e}")))?;
    let mut session = bendl_writer
        .into_stream_session()
        .map_err(|e| io::Error::other(format!("{e}")))?;
    {
        let reader = BufReader::new(File::open(input_path)?);
        encode_jsonl_to_ben(reader, &mut session, variant)?;
    }
    let bendl_writer = session.finish_into_writer(sample_count);
    bendl_writer
        .finish()
        .map_err(|e| io::Error::other(format!("{e}")))?;

    append_graph_asset(out_path, graph_path)
}

/// Encode `input_path` (JSONL or `.ben`) to XBEN inside a fresh `.bendl` file at `out_path` and
/// then append the graph as a post-stream asset.
#[allow(clippy::too_many_arguments)]
pub(super) fn run_xencode_bundle_with_graph(
    input_path: &Path,
    out_path: &str,
    variant: BenVariant,
    from_ben: bool,
    n_threads: Option<u32>,
    compression_level: Option<u32>,
    chunk_size: Option<usize>,
    block_size: Option<u64>,
    graph_path: &Path,
) -> Result<()> {
    std::fs::metadata(graph_path)
        .map_err(|e| io::Error::other(format!("failed to stat graph {graph_path:?}: {e}")))?;

    let sample_count: i64 = if from_ben {
        count_samples_from_file(input_path, BenWireFormat::Ben)? as i64
    } else {
        count_jsonl_lines(input_path)?
    };

    let out_file = File::create(out_path)?;
    let bendl_writer = BendlWriter::new(out_file, AssignmentFormat::Xben)
        .map_err(|e| io::Error::other(format!("{e}")))?;
    let mut session = bendl_writer
        .into_stream_session()
        .map_err(|e| io::Error::other(format!("{e}")))?;
    {
        let reader = BufReader::new(File::open(input_path)?);
        if from_ben {
            encode_ben_to_xben(
                reader,
                &mut session,
                n_threads,
                compression_level,
                chunk_size,
                block_size,
            )?;
        } else {
            encode_jsonl_to_xben(
                reader,
                &mut session,
                variant,
                n_threads,
                compression_level,
                chunk_size,
                block_size,
            )?;
        }
    }
    let bendl_writer = session.finish_into_writer(sample_count);
    bendl_writer
        .finish()
        .map_err(|e| io::Error::other(format!("{e}")))?;

    append_graph_asset(out_path, graph_path)
}
