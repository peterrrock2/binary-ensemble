use std::fs;
use std::path::PathBuf;

use super::writer::build_base_bundle;
use crate::io::bundle::compact::compact_bundle_in_place;

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
