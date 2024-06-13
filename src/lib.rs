//! This crate provides several command line tools and functions for converting
//! ensembles of districting plans contained in a JSONL file with lines of the
//! form
//!
//! ```text
//! {"assignment": <assignment>, "sample": <sample>}
//! ```
//!
//! into binary ensembles (BEN) and extremely compressed binary ensembles
//! (XBEN). It also provides several tools for working with these files
//! including several tools for relabeling the ensembles to improve
//! compression ratios.
//!
//! The main CLI tools provided by this crate are:
//!
//! - `ben`: A tool for converting JSONL files into BEN files.
//!    and for converting between BEN and XBEN files.
//! - `reben`: A tool for relabeling BEN files to improve compression ratios.
//!

pub mod decode;
pub mod encode;
pub mod utils;

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {{
        if let Ok(log_level) = std::env::var("RUST_LOG") {
            if log_level.to_lowercase() == "trace" {
                eprint!($($arg)*);
            }
        }
    }}
}

#[macro_export]
macro_rules! logln {
    ($($arg:tt)*) => {{
        if let Ok(log_level) = std::env::var("RUST_LOG") {
            if log_level.to_lowercase() == "trace" {
                eprintln!($($arg)*);
            }
        }
    }}
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BenVariant {
    Standard,
    MkvChain,
}
