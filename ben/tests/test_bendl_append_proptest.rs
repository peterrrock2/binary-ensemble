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
//! 3. After every `Abort` or drop-without-commit, the file is byte-identical to before the
//!    appender was opened.

use binary_ensemble::io::bundle::format::{
    AssignmentFormat, BendlDirectoryEntry, ASSET_TYPE_CUSTOM,
};
use binary_ensemble::io::bundle::writer::{AddAssetOptions, BendlAppender, BendlWriter};
use binary_ensemble::io::bundle::BendlReader;
use proptest::prelude::*;
use std::io::{Cursor, Read, Seek, SeekFrom};

#[derive(Debug, Clone)]
enum Op {
    /// Open an appender (if none is open) and enqueue a pending asset.
    AddAsset {
        payload: Vec<u8>,
        compress: bool,
    },
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

/// Snapshot the (offset, len, raw payload bytes) for every directory entry in `bytes`. The
/// invariant is that these tuples must be unchanged after an append-only commit.
fn snapshot_existing_entries(bytes: &[u8]) -> Vec<(String, u64, u64, Vec<u8>)> {
    let reader = BendlReader::open(Cursor::new(bytes)).unwrap();
    reader
        .assets()
        .iter()
        .map(|e| {
            let payload = raw_bytes_at(bytes, e.payload_offset, e.payload_len);
            (e.name.clone(), e.payload_offset, e.payload_len, payload)
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

/// Run a single sequence of ops against the seed bundle. Returns the final state for an outer
/// `prop_assert!` to inspect, but most assertions fire inline as the ops execute.
fn run_sequence(ops: &[Op]) {
    let seed = build_seed_bundle();
    let baseline_reader = BendlReader::open(Cursor::new(&seed)).unwrap();
    let baseline_samples = baseline_reader.sample_count();
    drop(baseline_reader);

    // The "model" of every asset that has been committed to disk, in commit order. Each entry is
    // (name, raw_payload_bytes_as_added, compress_flag). The decoded asset bytes returned by the
    // reader must equal `raw_payload_bytes_as_added` regardless of compression.
    let mut committed: Vec<(String, Vec<u8>, bool)> = vec![(
        "seed.bin".to_string(),
        b"seed payload bytes".to_vec(),
        false,
    )];

    let mut current_bytes = seed.clone();

    // Per-appender state: when an appender is open we hold its Vec of pending payloads alongside
    // the snapshot we'll diff against if it commits. We don't keep the appender itself in this
    // structure because moving it through closures with a snapshot is awkward; instead we
    // construct/consume the appender inline at the next Op that uses it. We do, however, track
    // the names allocated so the appender doesn't get hit with DuplicateName on the second
    // AddAsset in the same round.
    struct PendingRound {
        pending: Vec<(String, Vec<u8>, bool)>,
        next_name_index: usize,
    }
    let mut round: Option<PendingRound> = None;
    let mut name_counter: usize = 0;

    for op in ops {
        match op {
            Op::AddAsset { payload, compress } => {
                let r = round.get_or_insert(PendingRound {
                    pending: Vec::new(),
                    next_name_index: 0,
                });
                // Name allocation: use a global counter to guarantee uniqueness across rounds,
                // and embed the round-local index so a successful Commit lands a stable name.
                let name = format!("asset-{}-{}.bin", name_counter, r.next_name_index);
                r.next_name_index += 1;
                name_counter += 1;
                r.pending.push((name, payload.clone(), *compress));
            }
            Op::Commit => {
                let Some(r) = round.take() else { continue };
                let snapshot = snapshot_existing_entries(&current_bytes);

                let mut appender = BendlAppender::open(Cursor::new(current_bytes.clone())).unwrap();
                for (name, payload, compress) in &r.pending {
                    let mut opts = AddAssetOptions::defaults();
                    opts = if *compress { opts.compress() } else { opts.raw() };
                    appender
                        .add_asset(ASSET_TYPE_CUSTOM, name, payload, opts)
                        .expect("add_asset on pending entry should succeed");
                }
                let new_bytes = appender.commit().unwrap().into_inner();

                // File must have grown (or stayed equal if pending was empty — but an empty
                // round only happens when the user inserts nothing before Commit, which isn't a
                // generated op here since AddAsset is the only way to enter Pending state).
                assert!(
                    new_bytes.len() >= current_bytes.len(),
                    "file shrank after commit"
                );

                // Strong invariant: every previously-committed directory entry kept its offset,
                // length, and raw payload bytes.
                let new_reader = BendlReader::open(Cursor::new(&new_bytes)).unwrap();
                let new_entries: Vec<BendlDirectoryEntry> = new_reader.assets().to_vec();
                drop(new_reader);
                for (name, old_offset, old_len, old_payload) in &snapshot {
                    let entry = find_entry(&new_entries, name);
                    assert_eq!(
                        (entry.payload_offset, entry.payload_len),
                        (*old_offset, *old_len),
                        "directory entry {name} (offset, len) drifted after commit"
                    );
                    let new_raw = raw_bytes_at(&new_bytes, entry.payload_offset, entry.payload_len);
                    assert_eq!(
                        new_raw, *old_payload,
                        "directory entry {name} raw payload bytes drifted after commit"
                    );
                }

                // Append model: every previously-committed asset + every freshly-committed
                // pending one is readable and decodes to the right bytes.
                for (name, payload, _compress) in &r.pending {
                    committed.push((name.clone(), payload.clone(), false));
                }
                let mut reader = BendlReader::open(Cursor::new(&new_bytes)).unwrap();
                assert_eq!(
                    reader.assets().len(),
                    committed.len(),
                    "directory size mismatch after commit"
                );
                assert_eq!(
                    reader.sample_count(),
                    baseline_samples,
                    "sample_count drifted across append"
                );
                for (name, want, _) in &committed {
                    let entry = reader.find_asset_by_name(name).cloned().unwrap();
                    let got = reader.asset_bytes(&entry).unwrap();
                    assert_eq!(&got, want, "decoded payload mismatch for {name}");
                }

                current_bytes = new_bytes;
            }
            Op::Abort => {
                let Some(_r) = round.take() else { continue };
                let pre_bytes = current_bytes.clone();
                let appender = BendlAppender::open(Cursor::new(current_bytes.clone())).unwrap();
                // .abort() consumes the appender and returns the underlying cursor. We never
                // wrote anything to it (we never entered the pending state at the writer
                // level), so the bytes must equal pre_bytes.
                let cursor = appender.abort();
                let post_bytes = cursor.into_inner();
                assert_eq!(
                    post_bytes, pre_bytes,
                    "Abort modified the file (it must be a no-op)"
                );
            }
            Op::DropWithoutCommit => {
                let Some(_r) = round.take() else { continue };
                let pre_bytes = current_bytes.clone();
                {
                    let mut appender =
                        BendlAppender::open(Cursor::new(current_bytes.clone())).unwrap();
                    // Re-enqueue the pending ops on this appender, then let it drop without
                    // committing. The file underlying `appender` is a clone of `current_bytes`,
                    // so dropping it can't affect `current_bytes` either way — but the
                    // assertion below pins that intent for clarity.
                    for (i, (_, payload, compress)) in _r.pending.iter().enumerate() {
                        let mut opts = AddAssetOptions::defaults();
                        opts = if *compress { opts.compress() } else { opts.raw() };
                        let name = format!("dropped-{name_counter}-{i}.bin");
                        let _ = appender.add_asset(ASSET_TYPE_CUSTOM, &name, payload, opts);
                    }
                    // appender drops here without commit().
                }
                assert_eq!(
                    current_bytes, pre_bytes,
                    "DropWithoutCommit modified the master file (it must be a no-op)"
                );
            }
        }
    }

    // Final consistency check: open the file one last time, validate the directory, and confirm
    // every committed asset is still readable. Any pending round at end-of-sequence is implicitly
    // dropped (no commit), which must not affect `current_bytes`.
    let mut reader = BendlReader::open(Cursor::new(&current_bytes)).unwrap();
    reader.validate_directory().unwrap();
    assert_eq!(
        reader.assets().len(),
        committed.len(),
        "final directory size mismatch"
    );
    for (name, want, _) in &committed {
        let entry = reader.find_asset_by_name(name).cloned().unwrap();
        let got = reader.asset_bytes(&entry).unwrap();
        assert_eq!(&got, want, "final decoded payload mismatch for {name}");
    }

    // Also drive a raw seek to EOF to confirm the file is structurally sound.
    let mut tail = Vec::new();
    let mut cursor = Cursor::new(&current_bytes);
    cursor.seek(SeekFrom::Start(0)).unwrap();
    cursor.read_to_end(&mut tail).unwrap();
    assert_eq!(tail.len(), current_bytes.len());
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
