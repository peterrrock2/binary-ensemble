pub mod error;
pub use error::{CliError, CliResult};

use std::io::{self, Result};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

static QUIET: AtomicBool = AtomicBool::new(false);

/// Configure tracing for CLI execution.
///
/// When `verbose` is set and the user has not already provided `RUST_LOG`, the default log filter
/// is elevated to `trace`. The tracing subscriber is then initialized exactly once for the process.
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

/// Suppress in-place progress spinners for this process.
///
/// Independent of [`set_verbose`]: trace logging is gated by `RUST_LOG`, while spinners are gated
/// by this flag plus stderr TTY detection.
///
/// # Arguments
///
/// * `quiet` - When `true`, [`crate::progress::Spinner`] becomes a no-op.
///
/// # Returns
///
/// This function does not return a value.
pub fn set_quiet(quiet: bool) {
    QUIET.store(quiet, Ordering::Relaxed);
}

/// Whether progress spinners have been globally suppressed.
///
/// # Returns
///
/// Returns `true` when [`set_quiet`] was last called with `true`.
pub fn is_quiet() -> bool {
    QUIET.load(Ordering::Relaxed)
}

/// Decide whether overwriting an output path should proceed, given the state observed by the
/// caller.
///
/// This is the pure half of [`check_overwrite`]: it does no I/O, so it can be unit-tested by
/// enumerating the four reachable states (file missing / `overwrite` flag set / user said yes /
/// user said anything else).
///
/// # Arguments
///
/// * `file_exists` - Whether the candidate output path already exists.
/// * `overwrite` - Whether the caller passed `--overwrite` to skip prompting.
/// * `response` - The line the user typed in response to the overwrite prompt, or `None` if no
///   prompt was issued.
///
/// # Returns
///
/// Returns `true` when the caller may safely overwrite; `false` when the user (or the absence of a
/// yes-response) indicates the operation should be aborted.
pub(crate) fn check_overwrite_pure(
    file_exists: bool,
    overwrite: bool,
    response: Option<&str>,
) -> bool {
    if !file_exists || overwrite {
        return true;
    }
    matches!(
        response.map(|s| s.trim().to_lowercase()).as_deref(),
        Some("y") | Some("yes")
    )
}

/// Confirm whether an existing output path may be overwritten.
///
/// If `overwrite` is `false` and the destination already exists, the user is prompted on stdin. An
/// `AlreadyExists` error is returned when the user declines.
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
    let exists = Path::new(file_name).exists();
    let response = if exists && !overwrite {
        eprint!(
            "File {:?} already exists, do you want to overwrite it? (y/[n]): ",
            file_name
        );
        let mut buf = String::new();
        io::stdin().read_line(&mut buf)?;
        eprintln!();
        Some(buf)
    } else {
        None
    };
    if check_overwrite_pure(exists, overwrite, response.as_deref()) {
        Ok(())
    } else {
        Err(io::Error::from(io::ErrorKind::AlreadyExists))
    }
}

#[cfg(test)]
mod tests;
