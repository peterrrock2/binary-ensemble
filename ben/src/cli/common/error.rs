//! Error type used by the top-level `run()` functions of every CLI binary.
//!
//! The shape is intentionally narrow: a few specific variants for cases where a caller (or test)
//! might want to match the error type, plus an `Other` catch-all that preserves the older
//! `Result<(), String>` ergonomic so the existing per-command runners still propagate cleanly via
//! `?`.

use std::fmt;
use std::io;
use std::path::PathBuf;

/// Error returned by the top-level CLI `run()` functions.
#[derive(Debug)]
pub enum CliError {
    /// An underlying I/O error (file open, read, write, etc.).
    Io(io::Error),
    /// The output path already existed and the user declined to overwrite.
    OverwriteRefused(PathBuf),
    /// A free-form error message. Used as a catch-all so existing `Result<(), String>` runners
    /// still flow through unchanged.
    Other(String),
}

/// Convenience alias for `Result<T, CliError>`.
pub type CliResult<T = ()> = Result<T, CliError>;

impl CliError {
    /// Construct a free-form error from anything that displays to a string.
    pub fn other<S: Into<String>>(s: S) -> Self {
        CliError::Other(s.into())
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::Io(e) => write!(f, "{e}"),
            CliError::OverwriteRefused(p) => {
                write!(f, "user declined to overwrite {}", p.display())
            }
            CliError::Other(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for CliError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CliError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for CliError {
    fn from(e: io::Error) -> Self {
        CliError::Io(e)
    }
}

impl From<String> for CliError {
    fn from(s: String) -> Self {
        CliError::Other(s)
    }
}

impl From<&str> for CliError {
    fn from(s: &str) -> Self {
        CliError::Other(s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_error_round_trips_via_from() {
        let original = io::Error::new(io::ErrorKind::NotFound, "missing");
        let cli: CliError = original.into();
        assert!(matches!(cli, CliError::Io(_)));
        assert_eq!(cli.to_string(), "missing");
    }

    #[test]
    fn other_constructor_accepts_string_and_str() {
        let a: CliError = "boom".into();
        assert_eq!(a.to_string(), "boom");
        let b: CliError = String::from("kapow").into();
        assert_eq!(b.to_string(), "kapow");
        let c = CliError::other("bang");
        assert_eq!(c.to_string(), "bang");
    }

    #[test]
    fn overwrite_refused_displays_path() {
        let e = CliError::OverwriteRefused(PathBuf::from("/tmp/out.bin"));
        assert!(e.to_string().contains("/tmp/out.bin"));
        assert!(e.to_string().contains("declined"));
    }

    #[test]
    fn io_source_propagates() {
        use std::error::Error;
        let original = io::Error::other("deep");
        let cli = CliError::Io(original);
        assert!(cli.source().is_some());
    }
}
