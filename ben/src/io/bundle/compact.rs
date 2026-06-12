//! Compaction: rewriting a bundle without its unreferenced byte ranges.
//!
//! Removing an asset through [`super::writer::BendlAppender::remove_asset`] only drops the
//! directory entry, and every append leaves a superseded directory behind — both leave dead bytes
//! in the file that no reader ever touches. Compaction reclaims them. The user-facing removal
//! paths (the `bendl remove` CLI command and the Python facade) compact automatically, so for
//! them "removed" means the bytes are actually gone.
//!
//! Two strategies, chosen automatically by [`compact_bundle_in_place`]:
//!
//! - **Tail rewrite.** Asset removals and appends only ever create dead space *after* the
//!   assignment stream (pre-stream assets are written back-to-back, and appends land past the
//!   stream). When the prefix through the stream is fully live, only the small post-stream tail
//!   (surviving appended assets + directory) is rebuilt, in place, and the file is truncated. Cost
//!   is O(tail), independent of stream size — removing an appended asset from a 50 GB bundle costs
//!   milliseconds and needs no scratch space. The stream is never read, so this path performs no
//!   stream checksum verification.
//! - **Full rewrite.** When dead space exists before the stream (a removed pre-stream asset), the
//!   bundle is rewritten wholesale: assets carried by decoded payload (verify-on-touch), stream
//!   copied verbatim through the verified reader, temp file atomically swapped in.
//!
//! Both strategies preserve the stream's wire format (BEN or XBEN) as-is.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

use super::format::{
    encode_directory, AssignmentFormat, BendlDirectoryEntry, BendlHeader, KnownAssetKind,
    ASSET_FLAG_JSON, ASSET_TYPE_GRAPH, ASSET_TYPE_METADATA, ASSET_TYPE_NODE_PERMUTATION_MAP,
    HEADER_SIZE,
};
use super::reader::BendlReader;
use super::writer::{AddAssetOptions, BendlWriteError, BendlWriter};

/// Which compaction strategy [`compact_bundle_in_place`] ended up using.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compaction {
    /// The bundle had no unreferenced bytes; the file was left untouched.
    None,
    /// Only the post-stream tail was rebuilt; the stream region was never read or moved.
    TailRewrite,
    /// The whole bundle was rewritten through a temp file and atomically swapped in.
    FullRewrite,
}

/// A single asset read back from the source bundle, ready to be re-added to the new one.
struct PreservedAsset {
    asset_type: u16,
    name: String,
    is_json: bool,
    payload: Vec<u8>,
}

fn known_kind(asset_type: u16) -> Option<KnownAssetKind> {
    match asset_type {
        ASSET_TYPE_METADATA => Some(KnownAssetKind::Metadata),
        ASSET_TYPE_GRAPH => Some(KnownAssetKind::Graph),
        ASSET_TYPE_NODE_PERMUTATION_MAP => Some(KnownAssetKind::NodePermutationMap),
        _ => None,
    }
}

fn add_preserved<W: Write + Seek>(
    writer: &mut BendlWriter<W>,
    asset: &PreservedAsset,
) -> Result<(), BendlWriteError> {
    let opts = if asset.is_json {
        AddAssetOptions::defaults().json()
    } else {
        AddAssetOptions::defaults()
    };
    match known_kind(asset.asset_type) {
        Some(kind) => writer.add_known_asset(kind, &asset.payload, opts),
        None => writer.add_custom_asset(&asset.name, &asset.payload, opts),
    }
}

/// Rewrite the finalized bundle behind `reader` into `out`, dropping unreferenced byte ranges.
///
/// This is the full-rewrite strategy: assets are carried over by decoded payload (verify-on-touch
/// applies), and the assignment stream is copied verbatim through the verified stream reader, so
/// a checksum mismatch anywhere in the source surfaces as an error here instead of propagating.
/// Asset storage compression is normalized to the writer's default policy. Returns the
/// destination writer on success.
pub fn compact_bundle<R, W>(reader: &mut BendlReader<R>, out: W) -> Result<W, BendlWriteError>
where
    R: Read + Seek,
    W: Write + Seek,
{
    if !reader.is_finalized() {
        return Err(BendlWriteError::BundleIncomplete);
    }
    let sample_count = reader.header().sample_count;
    let stream_len = reader.header().stream_len;
    let assignment_format = reader
        .header()
        .assignment_format_typed()
        .unwrap_or(AssignmentFormat::Ben);

    // Read every asset's decoded payload up front (each read borrows the reader exclusively).
    let entries: Vec<_> = reader.assets().to_vec();
    let mut assets = Vec::with_capacity(entries.len());
    for entry in &entries {
        let payload = reader.asset_bytes(entry).map_err(io::Error::other)?;
        assets.push(PreservedAsset {
            asset_type: entry.asset_type,
            name: entry.name.clone(),
            is_json: entry.asset_flags & ASSET_FLAG_JSON != 0,
            payload,
        });
    }

    let mut writer = BendlWriter::new(out, assignment_format)?;
    for asset in &assets {
        add_preserved(&mut writer, asset)?;
    }

    if stream_len == 0 {
        writer.finish()
    } else {
        let mut stream = reader
            .assignment_stream_reader()
            .map_err(io::Error::other)?;
        let mut session = writer.into_stream_session()?;
        io::copy(&mut stream, &mut session)?;
        let writer = session.finish_into_writer(sample_count);
        writer.finish()
    }
}

/// The post-stream tail to rebuild: surviving appended assets (raw on-disk bytes, so their
/// storage form and checksums carry over unchanged) followed by the new directory.
struct PlannedTail {
    /// Concatenated raw bytes to write at the stream end: survivor payloads then directory.
    block: Vec<u8>,
    /// Final directory offset (stream end + survivor payload bytes).
    directory_offset: u64,
    /// Final directory length.
    directory_len: u64,
    /// Final file length.
    file_len: u64,
}

/// Decide whether the tail-rewrite strategy applies and, if so, plan it.
///
/// Applicable iff the prefix `[0, stream_end)` is fully live: the pre-stream assets tile
/// `[HEADER_SIZE, stream_offset)` exactly and every other live payload sits at or beyond the
/// stream end. Returns `None` when dead bytes exist before the stream end (full rewrite needed).
fn plan_tail(
    file: &mut File,
    header: &BendlHeader,
    entries: &[BendlDirectoryEntry],
) -> Result<Option<PlannedTail>, BendlWriteError> {
    let stream_end = header
        .stream_offset
        .checked_add(header.stream_len)
        .ok_or_else(|| io::Error::other("stream_offset + stream_len overflowed"))?;

    let mut pre: Vec<&BendlDirectoryEntry> = Vec::new();
    let mut post: Vec<&BendlDirectoryEntry> = Vec::new();
    for entry in entries {
        if entry.payload_offset < header.stream_offset {
            pre.push(entry);
        } else if entry.payload_offset >= stream_end {
            post.push(entry);
        } else {
            // A payload inside the stream region is malformed; let the full path report it.
            return Ok(None);
        }
    }

    // The prefix must be exactly tiled: header, then pre-stream payloads back-to-back, then the
    // stream. Any gap means pre-stream dead space, which only a full rewrite can reclaim.
    pre.sort_by_key(|e| e.payload_offset);
    let mut cursor = HEADER_SIZE as u64;
    for entry in &pre {
        if entry.payload_offset != cursor {
            return Ok(None);
        }
        cursor = cursor
            .checked_add(entry.payload_len)
            .ok_or_else(|| io::Error::other("payload range overflowed"))?;
    }
    if cursor != header.stream_offset {
        return Ok(None);
    }

    // Read survivors' raw on-disk bytes and lay them out from the stream end.
    post.sort_by_key(|e| e.payload_offset);
    let mut block = Vec::new();
    let mut new_entries: Vec<BendlDirectoryEntry> = pre.iter().map(|e| (*e).clone()).collect();
    let mut offset = stream_end;
    for entry in &post {
        let mut payload = vec![0u8; entry.payload_len as usize];
        file.seek(SeekFrom::Start(entry.payload_offset))?;
        file.read_exact(&mut payload)?;
        block.extend_from_slice(&payload);
        let mut moved = (*entry).clone();
        moved.payload_offset = offset;
        new_entries.push(moved);
        offset += entry.payload_len;
    }
    let directory_offset = offset;
    let directory_bytes = encode_directory(&new_entries)?;
    let directory_len = directory_bytes.len() as u64;
    block.extend_from_slice(&directory_bytes);

    Ok(Some(PlannedTail {
        block,
        directory_offset,
        directory_len,
        file_len: directory_offset + directory_len,
    }))
}

/// Write `header` (patched with the given directory location) at offset 0 and sync.
fn patch_header(
    file: &mut File,
    header: &mut BendlHeader,
    directory_offset: u64,
    directory_len: u64,
) -> io::Result<()> {
    header.directory_offset = directory_offset;
    header.directory_len = directory_len;
    file.seek(SeekFrom::Start(0))?;
    header.write_to(file)?;
    file.sync_data()
}

/// Execute a planned tail rewrite crash-safely.
///
/// Phase 1 appends the new tail block (survivor payloads + directory) at the current EOF and
/// patches the header to the appended directory — pure append, so a crash anywhere leaves either
/// the old or the appended directory authoritative over intact bytes. Phase 2 writes the same
/// block at the stream end and patches the header again; every byte it touches is dead under the
/// phase-1 state (the block is never larger than the dead region, which contains the survivors'
/// old payloads plus at least one superseded directory of equal entry count). The trailing
/// truncate runs last.
fn execute_tail(
    file: &mut File,
    header: &mut BendlHeader,
    plan: &PlannedTail,
) -> Result<(), BendlWriteError> {
    let block_start = plan.directory_offset - (plan.block.len() as u64 - plan.directory_len);

    // Phase 1: relocate the tail to the end of the file (append-only), then adopt it.
    let eof = file.seek(SeekFrom::End(0))?;
    debug_assert!(
        plan.file_len <= eof,
        "tail block must fit in the dead region"
    );
    file.write_all(&plan.block)?;
    file.sync_data()?;
    let staged_dir_offset = eof + (plan.directory_offset - block_start);
    patch_header(file, header, staged_dir_offset, plan.directory_len)?;

    // Phase 2: write the block at its final home (every touched byte is dead), adopt, truncate.
    file.seek(SeekFrom::Start(block_start))?;
    file.write_all(&plan.block)?;
    file.sync_data()?;
    patch_header(file, header, plan.directory_offset, plan.directory_len)?;
    file.set_len(plan.file_len)?;
    file.sync_data()?;
    Ok(())
}

/// Compact the bundle at `path` in place, choosing the cheapest applicable strategy.
///
/// Returns which strategy ran. [`Compaction::TailRewrite`] never reads or moves the assignment
/// stream (and therefore performs no stream checksum verification); [`Compaction::FullRewrite`]
/// streams the whole bundle through verified readers into a temp file and atomically swaps it
/// over `path`. On any error the original file is left untouched.
pub fn compact_bundle_in_place(path: &Path) -> Result<Compaction, BendlWriteError> {
    // Parse and validate through the reader so malformed bundles are rejected up front.
    let file = File::open(path)?;
    let reader = BendlReader::open(BufReader::new(file)).map_err(BendlWriteError::Format)?;
    if !reader.is_finalized() {
        return Err(BendlWriteError::BundleIncomplete);
    }
    let mut header = *reader.header();
    let entries: Vec<BendlDirectoryEntry> = reader.assets().to_vec();
    drop(reader);

    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    if let Some(plan) = plan_tail(&mut file, &header, &entries)? {
        // Already compact? Then the directory sits right at its planned offset and the file ends
        // right after it — nothing to do.
        let eof = file.seek(SeekFrom::End(0))?;
        if header.directory_offset == plan.directory_offset && eof == plan.file_len {
            return Ok(Compaction::None);
        }
        execute_tail(&mut file, &mut header, &plan)?;
        return Ok(Compaction::TailRewrite);
    }
    drop(file);

    // Dead space before the stream: full rewrite through a temp file.
    let file = File::open(path)?;
    let mut reader = BendlReader::open(BufReader::new(file)).map_err(BendlWriteError::Format)?;

    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".compact-tmp");
    let tmp = Path::new(&tmp).to_path_buf();

    let result: Result<(), BendlWriteError> = (|| {
        let out = BufWriter::new(File::create(&tmp)?);
        let out = compact_bundle(&mut reader, out)?;
        out.into_inner()
            .map_err(|e| io::Error::other(e.to_string()))?
            .sync_all()?;
        fs::rename(&tmp, path)?;
        Ok(())
    })();

    if result.is_err() && tmp.exists() {
        let _ = fs::remove_file(&tmp);
    }
    result.map(|()| Compaction::FullRewrite)
}
