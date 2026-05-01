use super::*;
use std::fs;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn unique_path(name: &str) -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("ben-cli-common-{name}-{nonce}"))
}

#[test]
fn set_verbose_sets_rust_log() {
    let _guard = env_lock().lock().unwrap();
    std::env::remove_var("RUST_LOG");
    set_verbose(true);
    assert_eq!(std::env::var("RUST_LOG").as_deref(), Ok("trace"));
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
