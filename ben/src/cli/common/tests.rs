use super::*;
use crate::test_utils::unique_path;
use std::fs;
use std::sync::{Mutex, OnceLock};

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[test]
fn set_verbose_sets_rust_log() {
    let _guard = env_lock().lock().unwrap();
    std::env::remove_var("RUST_LOG");
    set_verbose(true);
    assert_eq!(std::env::var("RUST_LOG").as_deref(), Ok("info"));
}

#[test]
fn set_verbose_preserves_existing_log_level() {
    let _guard = env_lock().lock().unwrap();
    std::env::set_var("RUST_LOG", "debug");
    set_verbose(true);
    assert_eq!(std::env::var("RUST_LOG").as_deref(), Ok("debug"));
    std::env::remove_var("RUST_LOG");
}

#[test]
fn set_verbose_initializes_logger_without_setting_trace() {
    let _guard = env_lock().lock().unwrap();
    std::env::remove_var("RUST_LOG");
    set_verbose(false);
    assert!(std::env::var("RUST_LOG").is_err());
}

#[test]
fn check_overwrite_allows_missing_file() {
    let path = unique_path("missing.txt");
    assert!(!path.exists());
    check_overwrite(path.to_str().unwrap(), false).unwrap();
}

#[test]
fn check_overwrite_allows_existing_file_when_forced() {
    let path = unique_path("existing.txt");
    fs::write(&path, "hello").unwrap();
    check_overwrite(path.to_str().unwrap(), true).unwrap();
    fs::remove_file(path).unwrap();
}

#[test]
fn check_overwrite_pure_passes_when_file_missing() {
    assert!(check_overwrite_pure(false, false, None));
    assert!(check_overwrite_pure(false, true, None));
}

#[test]
fn check_overwrite_pure_passes_when_overwrite_flag_set() {
    assert!(check_overwrite_pure(true, true, None));
}

#[test]
fn check_overwrite_pure_accepts_y_and_yes_responses() {
    assert!(check_overwrite_pure(true, false, Some("y\n")));
    assert!(check_overwrite_pure(true, false, Some("Y\n")));
    assert!(check_overwrite_pure(true, false, Some("yes\n")));
    assert!(check_overwrite_pure(true, false, Some("  YES  ")));
}

#[test]
fn check_overwrite_pure_rejects_other_responses() {
    assert!(!check_overwrite_pure(true, false, Some("n\n")));
    assert!(!check_overwrite_pure(true, false, Some("\n")));
    assert!(!check_overwrite_pure(true, false, Some("maybe\n")));
    assert!(!check_overwrite_pure(true, false, None));
}
