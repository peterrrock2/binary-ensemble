//! Decoding routines for BEN and XBEN formats.

mod ben;
mod ben32;
pub(crate) mod errors;
pub(crate) use errors::DecodeError;
mod jsonl;
mod twodelta;
mod xz;

pub use ben::decode_ben_line;
pub(crate) use ben32::{decode_ben32_line, jsonl_decode_ben32};
pub use jsonl::{decode_ben_to_jsonl, decode_xben_to_jsonl};
pub(crate) use twodelta::apply_twodelta_runs_to_assignment;
pub use twodelta::decode_twodelta_frame;
pub use xz::{decode_xben_to_ben, xz_decompress};

#[cfg(test)]
mod tests;
