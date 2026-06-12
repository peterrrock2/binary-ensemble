use std::fs::{self, File, OpenOptions};
use std::io::BufReader;
use std::path::PathBuf;

use super::writer::build_base_bundle;
use crate::io::bundle::compact::{compact_bundle_in_place, plan_tail, stage_tail, Compaction};
use crate::io::bundle::reader::BendlReader;
use crate::io::bundle::writer::{AddAssetOptions, BendlAppender};

/// A temp file that removes itself on drop, so failed assertions don't leak files.
struct TempBundle(PathBuf);

impl Drop for TempBundle {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn temp_bundle(bytes: &[u8], tag: &str) -> TempBundle {
    let path = std::env::temp_dir().join(format!(
        "bendl-compact-test-{}-{tag}.bendl",
        std::process::id()
    ));
    fs::write(&path, bytes).unwrap();
    TempBundle(path)
}

#[test]
fn in_place_compaction_rejects_out_of_bounds_payload_len_without_panicking() {
    // A corrupt directory length used to reach the tail planner and size an allocation from the
    // untrusted value (an abort on adversarial input); open-time extent validation must surface
    // it as an error and leave the file untouched.
    let (mut bytes, _) = build_base_bundle();
    let directory_offset = u64::from_le_bytes(bytes[24..32].try_into().unwrap()) as usize;
    let payload_len_offset = directory_offset + 4 + 16;
    bytes[payload_len_offset..payload_len_offset + 8].copy_from_slice(&(1u64 << 48).to_le_bytes());

    let tmp = temp_bundle(&bytes, "oob-payload");
    let before = fs::read(&tmp.0).unwrap();
    let err = compact_bundle_in_place(&tmp.0).unwrap_err();
    assert!(
        err.to_string().contains("beyond the file end"),
        "unexpected error: {err}"
    );
    assert_eq!(
        fs::read(&tmp.0).unwrap(),
        before,
        "failed compaction must leave the file untouched"
    );
}

#[test]
fn crash_after_stage_leaves_consistent_bundle_and_recompaction_recovers() {
    // The tail rewrite stages the survivors plus a directory addressing those staged copies at
    // EOF, adopts it, and only then rewrites the final tail. Simulate a crash immediately after
    // the staged adoption (the widest window) and check both halves of the crash-safety
    // invariant: the staged state is a fully intact bundle, and re-running compaction from it
    // converges losslessly. The staged directory used to carry the survivors' *final* offsets —
    // bytes not yet written — so in the crash state the survivor failed its checksum, and the
    // re-run rewrote it from those dead bytes and truncated away the only good copy.
    let (bytes, _) = build_base_bundle();
    let tmp = temp_bundle(&bytes, "crash-after-stage");
    let path = tmp.0.clone();
    let open_rw = || {
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap()
    };

    // Two appends (each leaves a superseded directory behind), then a directory-only removal of
    // the first appended asset: the second must move left when the tail is compacted.
    let survivor_payload: Vec<u8> = (0..600u32).map(|i| (i * 7 % 251) as u8).collect();
    let mut appender = BendlAppender::open(open_rw()).unwrap();
    appender
        .add_custom_asset("doomed.bin", &[0xAB; 700], AddAssetOptions::defaults().raw())
        .unwrap();
    appender.commit().unwrap();
    let mut appender = BendlAppender::open(open_rw()).unwrap();
    appender
        .add_custom_asset(
            "survivor.bin",
            &survivor_payload,
            AddAssetOptions::defaults().raw(),
        )
        .unwrap();
    appender.commit().unwrap();
    let mut appender = BendlAppender::open(open_rw()).unwrap();
    appender.remove_asset("doomed.bin").unwrap();
    appender.commit().unwrap();

    // Run phase 1 only — up to the moment the staged directory becomes authoritative — then
    // "crash" by dropping the handle before phase 2.
    {
        let reader = BendlReader::open(BufReader::new(File::open(&path).unwrap())).unwrap();
        let mut header = *reader.header();
        let entries = reader.assets().to_vec();
        drop(reader);
        let mut file = open_rw();
        let plan = plan_tail(&mut file, &header, &entries)
            .unwrap()
            .expect("post-stream dead space must be tail-compactable");
        stage_tail(&mut file, &mut header, &plan).unwrap();
    }

    // The crash state must be a fully consistent bundle: the directory loads, every checksum
    // holds, and the survivor reads back intact.
    {
        let mut reader = BendlReader::open(BufReader::new(File::open(&path).unwrap())).unwrap();
        reader.verify_all_asset_checksums().unwrap();
        let entry = reader.find_asset_by_name("survivor.bin").cloned().unwrap();
        assert_eq!(reader.asset_bytes(&entry).unwrap(), survivor_payload);
        assert!(reader.find_asset_by_name("doomed.bin").is_none());
    }

    // Recovery is the natural next step: re-running compaction must converge to the compact
    // bundle with the survivor intact, and a further run must find nothing to do.
    assert_eq!(
        compact_bundle_in_place(&path).unwrap(),
        Compaction::TailRewrite
    );
    let mut reader = BendlReader::open(BufReader::new(File::open(&path).unwrap())).unwrap();
    reader.verify_all_asset_checksums().unwrap();
    let entry = reader.find_asset_by_name("survivor.bin").cloned().unwrap();
    assert_eq!(reader.asset_bytes(&entry).unwrap(), survivor_payload);
    assert!(reader.find_asset_by_name("metadata.json").is_some());
    assert_eq!(compact_bundle_in_place(&path).unwrap(), Compaction::None);
}
