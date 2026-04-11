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
use crate::io::bundle::{
    AddAssetOptions, BendlReader, BendlWriteError, BendlWriter,
};
use crate::io::bundle::writer::BendlAppender;
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
    let file = File::open(&args.input)
        .map_err(|e| format!("failed to open {:?}: {e}", args.input))?;
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

    let file = File::open(&args.input)
        .map_err(|e| format!("failed to open {:?}: {e}", args.input))?;
    let mut reader = BendlReader::open(BufReader::new(file))
        .map_err(|e| format!("failed to parse bundle header: {e}"))?;

    let mut out = BufWriter::new(
        File::create(&args.output).map_err(|e| format!("failed to create {:?}: {e}", args.output))?,
    );

    if args.stream {
        let mut stream = reader
            .assignment_stream_reader()
            .map_err(|e| format!("failed to open stream region: {e}"))?;
        io::copy(&mut stream, &mut out)
            .map_err(|e| format!("failed to copy stream bytes: {e}"))?;
    } else if let Some(name) = args.asset.as_deref() {
        let entry = reader
            .find_asset_by_name(name)
            .cloned()
            .ok_or_else(|| format!("no asset named {name:?} in bundle"))?;
        let mut asset = reader
            .asset_reader(&entry)
            .map_err(|e| format!("failed to open asset {name:?}: {e}"))?;
        io::copy(&mut asset, &mut out)
            .map_err(|e| format!("failed to copy asset bytes: {e}"))?;
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
    let mut appender = BendlAppender::open(file)
        .map_err(|e| format!("failed to open appender: {e}"))?;

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
