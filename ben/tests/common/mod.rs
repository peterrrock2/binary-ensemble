//! Helpers shared across `ben/tests/*.rs` integration tests.
//!
//! Each integration test crate declares `mod common;` to opt in. The module re-exports the in-crate
//! test utilities from `binary_ensemble::test_utils` and adds integration-only helpers
//! (subprocess paths, etc.).

#![allow(dead_code, unused_imports)]

pub use binary_ensemble::test_utils::{
    expand_rle, jsonl_from_assignments, sample_ben_bytes, sample_bendl_bytes, unique_path,
};

/// Path to a compiled binary for shelling out from integration tests.
///
/// Returns the same `env!("CARGO_BIN_EXE_*")` value the existing test helpers use; centralised here
/// so future CLI tests can pick up the canonical lookup table.
pub fn binary_path(name: &str) -> &'static str {
    match name {
        "ben" => env!("CARGO_BIN_EXE_ben"),
        "bendl" => env!("CARGO_BIN_EXE_bendl"),
        _ => panic!("unknown binary {name}"),
    }
}
