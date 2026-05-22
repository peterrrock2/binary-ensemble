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
    let graph_bytes = std::fs::read(graph_path).map_err(|e| {
        io::Error::new(
            io::ErrorKind::Other,
            format!("failed to read graph {graph_path:?}: {e}"),
        )
    })?;

    let file = OpenOptions::new().read(true).write(true).open(out_path)?;
    let mut appender = BendlAppender::open(file)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{e}")))?;
    appender
        .add_asset(
            ASSET_TYPE_GRAPH,
            STANDARDIZED_NAME_GRAPH,
            &graph_bytes,
            AddAssetOptions::defaults().json(),
        )
        .map_err(|e| {
            io::Error::new(
                io::ErrorKind::Other,
                format!("failed to add graph asset: {e}"),
            )
        })?;
    appender
        .commit()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{e}")))?;
    Ok(())
}

/// Encode `input_path` (JSONL) to BEN inside a fresh `.bendl` bundle at `out_path` and then append
/// the graph as a post-stream asset.
pub(super) fn run_encode_bundle_with_graph(
    input_path: &Path,
    out_path: &str,
    variant: BenVariant,
    graph_path: &Path,
) -> Result<()> {
    // Validate the graph file is readable before we do any real work, so a bad --graph path doesn't
    // leave a half-written bundle behind.
    std::fs::metadata(graph_path).map_err(|e| {
        io::Error::new(
            io::ErrorKind::Other,
            format!("failed to stat graph {graph_path:?}: {e}"),
        )
    })?;

    let sample_count = count_jsonl_lines(input_path)?;

    let out_file = File::create(out_path)?;
    let bendl_writer = BendlWriter::new(out_file, AssignmentFormat::Ben)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{e}")))?;
    let mut session = bendl_writer
        .into_stream_session()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{e}")))?;
    {
        let reader = BufReader::new(File::open(input_path)?);
        encode_jsonl_to_ben(reader, &mut session, variant)?;
    }
    let bendl_writer = session.finish_into_writer(sample_count);
    bendl_writer
        .finish()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{e}")))?;

    append_graph_asset(out_path, graph_path)
}

/// Encode `input_path` (JSONL or `.ben`) to XBEN inside a fresh `.bendl` bundle at `out_path` and
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
    std::fs::metadata(graph_path).map_err(|e| {
        io::Error::new(
            io::ErrorKind::Other,
            format!("failed to stat graph {graph_path:?}: {e}"),
        )
    })?;

    let sample_count: i64 = if from_ben {
        count_samples_from_file(input_path, BenWireFormat::Ben)? as i64
    } else {
        count_jsonl_lines(input_path)?
    };

    let out_file = File::create(out_path)?;
    let bendl_writer = BendlWriter::new(out_file, AssignmentFormat::Xben)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{e}")))?;
    let mut session = bendl_writer
        .into_stream_session()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{e}")))?;
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
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{e}")))?;

    append_graph_asset(out_path, graph_path)
}

#[cfg(test)]
mod tests {
    //! In-process tests for the `--graph` bundle dispatchers. CLI subprocess tests
    //! (`tests/test_cli.rs`) confirm argv parsing and exit codes; coverage instrumentation does
    //! not follow subprocess boundaries, so unit tests here are required to actually exercise
    //! these functions' branches.
    use super::*;
    use crate::test_utils::unique_path;
    use std::io::Write;

    fn canonical_jsonl() -> &'static [u8] {
        b"{\"assignment\":[1,1,2],\"sample\":1}\n{\"assignment\":[2,2,3],\"sample\":2}\n"
    }

    fn canonical_graph() -> &'static [u8] {
        b"{\"nodes\":3,\"edges\":[[0,1],[1,2]]}"
    }

    /// Allocate a fresh per-test directory under the system temp dir. Returned path is created
    /// on disk so callers can write files into it immediately.
    fn fresh_temp_dir(label: &str) -> std::path::PathBuf {
        let p = unique_path(label);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn run_encode_bundle_with_graph_produces_readable_bundle() {
        let temp = fresh_temp_dir("encode-bundle-graph");
        let input = temp.join("input.jsonl");
        std::fs::write(&input, canonical_jsonl()).unwrap();
        let graph = temp.join("graph.json");
        std::fs::write(&graph, canonical_graph()).unwrap();
        let out = temp.join("out.bendl");
        let out_str = out.to_string_lossy().into_owned();

        run_encode_bundle_with_graph(&input, &out_str, BenVariant::Standard, &graph).unwrap();

        let reader = crate::io::bundle::BendlReader::open(
            std::fs::File::open(&out).expect("open bundle"),
        )
        .expect("open bundle");
        assert!(reader.is_finalized());
        assert!(reader.find_asset_by_name("graph.json").is_some());
    }

    #[test]
    fn run_xencode_bundle_with_graph_from_jsonl_input_succeeds() {
        // jsonl_and_xben dispatch arm: from_ben=false, encode_jsonl_to_xben path.
        let temp = fresh_temp_dir("xencode-bundle-graph-jsonl");
        let input = temp.join("input.jsonl");
        std::fs::write(&input, canonical_jsonl()).unwrap();
        let graph = temp.join("graph.json");
        std::fs::write(&graph, canonical_graph()).unwrap();
        let out = temp.join("out.bendl");
        let out_str = out.to_string_lossy().into_owned();

        run_xencode_bundle_with_graph(
            &input,
            &out_str,
            BenVariant::Standard,
            /* from_ben */ false,
            Some(1),
            Some(1),
            None,
            None,
            &graph,
        )
        .unwrap();

        let reader = crate::io::bundle::BendlReader::open(
            std::fs::File::open(&out).expect("open bundle"),
        )
        .expect("open bundle");
        assert!(reader.is_finalized());
        assert!(reader.find_asset_by_name("graph.json").is_some());
    }

    #[test]
    fn run_xencode_bundle_with_graph_from_ben_input_succeeds() {
        // from_ben=true dispatch arm — exercises the encode_ben_to_xben branch in the function
        // body, the gap the CLI subprocess tests can't touch under llvm-cov.
        let temp = fresh_temp_dir("xencode-bundle-graph-ben");

        let jsonl = temp.join("input.jsonl");
        std::fs::write(&jsonl, canonical_jsonl()).unwrap();
        let ben_path = temp.join("input.ben");
        {
            use std::io::BufReader;
            let reader = BufReader::new(std::fs::File::open(&jsonl).unwrap());
            let mut writer = std::fs::File::create(&ben_path).unwrap();
            encode_jsonl_to_ben(reader, &mut writer, BenVariant::Standard).unwrap();
            writer.flush().unwrap();
        }

        let graph = temp.join("graph.json");
        std::fs::write(&graph, canonical_graph()).unwrap();
        let out = temp.join("out.bendl");
        let out_str = out.to_string_lossy().into_owned();

        run_xencode_bundle_with_graph(
            &ben_path,
            &out_str,
            BenVariant::Standard,
            /* from_ben */ true,
            Some(1),
            Some(1),
            None,
            None,
            &graph,
        )
        .unwrap();

        let reader = crate::io::bundle::BendlReader::open(
            std::fs::File::open(&out).expect("open bundle"),
        )
        .expect("open bundle");
        assert!(reader.is_finalized());
        assert!(reader.find_asset_by_name("graph.json").is_some());
    }

    #[test]
    fn run_encode_bundle_with_graph_rejects_missing_graph_file() {
        let temp = fresh_temp_dir("encode-bundle-missing-graph");
        let input = temp.join("input.jsonl");
        std::fs::write(&input, canonical_jsonl()).unwrap();
        let nonexistent_graph = temp.join("does-not-exist.json");
        let out = temp.join("out.bendl");
        let out_str = out.to_string_lossy().into_owned();

        let err = run_encode_bundle_with_graph(
            &input,
            &out_str,
            BenVariant::Standard,
            &nonexistent_graph,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("graph") || err.kind() == io::ErrorKind::NotFound,
            "expected missing-graph error, got {err}"
        );
    }

    #[test]
    fn append_graph_asset_rejects_missing_graph_path() {
        let temp = fresh_temp_dir("append-graph-missing");
        let input = temp.join("input.jsonl");
        std::fs::write(&input, canonical_jsonl()).unwrap();
        let graph = temp.join("graph.json");
        std::fs::write(&graph, canonical_graph()).unwrap();
        let out = temp.join("out.bendl");
        let out_str = out.to_string_lossy().into_owned();
        run_encode_bundle_with_graph(&input, &out_str, BenVariant::Standard, &graph).unwrap();

        // append_graph_asset is the function under test (separate from the dispatchers, which
        // already validated graph existence at the top).
        let missing = temp.join("missing.json");
        let err = append_graph_asset(&out_str, &missing).unwrap_err();
        assert!(
            err.to_string().contains("graph") || err.kind() == io::ErrorKind::NotFound,
            "expected missing-graph error, got {err}"
        );
    }
}
