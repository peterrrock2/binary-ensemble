//! CLI front-end for the `.bendl` bundle container.
//!
//! Exposes four subcommands:
//!
//! - `create`  — wrap a `.ben` / `.xben` assignment stream plus optional
//!   asset files into a finalized `.bendl` bundle.
//! - `inspect` — print the header and directory of a `.bendl` file.
//! - `extract` — copy the embedded stream region or a named asset out
//!   of a bundle to disk.
//! - `append`  — add new asset files to an already-finalized bundle
//!   without rewriting the stream.

use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Seek, Write};
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

use crate::cli::common::{check_overwrite, set_verbose};
use crate::io::bundle::format::{
    AssignmentFormat, ASSET_FLAG_CHECKSUM, ASSET_FLAG_JSON, ASSET_FLAG_XZ, ASSET_TYPE_CUSTOM,
    ASSET_TYPE_GRAPH, ASSET_TYPE_METADATA, ASSET_TYPE_RELABEL_MAP,
};
use crate::io::bundle::writer::BendlAppender;
use crate::io::bundle::{AddAssetOptions, BendlReader, BendlWriteError, BendlWriter};
use crate::io::reader::subsample::count_samples_from_file;

/// Parsed form of a `name=path` option such as `--asset myblob=/tmp/x`.
#[derive(Debug, Clone)]
struct NamedAsset {
    name: String,
    path: PathBuf,
}

impl std::str::FromStr for NamedAsset {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (name, path) = s
            .split_once('=')
            .ok_or_else(|| format!("expected NAME=PATH, got {s:?}"))?;
        if name.is_empty() {
            return Err("custom asset name must be non-empty".to_string());
        }
        Ok(NamedAsset {
            name: name.to_string(),
            path: PathBuf::from(path),
        })
    }
}

/// `bendl` CLI entry point.
#[derive(Parser, Debug)]
#[command(
    name = "bendl",
    about = "Create, inspect, extract from, and append to .bendl bundle files.",
    version
)]
struct Args {
    /// Enable verbose tracing output.
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Package a `.ben` or `.xben` stream (plus optional assets) into a `.bendl`.
    Create(CreateArgs),
    /// Print the header and directory of a `.bendl` file.
    Inspect(InspectArgs),
    /// Extract the embedded stream or a named asset to a file.
    Extract(ExtractArgs),
    /// Append new assets to an already-finalized `.bendl` bundle.
    Append(AppendArgs),
}

#[derive(Parser, Debug)]
struct CreateArgs {
    /// Path to the `.ben` or `.xben` assignment stream to embed.
    /// File extension chooses the container format.
    #[arg(short = 'i', long)]
    input: PathBuf,
    /// Destination `.bendl` path.
    #[arg(short = 'o', long)]
    output: PathBuf,
    /// Optional `graph.json` asset path. Will be stored under the
    /// canonical name `graph.json` and xz-compressed by default.
    #[arg(long)]
    graph: Option<PathBuf>,
    /// Optional `metadata.json` asset path. Stored under canonical name.
    #[arg(long)]
    metadata: Option<PathBuf>,
    /// Optional `relabel_map.json` asset path. Stored under canonical name.
    #[arg(long)]
    relabel_map: Option<PathBuf>,
    /// Additional custom assets, specified as `NAME=PATH`. May be repeated.
    #[arg(long = "asset")]
    assets: Vec<NamedAsset>,
    /// Overwrite the output file if it already exists.
    #[arg(short = 'w', long)]
    overwrite: bool,
    /// Store `graph.json` raw instead of compressing it.
    #[arg(long)]
    graph_raw: bool,
}

#[derive(Parser, Debug)]
struct InspectArgs {
    /// `.bendl` file to inspect.
    input: PathBuf,
}

#[derive(Parser, Debug)]
struct ExtractArgs {
    /// `.bendl` file to extract from.
    input: PathBuf,
    /// Output file path for the extracted bytes.
    #[arg(short = 'o', long)]
    output: PathBuf,
    /// Extract the embedded assignment stream region verbatim. Mutually
    /// exclusive with `--asset`.
    #[arg(long, conflicts_with = "asset")]
    stream: bool,
    /// Name of the asset to extract (e.g. `graph.json`). If the asset is
    /// xz-compressed, the extracted file contains the decompressed bytes.
    #[arg(long)]
    asset: Option<String>,
    /// Overwrite the output file if it already exists.
    #[arg(short = 'w', long)]
    overwrite: bool,
}

#[derive(Parser, Debug)]
struct AppendArgs {
    /// `.bendl` file to append to. Must be finalized (`complete == 1`).
    input: PathBuf,
    /// Optional `graph.json` asset path to add.
    #[arg(long)]
    graph: Option<PathBuf>,
    /// Optional `metadata.json` asset path to add.
    #[arg(long)]
    metadata: Option<PathBuf>,
    /// Optional `relabel_map.json` asset path to add.
    #[arg(long)]
    relabel_map: Option<PathBuf>,
    /// Additional custom assets, specified as `NAME=PATH`. May be repeated.
    #[arg(long = "asset")]
    assets: Vec<NamedAsset>,
    /// Store `graph.json` raw instead of compressing it.
    #[arg(long)]
    graph_raw: bool,
}

/// Parse CLI arguments and execute the selected subcommand.
pub fn run() {
    let args = Args::parse();
    set_verbose(args.verbose);

    let result = match args.command {
        Command::Create(a) => run_create(a),
        Command::Inspect(a) => run_inspect(a),
        Command::Extract(a) => run_extract(a),
        Command::Append(a) => run_append(a),
    };

    if let Err(err) = result {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}

/// Detect the container format of `path` from its extension.
fn format_from_path(path: &Path) -> Result<AssignmentFormat, String> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("ben") => Ok(AssignmentFormat::Ben),
        Some("xben") => Ok(AssignmentFormat::Xben),
        other => Err(format!(
            "unable to determine assignment format from extension {other:?}; \
             expected .ben or .xben"
        )),
    }
}

/// `mode` argument expected by `count_samples_from_file`.
fn mode_str(format: AssignmentFormat) -> &'static str {
    match format {
        AssignmentFormat::Ben => "ben",
        AssignmentFormat::Xben => "xben",
    }
}

fn run_create(args: CreateArgs) -> Result<(), String> {
    let format = format_from_path(&args.input)?;
    check_overwrite(
        args.output.to_str().ok_or("non-utf8 output path")?,
        args.overwrite,
    )
    .map_err(|e| format!("{e}"))?;

    // Count samples up front so we can patch the header at finalize time.
    // This pre-scan is O(stream size); the second pass streams bytes directly.
    let sample_count: i64 = count_samples_from_file(&args.input, mode_str(format))
        .map_err(|e| format!("failed to count samples in {:?}: {e}", args.input))?
        as i64;

    let out_file = File::create(&args.output)
        .map_err(|e| format!("failed to create {:?}: {e}", args.output))?;
    let mut writer = BendlWriter::new(out_file, format)
        .map_err(|e| format!("failed to initialize bundle writer: {e}"))?;

    // Add singleton assets first, in canonical order.
    if let Some(ref path) = args.metadata {
        add_file_asset(
            &mut writer,
            ASSET_TYPE_METADATA,
            "metadata.json",
            path,
            AddAssetOptions::defaults().json(),
        )?;
    }
    if let Some(ref path) = args.graph {
        let opts = if args.graph_raw {
            AddAssetOptions::defaults().json().raw()
        } else {
            AddAssetOptions::defaults().json()
        };
        add_file_asset(&mut writer, ASSET_TYPE_GRAPH, "graph.json", path, opts)?;
    }
    if let Some(ref path) = args.relabel_map {
        add_file_asset(
            &mut writer,
            ASSET_TYPE_RELABEL_MAP,
            "relabel_map.json",
            path,
            AddAssetOptions::defaults().json(),
        )?;
    }
    for NamedAsset { name, path } in &args.assets {
        add_file_asset(
            &mut writer,
            ASSET_TYPE_CUSTOM,
            name,
            path,
            AddAssetOptions::defaults(),
        )?;
    }

    // Stream phase: copy bytes from the input file directly into the
    // bundle's stream region. This preserves the exact BEN/XBEN bytes.
    {
        let mut handle = writer
            .begin_stream()
            .map_err(|e| format!("failed to open stream region: {e}"))?;
        let mut input = BufReader::new(
            File::open(&args.input).map_err(|e| format!("failed to open {:?}: {e}", args.input))?,
        );
        io::copy(&mut input, &mut handle)
            .map_err(|e| format!("failed to copy assignment stream: {e}"))?;
        handle
            .finish(sample_count)
            .map_err(|e| format!("failed to close stream region: {e}"))?;
    }

    writer
        .finish()
        .map_err(|e| format!("failed to finalize bundle: {e}"))?;

    eprintln!(
        "Wrote {:?} ({} samples, format = {:?})",
        args.output, sample_count, format
    );
    Ok(())
}

fn add_file_asset<W: Write + Seek>(
    writer: &mut BendlWriter<W>,
    asset_type: u16,
    name: &str,
    path: &Path,
    options: AddAssetOptions,
) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("failed to read {path:?}: {e}"))?;
    writer
        .add_asset(asset_type, name, &bytes, options)
        .map_err(|e: BendlWriteError| format!("failed to add asset {name:?}: {e}"))
}

fn run_inspect(args: InspectArgs) -> Result<(), String> {
    let file =
        File::open(&args.input).map_err(|e| format!("failed to open {:?}: {e}", args.input))?;
    let reader = BendlReader::open(BufReader::new(file))
        .map_err(|e| format!("failed to parse bundle header: {e}"))?;

    let header = reader.header();
    println!("file:              {}", args.input.display());
    println!(
        "version:           {}.{}",
        header.major_version, header.minor_version
    );
    println!("complete:          {}", reader.is_complete());
    println!(
        "assignment_format: {}",
        match reader.assignment_format() {
            Some(AssignmentFormat::Ben) => "ben",
            Some(AssignmentFormat::Xben) => "xben",
            None => "unknown",
        }
    );
    println!(
        "sample_count:      {}",
        match reader.sample_count() {
            Some(n) => n.to_string(),
            None => "<unknown>".to_string(),
        }
    );
    println!(
        "stream:            offset={} len={}",
        header.stream_offset, header.stream_len
    );
    println!(
        "directory:         offset={} len={}",
        header.directory_offset, header.directory_len
    );

    let entries = reader.assets();
    println!("assets:            {} entries", entries.len());
    for entry in entries {
        let mut flag_parts: Vec<&str> = Vec::new();
        if entry.asset_flags & ASSET_FLAG_JSON != 0 {
            flag_parts.push("json");
        }
        if entry.asset_flags & ASSET_FLAG_XZ != 0 {
            flag_parts.push("xz");
        }
        if entry.asset_flags & ASSET_FLAG_CHECKSUM != 0 {
            flag_parts.push("checksum");
        }
        let flag_str = if flag_parts.is_empty() {
            "-".to_string()
        } else {
            flag_parts.join(",")
        };
        println!(
            "  type={:<4} name={:<24} offset={:<10} len={:<10} flags={}",
            entry.asset_type, entry.name, entry.payload_offset, entry.payload_len, flag_str
        );
    }

    Ok(())
}

fn run_extract(args: ExtractArgs) -> Result<(), String> {
    if !args.stream && args.asset.is_none() {
        return Err("extract requires either --stream or --asset <name>".to_string());
    }
    check_overwrite(
        args.output.to_str().ok_or("non-utf8 output path")?,
        args.overwrite,
    )
    .map_err(|e| format!("{e}"))?;

    let file =
        File::open(&args.input).map_err(|e| format!("failed to open {:?}: {e}", args.input))?;
    let mut reader = BendlReader::open(BufReader::new(file))
        .map_err(|e| format!("failed to parse bundle header: {e}"))?;

    let mut out = BufWriter::new(
        File::create(&args.output)
            .map_err(|e| format!("failed to create {:?}: {e}", args.output))?,
    );

    if args.stream {
        let mut stream = reader
            .assignment_stream_reader()
            .map_err(|e| format!("failed to open stream region: {e}"))?;
        io::copy(&mut stream, &mut out).map_err(|e| format!("failed to copy stream bytes: {e}"))?;
    } else {
        // asset is Some — validated by the early return above.
        let name = args.asset.unwrap();
        let entry = reader
            .find_asset_by_name(&name)
            .cloned()
            .ok_or_else(|| format!("no asset named {name:?} in bundle"))?;
        let mut asset = reader
            .asset_reader(&entry)
            .map_err(|e| format!("failed to open asset {name:?}: {e}"))?;
        io::copy(&mut asset, &mut out).map_err(|e| format!("failed to copy asset bytes: {e}"))?;
    }

    out.flush().map_err(|e| format!("flush failed: {e}"))?;
    Ok(())
}

fn run_append(args: AppendArgs) -> Result<(), String> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&args.input)
        .map_err(|e| format!("failed to open {:?} for read+write: {e}", args.input))?;
    let mut appender =
        BendlAppender::open(file).map_err(|e| format!("failed to open appender: {e}"))?;

    let mut added = 0usize;
    if let Some(ref path) = args.metadata {
        append_file_asset(
            &mut appender,
            ASSET_TYPE_METADATA,
            "metadata.json",
            path,
            AddAssetOptions::defaults().json(),
        )?;
        added += 1;
    }
    if let Some(ref path) = args.graph {
        let opts = if args.graph_raw {
            AddAssetOptions::defaults().json().raw()
        } else {
            AddAssetOptions::defaults().json()
        };
        append_file_asset(&mut appender, ASSET_TYPE_GRAPH, "graph.json", path, opts)?;
        added += 1;
    }
    if let Some(ref path) = args.relabel_map {
        append_file_asset(
            &mut appender,
            ASSET_TYPE_RELABEL_MAP,
            "relabel_map.json",
            path,
            AddAssetOptions::defaults().json(),
        )?;
        added += 1;
    }
    for NamedAsset { name, path } in &args.assets {
        append_file_asset(
            &mut appender,
            ASSET_TYPE_CUSTOM,
            name,
            path,
            AddAssetOptions::defaults(),
        )?;
        added += 1;
    }

    if added == 0 {
        // Nothing to do; leave the file untouched.
        appender.abort();
        eprintln!("No assets specified; bundle is unchanged.");
        return Ok(());
    }

    appender
        .commit()
        .map_err(|e| format!("failed to commit append: {e}"))?;
    eprintln!("Appended {added} asset(s) to {:?}", args.input);
    Ok(())
}

fn append_file_asset<W: Read + Write + Seek + crate::io::bundle::writer::BendlTruncate>(
    appender: &mut BendlAppender<W>,
    asset_type: u16,
    name: &str,
    path: &Path,
    options: AddAssetOptions,
) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("failed to read {path:?}: {e}"))?;
    appender
        .add_asset(asset_type, name, &bytes, options)
        .map_err(|e: BendlWriteError| format!("failed to add asset {name:?}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::encode::encode_jsonl_to_ben;
    use crate::io::bundle::{BendlReader, BendlWriter};
    use crate::io::bundle::format::AssignmentFormat;
    use clap::Parser;
    use std::io::{BufReader, Cursor};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_path(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("bendl-cli-{name}-{nonce}"))
    }

    /// Write a minimal finalized .bendl file and return its path.
    fn write_temp_bendl(name: &str, format: AssignmentFormat) -> PathBuf {
        let path = unique_path(name);
        let stream = b"STANDARD BEN FILE\x00fake";
        let mut buf: Vec<u8> = Vec::new();
        let mut writer = BendlWriter::new(Cursor::new(&mut buf), format).unwrap();
        writer.write_stream_bytes(stream, 1).unwrap();
        writer.finish().unwrap();
        std::fs::write(&path, &buf).unwrap();
        path
    }

    #[test]
    fn write_temp_bendl_xben_variant_works() {
        // Exercises the Xben branch of write_temp_bendl.
        let path = write_temp_bendl("xben_helper_check.bendl", AssignmentFormat::Xben);
        let reader = BendlReader::open(BufReader::new(
            std::fs::File::open(&path).unwrap(),
        ))
        .unwrap();
        assert!(reader.is_complete());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn named_asset_from_str_rejects_empty_name() {
        let err = "=path/to/file".parse::<NamedAsset>().unwrap_err();
        assert!(err.contains("non-empty"));
    }

    #[test]
    fn format_from_path_detects_xben() {
        let fmt = format_from_path(std::path::Path::new("stream.xben")).unwrap();
        assert_eq!(fmt, AssignmentFormat::Xben);
    }

    #[test]
    fn format_from_path_rejects_unknown_extension() {
        let err = format_from_path(std::path::Path::new("archive.tar")).unwrap_err();
        assert!(err.contains("expected .ben or .xben"));
    }

    #[test]
    fn mode_str_returns_xben_for_xben() {
        assert_eq!(mode_str(AssignmentFormat::Xben), "xben");
    }

    #[test]
    fn run_create_with_relabel_map_and_custom_asset() {
        let ben = {
            // Must end in .ben so format_from_path recognises it.
            let p = std::env::temp_dir().join(format!(
                "bendl-create-relabel-{}.ben",
                SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
            ));
            let jsonl = b"{\"assignment\":[1,2,3],\"sample\":1}\n";
            let mut b = Vec::new();
            encode_jsonl_to_ben(Cursor::new(jsonl), &mut b, crate::BenVariant::Standard).unwrap();
            std::fs::write(&p, &b).unwrap();
            p
        };
        let relabel = unique_path("create_relabel_map.json");
        std::fs::write(&relabel, b"{\"0\":1,\"1\":0}").unwrap();
        let custom = unique_path("create_custom.bin");
        std::fs::write(&custom, b"custom bytes").unwrap();
        let out = unique_path("create_with_assets.bendl");

        let asset_str = format!("myblob={}", custom.display());
        let args = CreateArgs {
            input: ben.clone(),
            output: out.clone(),
            graph: None,
            metadata: None,
            relabel_map: Some(relabel.clone()),
            assets: vec![asset_str.parse().unwrap()],
            overwrite: false,
            graph_raw: false,
        };
        run_create(args).unwrap();

        let reader = BendlReader::open(BufReader::new(std::fs::File::open(&out).unwrap())).unwrap();
        assert!(reader.find_asset_by_name("relabel_map.json").is_some());
        assert!(reader.find_asset_by_name("myblob").is_some());

        for p in [&ben, &relabel, &custom, &out] { let _ = std::fs::remove_file(p); }
    }

    #[test]
    fn run_inspect_xben_format_and_checksum_flag() {
        use crate::io::bundle::AddAssetOptions;
        use crate::io::bundle::format::ASSET_TYPE_CUSTOM;

        // Build a .bendl with a checksum asset so the flag_parts checksum
        // branch is exercised.
        let mut buf: Vec<u8> = Vec::new();
        let mut writer = BendlWriter::new(Cursor::new(&mut buf), AssignmentFormat::Xben).unwrap();
        writer
            .add_asset(
                ASSET_TYPE_CUSTOM,
                "checksummed",
                b"data",
                AddAssetOptions {
                    checksum: Some(vec![0xAB, 0xCD]),
                    ..AddAssetOptions::defaults()
                },
            )
            .unwrap();
        writer.write_stream_bytes(b"STANDARD BEN FILE\x00fake", 1).unwrap();
        writer.finish().unwrap();
        let path = unique_path("inspect_xben.bendl");
        std::fs::write(&path, &buf).unwrap();

        run_inspect(InspectArgs { input: path.clone() }).unwrap();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn run_append_no_assets_is_noop() {
        let bendl = write_temp_bendl("append_noop.bendl", AssignmentFormat::Ben);
        let args = AppendArgs {
            input: bendl.clone(),
            graph: None,
            metadata: None,
            relabel_map: None,
            assets: vec![],
            graph_raw: false,
        };
        run_append(args).unwrap();
        // File should be unchanged (bundle is still valid).
        let reader = BendlReader::open(BufReader::new(
            std::fs::File::open(&bendl).unwrap(),
        ))
        .unwrap();
        assert!(reader.is_complete());
        let _ = std::fs::remove_file(&bendl);
    }

    #[test]
    fn run_append_with_metadata_and_relabel_map() {
        let bendl = write_temp_bendl("append_assets.bendl", AssignmentFormat::Ben);
        let meta = unique_path("append_meta.json");
        std::fs::write(&meta, b"{\"version\":1}").unwrap();
        let relabel = unique_path("append_relabel.json");
        std::fs::write(&relabel, b"{\"0\":1}").unwrap();

        let args = AppendArgs {
            input: bendl.clone(),
            graph: None,
            metadata: Some(meta.clone()),
            relabel_map: Some(relabel.clone()),
            assets: vec![],
            graph_raw: false,
        };
        run_append(args).unwrap();

        let reader = BendlReader::open(BufReader::new(
            std::fs::File::open(&bendl).unwrap(),
        ))
        .unwrap();
        assert!(reader.find_asset_by_name("metadata.json").is_some());
        assert!(reader.find_asset_by_name("relabel_map.json").is_some());

        for p in [&bendl, &meta, &relabel] { let _ = std::fs::remove_file(p); }
    }

    #[test]
    fn run_create_with_graph_raw_flag() {
        let ben = {
            let p = std::env::temp_dir().join(format!(
                "bendl-create-raw-{}.ben",
                SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
            ));
            let jsonl = b"{\"assignment\":[1,2],\"sample\":1}\n";
            let mut b = Vec::new();
            encode_jsonl_to_ben(Cursor::new(jsonl), &mut b, crate::BenVariant::Standard).unwrap();
            std::fs::write(&p, &b).unwrap();
            p
        };
        let graph = unique_path("create_raw_graph.json");
        std::fs::write(&graph, b"{\"nodes\":[0,1]}").unwrap();
        let out = unique_path("create_raw.bendl");

        let args = CreateArgs {
            input: ben.clone(),
            output: out.clone(),
            graph: Some(graph.clone()),
            metadata: None,
            relabel_map: None,
            assets: vec![],
            overwrite: false,
            graph_raw: true,
        };
        run_create(args).unwrap();

        let reader = BendlReader::open(BufReader::new(
            std::fs::File::open(&out).unwrap(),
        ))
        .unwrap();
        assert!(reader.find_asset_by_name("graph.json").is_some());

        for p in [&ben, &graph, &out] { let _ = std::fs::remove_file(p); }
    }

    #[test]
    fn run_inspect_unknown_format_and_no_sample_count() {
        use crate::io::bundle::format::{BENDL_MAGIC, BENDL_MAJOR_VERSION, BENDL_MINOR_VERSION,
                                        COMPLETE_NO, HEADER_SIZE};

        // Build a header with an unknown assignment format byte and
        // complete=0 so sample_count() returns None.
        let mut header = [0u8; HEADER_SIZE];
        header[0..8].copy_from_slice(&BENDL_MAGIC);
        header[8..10].copy_from_slice(&BENDL_MAJOR_VERSION.to_le_bytes());
        header[10..12].copy_from_slice(&BENDL_MINOR_VERSION.to_le_bytes());
        header[12] = COMPLETE_NO;
        header[13] = 0xFF; // unknown format byte
        // stream_offset = HEADER_SIZE, stream_len = 0, sample_count = -1
        let stream_offset = HEADER_SIZE as u64;
        header[40..48].copy_from_slice(&stream_offset.to_le_bytes());
        let sample_count: i64 = -1;
        header[56..64].copy_from_slice(&sample_count.to_le_bytes());

        let path = unique_path("inspect_unknown.bendl");
        std::fs::write(&path, &header).unwrap();
        run_inspect(InspectArgs { input: path.clone() }).unwrap();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn run_append_with_graph_raw_and_graph_asset() {
        let bendl = write_temp_bendl("append_graph_raw.bendl", AssignmentFormat::Ben);
        let graph = unique_path("append_graph_raw.json");
        std::fs::write(&graph, b"{\"nodes\":[0,1,2]}").unwrap();

        let args = AppendArgs {
            input: bendl.clone(),
            graph: Some(graph.clone()),
            metadata: None,
            relabel_map: None,
            assets: vec![],
            graph_raw: true,
        };
        run_append(args).unwrap();

        let reader = BendlReader::open(BufReader::new(
            std::fs::File::open(&bendl).unwrap(),
        ))
        .unwrap();
        assert!(reader.find_asset_by_name("graph.json").is_some());

        for p in [&bendl, &graph] { let _ = std::fs::remove_file(p); }
    }

    #[test]
    fn run_extract_rejects_missing_stream_and_asset() {
        let args = ExtractArgs::try_parse_from([
            "extract",
            "--output", "/tmp/out.bin",
            "bundle.bendl",
        ])
        .unwrap();
        let err = run_extract(args).unwrap_err();
        assert!(err.contains("either --stream or --asset"));
    }

    #[test]
    fn run_create_errors_on_missing_metadata_file() {
        let ben = {
            let p = std::env::temp_dir().join(format!(
                "bendl-err-meta-{}.ben",
                SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
            ));
            let jsonl = b"{\"assignment\":[1],\"sample\":1}\n";
            let mut b = Vec::new();
            encode_jsonl_to_ben(Cursor::new(jsonl), &mut b, crate::BenVariant::Standard).unwrap();
            std::fs::write(&p, &b).unwrap();
            p
        };
        let out = unique_path("err_meta.bendl");
        let args = CreateArgs {
            input: ben.clone(),
            output: out.clone(),
            graph: None,
            metadata: Some(unique_path("nonexistent_meta.json")),
            relabel_map: None,
            assets: vec![],
            overwrite: false,
            graph_raw: false,
        };
        let err = run_create(args).unwrap_err();
        assert!(err.contains("failed to read"));
        let _ = std::fs::remove_file(&ben);
        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn run_create_errors_on_missing_relabel_map_file() {
        let ben = {
            let p = std::env::temp_dir().join(format!(
                "bendl-err-relabel-{}.ben",
                SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
            ));
            let mut b = Vec::new();
            encode_jsonl_to_ben(
                Cursor::new(b"{\"assignment\":[1],\"sample\":1}\n"),
                &mut b,
                crate::BenVariant::Standard,
            ).unwrap();
            std::fs::write(&p, &b).unwrap();
            p
        };
        let out = unique_path("err_relabel.bendl");
        let args = CreateArgs {
            input: ben.clone(),
            output: out.clone(),
            graph: None,
            metadata: None,
            relabel_map: Some(unique_path("nonexistent_relabel.json")),
            assets: vec![],
            overwrite: false,
            graph_raw: false,
        };
        let err = run_create(args).unwrap_err();
        assert!(err.contains("failed to read"));
        let _ = std::fs::remove_file(&ben);
        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn run_create_errors_on_missing_custom_asset_file() {
        let ben = {
            let p = std::env::temp_dir().join(format!(
                "bendl-err-custom-{}.ben",
                SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
            ));
            let mut b = Vec::new();
            encode_jsonl_to_ben(
                Cursor::new(b"{\"assignment\":[1],\"sample\":1}\n"),
                &mut b,
                crate::BenVariant::Standard,
            ).unwrap();
            std::fs::write(&p, &b).unwrap();
            p
        };
        let out = unique_path("err_custom.bendl");
        let nonexistent: PathBuf = unique_path("nonexistent.bin");
        let asset_str = format!("myasset={}", nonexistent.display());
        let args = CreateArgs {
            input: ben.clone(),
            output: out.clone(),
            graph: None,
            metadata: None,
            relabel_map: None,
            assets: vec![asset_str.parse().unwrap()],
            overwrite: false,
            graph_raw: false,
        };
        let err = run_create(args).unwrap_err();
        assert!(err.contains("failed to read"));
        let _ = std::fs::remove_file(&ben);
        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn run_extract_asset_by_name() {
        use crate::io::bundle::AddAssetOptions;
        use crate::io::bundle::format::ASSET_TYPE_CUSTOM;

        // Build a bundle with a named asset then extract it.
        let mut buf: Vec<u8> = Vec::new();
        let mut writer = BendlWriter::new(Cursor::new(&mut buf), AssignmentFormat::Ben).unwrap();
        writer
            .add_asset(ASSET_TYPE_CUSTOM, "hello.txt", b"world", AddAssetOptions::defaults())
            .unwrap();
        writer.write_stream_bytes(b"STANDARD BEN FILE\x00fake", 1).unwrap();
        writer.finish().unwrap();
        let bendl = unique_path("extract_asset.bendl");
        std::fs::write(&bendl, &buf).unwrap();

        let out = unique_path("extract_asset_out.txt");
        let args = ExtractArgs::try_parse_from([
            "extract",
            "--asset", "hello.txt",
            "--output", out.to_str().unwrap(),
            bendl.to_str().unwrap(),
        ])
        .unwrap();
        run_extract(args).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"world");

        let _ = std::fs::remove_file(&bendl);
        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn run_append_errors_on_missing_metadata_file() {
        let bendl = write_temp_bendl("append_err_meta.bendl", AssignmentFormat::Ben);
        let args = AppendArgs {
            input: bendl.clone(),
            graph: None,
            metadata: Some(unique_path("nonexistent_meta.json")),
            relabel_map: None,
            assets: vec![],
            graph_raw: false,
        };
        let err = run_append(args).unwrap_err();
        assert!(err.contains("failed to read"));
        let _ = std::fs::remove_file(&bendl);
    }

    #[test]
    fn run_append_errors_on_missing_relabel_map_file() {
        let bendl = write_temp_bendl("append_err_relabel.bendl", AssignmentFormat::Ben);
        let args = AppendArgs {
            input: bendl.clone(),
            graph: None,
            metadata: None,
            relabel_map: Some(unique_path("nonexistent_relabel.json")),
            assets: vec![],
            graph_raw: false,
        };
        let err = run_append(args).unwrap_err();
        assert!(err.contains("failed to read"));
        let _ = std::fs::remove_file(&bendl);
    }

    #[test]
    fn run_append_errors_on_missing_custom_asset_file() {
        let bendl = write_temp_bendl("append_err_custom.bendl", AssignmentFormat::Ben);
        let nonexistent = unique_path("nonexistent_custom.bin");
        let asset_str = format!("myasset={}", nonexistent.display());
        let args = AppendArgs {
            input: bendl.clone(),
            graph: None,
            metadata: None,
            relabel_map: None,
            assets: vec![asset_str.parse().unwrap()],
            graph_raw: false,
        };
        let err = run_append(args).unwrap_err();
        assert!(err.contains("failed to read"));
        let _ = std::fs::remove_file(&bendl);
    }
}
