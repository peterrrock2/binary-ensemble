use std::io::{self, Result};
use std::path::Path;

/// Configure tracing for CLI execution.
///
/// When `verbose` is set and the user has not already provided `RUST_LOG`, the
/// default log filter is elevated to `trace`. The tracing subscriber is then
/// initialized exactly once for the process.
///
/// # Arguments
///
/// * `verbose` - Whether verbose trace logging should be enabled by default.
///
/// # Returns
///
/// This function does not return a value.
pub fn set_verbose(verbose: bool) {
    if verbose && std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "trace");
    }
    crate::logging::init_logging();
}

/// Confirm whether an existing output path may be overwritten.
///
/// If `overwrite` is `false` and the destination already exists, the user is
/// prompted on stdin. An `AlreadyExists` error is returned when the user
/// declines.
///
/// # Arguments
///
/// * `file_name` - The candidate output path.
/// * `overwrite` - Whether to skip the interactive overwrite prompt.
///
/// # Returns
///
/// Returns `Ok(())` when the output path may be used.
pub fn check_overwrite(file_name: &str, overwrite: bool) -> Result<()> {
    if Path::new(file_name).exists() && !overwrite {
        eprint!(
            "File {:?} already exists, do you want to overwrite it? (y/[n]): ",
            file_name
        );
        let mut user_input = String::new();
        io::stdin().read_line(&mut user_input).unwrap();
        eprintln!();
        if user_input.trim().to_lowercase() != "y" {
            return Err(io::Error::from(io::ErrorKind::AlreadyExists));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
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
}
