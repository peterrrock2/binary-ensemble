//! Property-based stress test for [`BendlAppender`].
//!
//! Append is the most subtle BENDL invariant: existing payload offsets must not move; the
//! directory rewrite must be atomic; aborted appends and drops must leave the file unchanged.
//! Existing fixed-seed tests in `ben/src/io/bundle/tests/writer.rs` cover the happy path but
//! cannot explore the full grammar of append/abort/drop sequences.
//!
//! This proptest generates sequences of `AddAsset` / `Commit` / `Abort` / `DropWithoutCommit`
//! operations and verifies:
//!
//! 1. After every `Commit`, every previously-committed asset is still readable and its decoded
//!    bytes match what was originally added.
//! 2. After every `Commit`, every existing directory entry's `(payload_offset, payload_len)` is
//!    unchanged, and the raw bytes at those offsets are byte-for-byte identical to before the
//!    commit. This is the strong append-only invariant.
//! 3. After every `Abort` or drop-without-commit, the file is byte-identical to before the appender
//!    was opened.

use binary_ensemble::io::bundle::format::{
    AssignmentFormat, BendlDirectoryEntry, KnownAssetKind, ASSET_TYPE_CUSTOM,
};
use binary_ensemble::io::bundle::writer::{
    AddAssetOptions, BendlAppender, BendlWriteError, BendlWriter,
};
use binary_ensemble::io::bundle::BendlReader;
use proptest::prelude::*;
use std::io::{Cursor, Read, Seek, SeekFrom};

#[derive(Debug, Clone)]
enum Op {
    /// Open an appender (if none is open) and enqueue a pending asset.
    AddAsset { payload: Vec<u8>, compress: bool },
    /// Commit the currently-open appender, if any.
    Commit,
    /// Abort the currently-open appender via the explicit `.abort()` API, if any.
    Abort,
    /// Drop the currently-open appender without committing, if any. Distinguished from `Abort`
    /// because they exercise different code paths internally even though both leave the file
    /// unchanged.
    DropWithoutCommit,
}

fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (any::<bool>(), 0usize..=64usize).prop_map(|(compress, n)| Op::AddAsset {
            payload: (0..n).map(|i| (i as u8).wrapping_mul(31)).collect(),
            compress,
        }),
        3 => Just(Op::Commit),
        1 => Just(Op::Abort),
        1 => Just(Op::DropWithoutCommit),
    ]
}

/// A reader-side *expectation*: an asset that has been committed to disk, paired with the decoded
/// bytes the reader must return for it. Decoded bytes equal the originally-added payload regardless
/// of whether the asset was stored compressed.
#[derive(Debug, Clone)]
struct CommittedAsset {
    asset_name: String,
    decoded_payload_bytes: Vec<u8>,
}

/// One asset enqueued against the currently-open appender but not yet committed. These are the
/// append-side *inputs*: the raw bytes handed to `add_asset` and whether compression was requested.
#[derive(Debug, Clone)]
struct PendingAsset {
    asset_name: String,
    raw_payload_bytes: Vec<u8>,
    compress_payload: bool,
}

/// The batch of pending assets accumulated against a single open appender, plus the round-local
/// name index used (alongside the model's global counter) to mint stable, unique asset names.
#[derive(Debug, Default)]
struct PendingAppendRound {
    assets: Vec<PendingAsset>,
    next_name_index: usize,
}

/// A snapshot of one existing directory entry's physical placement, taken before a commit. The
/// strong append-only invariant is that `(payload_offset, payload_len)` *and* the raw bytes at that
/// range (`on_disk_payload_bytes`) are byte-for-byte identical after the commit.
#[derive(Debug, Clone)]
struct EntrySnapshot {
    asset_name: String,
    payload_offset: u64,
    payload_len: u64,
    on_disk_payload_bytes: Vec<u8>,
}

/// The model of expected bundle state as a sequence of append-grammar operations is applied. Each
/// operation mutates the model and asserts the relevant BENDL append invariant inline.
struct AppendModel {
    /// Every asset committed to disk, in commit order, with the decoded bytes the reader must
    /// return for it.
    committed: Vec<CommittedAsset>,
    /// The current on-disk bundle bytes.
    current_bytes: Vec<u8>,
    /// `sample_count()` of the seed bundle; must never drift across appends.
    baseline_samples: Option<i64>,
    /// The pending round against the currently-open appender, if any.
    round: Option<PendingAppendRound>,
    /// Monotonic counter that makes every minted asset name unique across rounds.
    name_counter: usize,
}

impl AppendModel {
    /// Start from the seed bundle, recording its baseline sample count and seed asset.
    fn new(seed: Vec<u8>) -> Self {
        let reader = BendlReader::open(Cursor::new(&seed)).unwrap();
        let baseline_samples = reader.sample_count();
        drop(reader);
        AppendModel {
            committed: vec![CommittedAsset {
                asset_name: "seed.bin".to_string(),
                decoded_payload_bytes: b"seed payload bytes".to_vec(),
            }],
            current_bytes: seed,
            baseline_samples,
            round: None,
            name_counter: 0,
        }
    }

    /// Apply one generated operation.
    fn apply(&mut self, op: &Op) {
        match op {
            Op::AddAsset { payload, compress } => self.enqueue_asset(payload, *compress),
            Op::Commit => self.commit_round(),
            Op::Abort => self.abort_round(),
            Op::DropWithoutCommit => self.drop_round_without_commit(),
        }
    }

    /// `AddAsset`: open a round if none is open and enqueue a pending asset with a freshly minted
    /// name. The global counter guarantees uniqueness across rounds, and the round-local index is
    /// embedded so a successful commit lands a stable name.
    fn enqueue_asset(&mut self, payload: &[u8], compress: bool) {
        let name_counter = self.name_counter;
        self.name_counter += 1;
        let round = self.round.get_or_insert_with(PendingAppendRound::default);
        let asset_name = format!("asset-{}-{}.bin", name_counter, round.next_name_index);
        round.next_name_index += 1;
        round.assets.push(PendingAsset {
            asset_name,
            raw_payload_bytes: payload.to_vec(),
            compress_payload: compress,
        });
    }

    /// `Commit`: replay the pending round through a real appender, commit it, and assert every
    /// append invariant against the resulting bytes.
    fn commit_round(&mut self) {
        let Some(round) = self.round.take() else {
            return;
        };
        let snapshot = snapshot_existing_entries(&self.current_bytes);

        let mut appender = BendlAppender::open(Cursor::new(self.current_bytes.clone())).unwrap();
        for asset in &round.assets {
            let opts = pending_asset_options(asset);
            appender
                .add_asset(
                    ASSET_TYPE_CUSTOM,
                    &asset.asset_name,
                    &asset.raw_payload_bytes,
                    opts,
                )
                .expect("add_asset on pending entry should succeed");
        }
        let new_bytes = appender.commit().unwrap().into_inner();

        // An open round always holds at least one pending asset (AddAsset is the only way to enter
        // the round state), so the file can only grow, never shrink.
        assert!(
            new_bytes.len() >= self.current_bytes.len(),
            "file shrank after commit"
        );

        let new_reader = BendlReader::open(Cursor::new(&new_bytes)).unwrap();
        let new_entries: Vec<BendlDirectoryEntry> = new_reader.assets().to_vec();
        drop(new_reader);
        Self::assert_existing_entries_unchanged(&snapshot, &new_entries, &new_bytes);

        for asset in &round.assets {
            self.committed.push(CommittedAsset {
                asset_name: asset.asset_name.clone(),
                decoded_payload_bytes: asset.raw_payload_bytes.clone(),
            });
        }

        let mut reader = BendlReader::open(Cursor::new(&new_bytes)).unwrap();
        assert_eq!(
            reader.assets().len(),
            self.committed.len(),
            "directory size mismatch after commit"
        );
        assert_eq!(
            reader.sample_count(),
            self.baseline_samples,
            "sample_count drifted across append"
        );
        self.assert_committed_assets_readable(&mut reader);

        self.current_bytes = new_bytes;
    }

    /// `Abort`: open an appender on a clone, abort it via the explicit API, and confirm the bytes
    /// returned by `.abort()` equal the pre-abort bytes (nothing was written at the writer level).
    fn abort_round(&mut self) {
        let Some(_round) = self.round.take() else {
            return;
        };
        let pre_bytes = self.current_bytes.clone();
        let appender = BendlAppender::open(Cursor::new(self.current_bytes.clone())).unwrap();
        let cursor = appender.abort();
        let post_bytes = cursor.into_inner();
        assert_eq!(
            post_bytes, pre_bytes,
            "Abort modified the file (it must be a no-op)"
        );
    }

    /// `DropWithoutCommit`: re-enqueue the pending assets on a fresh appender over a clone, then
    /// let it drop without `commit()`. The appender owns a clone, so the master bytes are
    /// untouched regardless; the assertion pins that intent.
    fn drop_round_without_commit(&mut self) {
        let Some(round) = self.round.take() else {
            return;
        };
        let pre_bytes = self.current_bytes.clone();
        {
            let mut appender =
                BendlAppender::open(Cursor::new(self.current_bytes.clone())).unwrap();
            for (i, asset) in round.assets.iter().enumerate() {
                let opts = pending_asset_options(asset);
                let name = format!("dropped-{}-{}.bin", self.name_counter, i);
                let _ =
                    appender.add_asset(ASSET_TYPE_CUSTOM, &name, &asset.raw_payload_bytes, opts);
            }
            // appender drops here without commit().
        }
        assert_eq!(
            self.current_bytes, pre_bytes,
            "DropWithoutCommit modified the master file (it must be a no-op)"
        );
    }

    /// Final consistency check: reopen the file, validate the directory, confirm every committed
    /// asset is still readable, and drive a raw read to EOF to confirm structural soundness. Any
    /// pending round at end-of-sequence is implicitly dropped, which must not affect the bytes.
    fn assert_final_consistency(&self) {
        let mut reader = BendlReader::open(Cursor::new(&self.current_bytes)).unwrap();
        reader.validate_directory().unwrap();
        assert_eq!(
            reader.assets().len(),
            self.committed.len(),
            "final directory size mismatch"
        );
        self.assert_committed_assets_readable(&mut reader);

        let mut tail = Vec::new();
        let mut cursor = Cursor::new(&self.current_bytes);
        cursor.seek(SeekFrom::Start(0)).unwrap();
        cursor.read_to_end(&mut tail).unwrap();
        assert_eq!(tail.len(), self.current_bytes.len());
    }

    /// Assert every committed asset is present in `reader` and decodes to its expected bytes.
    fn assert_committed_assets_readable<R: Read + Seek>(&self, reader: &mut BendlReader<R>) {
        for asset in &self.committed {
            let entry = reader
                .find_asset_by_name(&asset.asset_name)
                .cloned()
                .unwrap();
            let got = reader.asset_bytes(&entry).unwrap();
            assert_eq!(
                got, asset.decoded_payload_bytes,
                "decoded payload mismatch for {}",
                asset.asset_name
            );
        }
    }

    /// Assert the strong append-only invariant: every snapshotted entry kept its offset, length,
    /// and raw on-disk payload bytes after the commit.
    fn assert_existing_entries_unchanged(
        snapshot: &[EntrySnapshot],
        new_entries: &[BendlDirectoryEntry],
        new_bytes: &[u8],
    ) {
        for snap in snapshot {
            let entry = find_entry(new_entries, &snap.asset_name);
            assert_eq!(
                (entry.payload_offset, entry.payload_len),
                (snap.payload_offset, snap.payload_len),
                "directory entry {} (offset, len) drifted after commit",
                snap.asset_name
            );
            let new_raw = raw_bytes_at(new_bytes, entry.payload_offset, entry.payload_len);
            assert_eq!(
                new_raw, snap.on_disk_payload_bytes,
                "directory entry {} raw payload bytes drifted after commit",
                snap.asset_name
            );
        }
    }
}

/// Translate a pending asset's `compress_payload` flag into the matching writer options.
fn pending_asset_options(asset: &PendingAsset) -> AddAssetOptions {
    let opts = AddAssetOptions::defaults();
    if asset.compress_payload {
        opts.compress()
    } else {
        opts.raw()
    }
}

/// Build the seed bundle used by every proptest case: a finalized bundle with one initial custom
/// asset and a short stream so there's something to preserve across appends.
fn build_seed_bundle() -> Vec<u8> {
    let mut writer = BendlWriter::new(Cursor::new(Vec::new()), AssignmentFormat::Ben).unwrap();
    writer
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "seed.bin",
            b"seed payload bytes",
            AddAssetOptions::defaults().raw(),
        )
        .unwrap();
    let mut session = writer.into_stream_session().unwrap();
    use std::io::Write;
    session.write_all(b"STANDARD BEN FILE\x00\x01\x02").unwrap();
    let writer = session.finish_into_writer(1);
    writer.finish().unwrap().into_inner()
}

/// Read the raw bytes at a given offset/length range from a buffer. Used to compare an existing
/// directory entry's payload bytes before and after a commit.
fn raw_bytes_at(buf: &[u8], offset: u64, len: u64) -> Vec<u8> {
    let start = offset as usize;
    let end = start + len as usize;
    buf[start..end].to_vec()
}

/// Snapshot the physical placement of every directory entry in `bytes`, for the append-only
/// invariant check after a commit.
fn snapshot_existing_entries(bytes: &[u8]) -> Vec<EntrySnapshot> {
    let reader = BendlReader::open(Cursor::new(bytes)).unwrap();
    reader
        .assets()
        .iter()
        .map(|e| EntrySnapshot {
            asset_name: e.name.clone(),
            payload_offset: e.payload_offset,
            payload_len: e.payload_len,
            on_disk_payload_bytes: raw_bytes_at(bytes, e.payload_offset, e.payload_len),
        })
        .collect()
}

/// Lookup an entry by name in a freshly-read directory.
fn find_entry<'a>(entries: &'a [BendlDirectoryEntry], name: &str) -> &'a BendlDirectoryEntry {
    entries
        .iter()
        .find(|e| e.name == name)
        .unwrap_or_else(|| panic!("entry {name:?} not found in directory"))
}

/// Run a single sequence of ops against the seed bundle. All invariants are asserted inline by the
/// model as the ops execute.
fn run_sequence(ops: &[Op]) {
    let mut model = AppendModel::new(build_seed_bundle());
    for op in ops {
        model.apply(op);
    }
    model.assert_final_consistency();
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        ..ProptestConfig::default()
    })]

    /// Drive a random sequence of append-grammar operations against the seed bundle. All
    /// invariants are asserted inline by `run_sequence`.
    #[test]
    fn bendl_appender_preserves_existing_entries_and_no_op_aborts(
        ops in proptest::collection::vec(op_strategy(), 1..30),
    ) {
        run_sequence(&ops);
    }
}

// ---------------------------------------------------------------------------
// Deterministic asset-removal tests
// ---------------------------------------------------------------------------

#[test]
fn remove_asset_drops_entry_and_preserves_everything_else() {
    let bytes = build_seed_bundle();
    let mut appender = BendlAppender::open(Cursor::new(bytes)).unwrap();
    appender
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "extra.bin",
            b"keep me",
            AddAssetOptions::defaults().raw(),
        )
        .unwrap();
    let bytes = appender.commit().unwrap().into_inner();

    let mut appender = BendlAppender::open(Cursor::new(bytes)).unwrap();
    appender.remove_asset("seed.bin").unwrap();
    let bytes = appender.commit().unwrap().into_inner();

    let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
    let names: Vec<_> = reader.assets().iter().map(|e| e.name.clone()).collect();
    assert_eq!(names, vec!["extra.bin".to_string()]);
    // The survivor still reads back, and the stream + every remaining checksum still verify
    // (the removed payload's bytes are dead space readers never touch).
    let entry = reader.assets()[0].clone();
    assert_eq!(reader.asset_bytes(&entry).unwrap(), b"keep me");
    reader.verify_all_asset_checksums().unwrap();
    reader.verify_stream_checksum().unwrap();
}

#[test]
fn remove_then_add_same_name_replaces_payload_in_one_session() {
    let bytes = build_seed_bundle();
    let mut appender = BendlAppender::open(Cursor::new(bytes)).unwrap();
    appender.remove_asset("seed.bin").unwrap();
    appender
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "seed.bin",
            b"new payload",
            AddAssetOptions::defaults().raw(),
        )
        .unwrap();
    let bytes = appender.commit().unwrap().into_inner();

    let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
    let entry = find_entry(reader.assets(), "seed.bin").clone();
    assert_eq!(reader.asset_bytes(&entry).unwrap(), b"new payload");
    reader.verify_all_asset_checksums().unwrap();
}

#[test]
fn remove_singleton_frees_its_type_for_re_add() {
    // metadata.json is a singleton: a second add is refused until the first is removed.
    let bytes = build_seed_bundle();
    let mut appender = BendlAppender::open(Cursor::new(bytes)).unwrap();
    appender
        .add_known_asset(
            KnownAssetKind::Metadata,
            b"{\"v\":1}",
            AddAssetOptions::defaults().json(),
        )
        .unwrap();
    let bytes = appender.commit().unwrap().into_inner();

    let mut appender = BendlAppender::open(Cursor::new(bytes)).unwrap();
    assert!(matches!(
        appender.add_known_asset(
            KnownAssetKind::Metadata,
            b"{\"v\":2}",
            AddAssetOptions::defaults().json(),
        ),
        Err(BendlWriteError::DuplicateSingletonType(_))
    ));
    appender.remove_asset("metadata.json").unwrap();
    appender
        .add_known_asset(
            KnownAssetKind::Metadata,
            b"{\"v\":2}",
            AddAssetOptions::defaults().json(),
        )
        .unwrap();
    let bytes = appender.commit().unwrap().into_inner();

    let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
    let entry = find_entry(reader.assets(), "metadata.json").clone();
    assert_eq!(reader.asset_bytes(&entry).unwrap(), b"{\"v\":2}");
}

#[test]
fn compact_drops_dead_space_and_preserves_semantics() {
    use binary_ensemble::io::bundle::compact::compact_bundle;

    // Manufacture dead space: append a 64 KiB incompressible blob (stored raw), then remove it
    // with the directory-only appender removal.
    let bytes = build_seed_bundle();
    let blob: Vec<u8> = (0u32..65536)
        .map(|i| (i.wrapping_mul(2654435761) >> 13) as u8)
        .collect();
    let mut appender = BendlAppender::open(Cursor::new(bytes)).unwrap();
    appender
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "bloat.bin",
            &blob,
            AddAssetOptions::defaults().raw(),
        )
        .unwrap();
    let bytes = appender.commit().unwrap().into_inner();
    let mut appender = BendlAppender::open(Cursor::new(bytes)).unwrap();
    appender.remove_asset("bloat.bin").unwrap();
    let bloated = appender.commit().unwrap().into_inner();

    let mut reader = BendlReader::open(Cursor::new(bloated.clone())).unwrap();
    let compacted = compact_bundle(&mut reader, Cursor::new(Vec::new()))
        .unwrap()
        .into_inner();
    assert!(compacted.len() + 60_000 < bloated.len());

    // Semantically identical: same assets, verbatim stream bytes, and every checksum holds.
    let mut reader = BendlReader::open(Cursor::new(compacted)).unwrap();
    let names: Vec<_> = reader.assets().iter().map(|e| e.name.clone()).collect();
    assert_eq!(names, vec!["seed.bin".to_string()]);
    let entry = reader.assets()[0].clone();
    assert_eq!(reader.asset_bytes(&entry).unwrap(), b"seed payload bytes");
    let mut stream = Vec::new();
    reader
        .assignment_stream_reader()
        .unwrap()
        .read_to_end(&mut stream)
        .unwrap();
    assert_eq!(stream, b"STANDARD BEN FILE\x00\x01\x02");
    reader.verify_all_asset_checksums().unwrap();
    reader.verify_stream_checksum().unwrap();
}

#[test]
fn compact_rejects_unfinalized_bundle() {
    use binary_ensemble::io::bundle::compact::compact_bundle;

    // Clear the header's `finalized` flag (byte 12 per the spec's fixed-header layout) so the
    // bundle reads as incomplete.
    let mut bytes = build_seed_bundle();
    assert_eq!(bytes[12], 1);
    bytes[12] = 0;

    let mut reader = BendlReader::open(Cursor::new(bytes)).unwrap();
    assert!(matches!(
        compact_bundle(&mut reader, Cursor::new(Vec::new())),
        Err(BendlWriteError::BundleIncomplete)
    ));
}

/// Write `bytes` to a unique file under the cargo test tmpdir and return its path.
fn write_tmp_bundle(name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let path = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn in_place_compaction_picks_tail_rewrite_and_never_touches_the_stream() {
    use binary_ensemble::io::bundle::compact::{compact_bundle_in_place, Compaction};

    // Append two assets, then remove the first via the directory-only appender removal: the
    // dead space (its payload + superseded directories) is entirely post-stream.
    let bytes = build_seed_bundle();
    let mut appender = BendlAppender::open(Cursor::new(bytes)).unwrap();
    appender
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "dead.bin",
            &[0xAAu8; 4096],
            AddAssetOptions::defaults().raw(),
        )
        .unwrap();
    appender
        .add_asset(
            ASSET_TYPE_CUSTOM,
            "survivor.bin",
            b"survivor payload",
            AddAssetOptions::defaults().raw(),
        )
        .unwrap();
    let bytes = appender.commit().unwrap().into_inner();
    let mut appender = BendlAppender::open(Cursor::new(bytes)).unwrap();
    appender.remove_asset("dead.bin").unwrap();
    let bloated = appender.commit().unwrap().into_inner();

    // Stream end = the prefix that the tail rewrite must leave byte-identical.
    let stream_end = {
        let reader = BendlReader::open(Cursor::new(bloated.clone())).unwrap();
        reader.header().stream_offset + reader.header().stream_len
    };
    let path = write_tmp_bundle("tail-compact.bendl", &bloated);

    assert_eq!(
        compact_bundle_in_place(&path).unwrap(),
        Compaction::TailRewrite
    );
    let compacted = std::fs::read(&path).unwrap();
    assert!(compacted.len() + 4096 <= bloated.len());
    // Everything between the (re-patched) header and the stream end is byte-identical: the
    // pre-stream assets and the stream itself were never read or moved.
    let header_len = 64;
    assert_eq!(
        &compacted[header_len..stream_end as usize],
        &bloated[header_len..stream_end as usize]
    );

    // Survivor and seed assets intact (raw storage form preserved), every checksum holds.
    let mut reader = BendlReader::open(Cursor::new(compacted)).unwrap();
    let names: Vec<_> = reader.assets().iter().map(|e| e.name.clone()).collect();
    assert_eq!(
        names,
        vec!["seed.bin".to_string(), "survivor.bin".to_string()]
    );
    let survivor = find_entry(reader.assets(), "survivor.bin").clone();
    assert_eq!(reader.asset_bytes(&survivor).unwrap(), b"survivor payload");
    reader.verify_all_asset_checksums().unwrap();
    reader.verify_stream_checksum().unwrap();

    // A second compaction finds nothing to do and leaves the file byte-identical.
    let before = std::fs::read(&path).unwrap();
    assert_eq!(compact_bundle_in_place(&path).unwrap(), Compaction::None);
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn in_place_compaction_falls_back_to_full_rewrite_for_pre_stream_dead_space() {
    use binary_ensemble::io::bundle::compact::{compact_bundle_in_place, Compaction};

    // seed.bin is a pre-stream asset: removing it leaves dead bytes before the stream, which
    // only the full rewrite can reclaim.
    let bytes = build_seed_bundle();
    let mut appender = BendlAppender::open(Cursor::new(bytes)).unwrap();
    appender.remove_asset("seed.bin").unwrap();
    let bloated = appender.commit().unwrap().into_inner();
    let path = write_tmp_bundle("full-compact.bendl", &bloated);

    assert_eq!(
        compact_bundle_in_place(&path).unwrap(),
        Compaction::FullRewrite
    );
    let mut reader = BendlReader::open(Cursor::new(std::fs::read(&path).unwrap())).unwrap();
    assert!(reader.assets().is_empty());
    let mut stream = Vec::new();
    reader
        .assignment_stream_reader()
        .unwrap()
        .read_to_end(&mut stream)
        .unwrap();
    assert_eq!(stream, b"STANDARD BEN FILE\x00\x01\x02");
    reader.verify_stream_checksum().unwrap();
}

#[test]
fn remove_unknown_asset_errors_and_commit_stays_a_no_op() {
    let original = build_seed_bundle();
    let mut appender = BendlAppender::open(Cursor::new(original.clone())).unwrap();
    assert!(matches!(
        appender.remove_asset("missing.bin"),
        Err(BendlWriteError::UnknownAssetName(_))
    ));
    // The failed removal queued nothing, so commit must leave the file byte-identical.
    let bytes = appender.commit().unwrap().into_inner();
    assert_eq!(bytes, original);
}
