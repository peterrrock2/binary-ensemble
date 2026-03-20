//! Decoding routines for BEN and XBEN formats.

mod ben;
mod ben32;
pub(crate) mod errors;
pub(crate) use errors::DecodeError;
mod xz;

pub use ben::{decode_ben_line, decode_ben_to_jsonl};
pub(crate) use ben32::{decode_ben32_line, jsonl_decode_ben32};
pub use xz::{decode_xben_to_ben, decode_xben_to_jsonl, xz_decompress};

#[cfg(test)]
mod tests;
