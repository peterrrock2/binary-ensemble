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
mod tests;
