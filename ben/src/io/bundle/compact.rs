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

use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use super::format::{
    encode_directory, AssignmentFormat, BendlDirectoryEntry, BendlHeader, KnownAssetKind,
    ASSET_FLAG_JSON, ASSET_TYPE_GRAPH, ASSET_TYPE_METADATA, ASSET_TYPE_NODE_PERMUTATION_MAP,
    HEADER_SIZE,
};
use super::reader::{validate_entry_extents, BendlReader};
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
    compact_bundle_excluding(reader, out, &HashSet::new())
}

/// [`compact_bundle`], skipping every asset whose name is in `exclude`.
///
/// Excluded assets are never read, so a removal also succeeds when the asset being removed is
/// itself corrupt.
fn compact_bundle_excluding<R, W>(
    reader: &mut BendlReader<R>,
    out: W,
    exclude: &HashSet<&str>,
) -> Result<W, BendlWriteError>
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

    // Read every surviving asset's decoded payload up front (each read borrows the reader
    // exclusively).
    let entries: Vec<_> = reader.assets().to_vec();
    let mut assets = Vec::with_capacity(entries.len());
    for entry in &entries {
        if exclude.contains(entry.name.as_str()) {
            continue;
        }
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

/// One survivor payload's source range, to be copied (raw on-disk bytes, so storage form and
/// checksums carry over unchanged) into the rebuilt tail.
struct PayloadMove {
    /// Byte offset of the payload in the current file.
    src: u64,
    /// Payload length in bytes.
    len: u64,
}

/// The post-stream tail to rebuild: surviving appended assets followed by the new directory.
///
/// Planning is pure arithmetic over the directory — no payload byte is read until a rewrite
/// actually executes, and the rewrite itself copies file-to-file through a fixed-size buffer,
/// so tail compaction needs no payload-sized memory.
pub(super) struct PlannedTail {
    /// Survivor payload source ranges, in final layout order.
    moves: Vec<PayloadMove>,
    /// Total survivor payload bytes (the moves' lengths summed).
    payloads_len: u64,
    /// All surviving entries (pre-stream entries unchanged, survivors at their final offsets).
    final_entries: Vec<BendlDirectoryEntry>,
    /// The encoded form of `final_entries`.
    final_directory_bytes: Vec<u8>,
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
pub(super) fn plan_tail(
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

    // Lay the survivors out from the stream end — arithmetic only, no payload reads. Extent
    // validation before planning guarantees every source range lies within the file.
    post.sort_by_key(|e| e.payload_offset);
    let mut moves = Vec::with_capacity(post.len());
    let mut final_entries: Vec<BendlDirectoryEntry> = pre.iter().map(|e| (*e).clone()).collect();
    let mut offset = stream_end;
    for entry in &post {
        moves.push(PayloadMove {
            src: entry.payload_offset,
            len: entry.payload_len,
        });
        let mut moved = (*entry).clone();
        moved.payload_offset = offset;
        final_entries.push(moved);
        offset = offset
            .checked_add(entry.payload_len)
            .ok_or_else(|| io::Error::other("payload range overflowed"))?;
    }
    let payloads_len = offset - stream_end;
    let directory_offset = offset;
    let final_directory_bytes = encode_directory(&final_entries)?;
    let directory_len = final_directory_bytes.len() as u64;

    Ok(Some(PlannedTail {
        moves,
        payloads_len,
        final_entries,
        final_directory_bytes,
        directory_offset,
        directory_len,
        file_len: directory_offset
            .checked_add(directory_len)
            .ok_or_else(|| io::Error::other("directory range overflowed"))?,
    }))
}

/// Copy `len` bytes within `file` from `src` to `dst` through a fixed-size buffer.
///
/// The caller is responsible for ensuring the ranges don't overlap in a way that would read
/// already-overwritten bytes; both call sites here copy between disjoint regions.
fn copy_within(file: &mut File, src: u64, dst: u64, len: u64) -> io::Result<()> {
    let mut buf = vec![0u8; 64 * 1024];
    let mut done = 0u64;
    while done < len {
        let chunk = buf.len().min((len - done) as usize);
        file.seek(SeekFrom::Start(src + done))?;
        file.read_exact(&mut buf[..chunk])?;
        file.seek(SeekFrom::Start(dst + done))?;
        file.write_all(&buf[..chunk])?;
        done += chunk as u64;
    }
    Ok(())
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

/// Execute a planned tail rewrite crash-safely: stage at EOF, adopt, rewrite at the final home,
/// adopt, truncate.
///
/// The invariant both phases preserve is that the authoritative directory only ever references
/// bytes that have already been written and synced. Re-running compaction from any intermediate
/// state converges losslessly, because each state's directory points at intact payload copies.
fn execute_tail(
    file: &mut File,
    header: &mut BendlHeader,
    plan: &PlannedTail,
) -> Result<(), BendlWriteError> {
    let staged_base = stage_tail(file, header, plan)?;
    finalize_tail(file, header, plan, staged_base)
}

/// Phase 1: copy the survivor payloads and write a *staged* directory — one whose entries point
/// at those appended copies — at the current EOF, then patch the header to adopt it. Returns the
/// EOF the staging started at (the staged payload base).
///
/// Every write is append-only and the staged directory references only bytes that already exist
/// (the live prefix plus the appended copies), so a crash anywhere up to and including the header
/// patch leaves an intact bundle: either the old directory or the fully self-consistent staged
/// one is authoritative.
pub(super) fn stage_tail(
    file: &mut File,
    header: &mut BendlHeader,
    plan: &PlannedTail,
) -> Result<u64, BendlWriteError> {
    let block_start = plan.directory_offset - plan.payloads_len;

    let eof = file.seek(SeekFrom::End(0))?;
    debug_assert!(
        plan.file_len <= eof,
        "tail block must fit in the dead region"
    );
    let staged_entries: Vec<BendlDirectoryEntry> = plan
        .final_entries
        .iter()
        .map(|entry| {
            let mut staged = entry.clone();
            if staged.payload_offset >= block_start {
                staged.payload_offset = eof + (staged.payload_offset - block_start);
            }
            staged
        })
        .collect();
    let staged_directory_bytes = encode_directory(&staged_entries)?;
    debug_assert_eq!(
        staged_directory_bytes.len() as u64,
        plan.directory_len,
        "staged and final directories must encode to the same length"
    );

    let mut dst = eof;
    for mv in &plan.moves {
        copy_within(file, mv.src, dst, mv.len)?;
        dst += mv.len;
    }
    file.seek(SeekFrom::Start(eof + plan.payloads_len))?;
    file.write_all(&staged_directory_bytes)?;
    file.sync_data()?;
    patch_header(file, header, eof + plan.payloads_len, plan.directory_len)?;
    Ok(eof)
}

/// Phase 2: copy the payloads down from the staged region to the stream end, write the final
/// directory, patch the header to it, and truncate the staged tail away.
///
/// Every byte this touches is dead under the staged state: the staged directory references only
/// the live prefix (which ends at the stream end) and the staged copies at or beyond the old EOF,
/// and the final tail never extends past the old EOF — so the source and destination of the copy
/// are disjoint. The truncate runs only after the final header patch is synced.
fn finalize_tail(
    file: &mut File,
    header: &mut BendlHeader,
    plan: &PlannedTail,
    staged_base: u64,
) -> Result<(), BendlWriteError> {
    let block_start = plan.directory_offset - plan.payloads_len;

    copy_within(file, staged_base, block_start, plan.payloads_len)?;
    file.seek(SeekFrom::Start(plan.directory_offset))?;
    file.write_all(&plan.final_directory_bytes)?;
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
    compact_in_place_excluding(path, &[])
}

/// Remove the named assets from the bundle at `path` and compact it, as one operation.
///
/// The removal and the compaction commit together: the directory that drops the names is the
/// same one the rewrite publishes, so no intermediate state ever exists in which an asset is
/// unreferenced but its bytes remain — and on any error the original file is left untouched,
/// with every asset still present for a retry. Unknown names are rejected up front, before any
/// byte of the file changes. The assets being removed are never read, so removal also succeeds
/// when the asset being removed is itself corrupt.
///
/// `names` may repeat (duplicates collapse). An empty `names` is plain
/// [`compact_bundle_in_place`].
pub fn remove_assets_in_place(
    path: &Path,
    names: &[&str],
) -> Result<Compaction, BendlWriteError> {
    compact_in_place_excluding(path, names)
}

fn compact_in_place_excluding(
    path: &Path,
    remove: &[&str],
) -> Result<Compaction, BendlWriteError> {
    // Parse and validate through the reader so malformed bundles are rejected up front.
    let file = File::open(path)?;
    let file_len = file.metadata()?.len();
    let reader = BendlReader::open(BufReader::new(file)).map_err(BendlWriteError::Format)?;
    if !reader.is_finalized() {
        return Err(BendlWriteError::BundleIncomplete);
    }
    // Compaction trusts the directory's lengths to size allocations and the rewritten layout
    // (reader opens stay lenient so truncated bundles remain inspectable), so out-of-range
    // extents must be rejected before any planning.
    validate_entry_extents(reader.assets(), file_len).map_err(|e| {
        BendlWriteError::Format(super::format::BendlFormatError::MalformedDirectory(
            e.to_string(),
        ))
    })?;
    let remove: HashSet<&str> = remove.iter().copied().collect();
    for name in &remove {
        if reader.find_asset_by_name(name).is_none() {
            return Err(BendlWriteError::UnknownAssetName((*name).to_string()));
        }
    }
    let mut header = *reader.header();
    let entries: Vec<BendlDirectoryEntry> = reader
        .assets()
        .iter()
        .filter(|e| !remove.contains(e.name.as_str()))
        .cloned()
        .collect();
    drop(reader);

    // Planning is pure directory arithmetic, so the already-compact case is decided here —
    // before the file is even opened for writing, and without reading a single payload byte.
    if let Some(plan) = plan_tail(&header, &entries)? {
        // Already compact? Then the directory sits right at its planned offset and the file ends
        // right after it — nothing to do. (Unreachable with removals: dropping an entry always
        // shrinks the directory, so the planned layout cannot match the current one.)
        if header.directory_offset == plan.directory_offset && file_len == plan.file_len {
            return Ok(Compaction::None);
        }
        let mut file = OpenOptions::new().read(true).write(true).open(path)?;
        execute_tail(&mut file, &mut header, &plan)?;
        return Ok(Compaction::TailRewrite);
    }

    // Dead space before the stream end: full rewrite through a uniquely named temp file that
    // inherits the bundle's permissions, synced and atomically renamed over `path`. The unique
    // name keeps two concurrent compactions of the same bundle from interleaving writes into a
    // shared temp inode.
    let file = File::open(path)?;
    let mut reader = BendlReader::open(BufReader::new(file)).map_err(BendlWriteError::Format)?;

    static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!(
        ".{file_name}.compact-{}-{}.tmp",
        std::process::id(),
        TMP_COUNTER.fetch_add(1, Ordering::Relaxed),
    ));

    let result: Result<(), BendlWriteError> = (|| {
        let out = File::options().write(true).create_new(true).open(&tmp)?;
        // The rewrite holds the same bytes the bundle holds; never let the copy sit with wider
        // permissions than the original, and never let the swap change the bundle's mode.
        if let Ok(meta) = fs::metadata(path) {
            let _ = fs::set_permissions(&tmp, meta.permissions());
        }
        let out = BufWriter::new(out);
        let out = compact_bundle_excluding(&mut reader, out, &remove)?;
        out.into_inner()
            .map_err(|e| io::Error::other(e.to_string()))?
            .sync_all()?;
        fs::rename(&tmp, path)?;
        // Make the rename itself durable where the platform allows it; the data already is.
        #[cfg(unix)]
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            if let Ok(dir) = File::open(parent) {
                let _ = dir.sync_all();
            }
        }
        Ok(())
    })();

    if result.is_err() && tmp.exists() {
        let _ = fs::remove_file(&tmp);
    }
    result.map(|()| Compaction::FullRewrite)
}
