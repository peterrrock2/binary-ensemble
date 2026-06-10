//! Decoding routines for BEN and XBEN formats.

mod ben;
mod ben32;
pub(crate) mod errors;
pub(crate) use errors::DecodeError;
mod jsonl;
pub mod path;
mod twodelta;
mod xz;

pub use ben::decode_ben_line;
pub(crate) use ben32::decode_ben32_line;
pub use jsonl::{decode_ben_to_jsonl, decode_xben_to_jsonl};
pub use path::{
    decode_ben_to_jsonl_path, decode_xben_to_ben_path, decode_xben_to_jsonl_path,
    xz_decompress_path,
};
pub(crate) use twodelta::apply_twodelta_runs_to_assignment;
pub use twodelta::decode_twodelta_frame;
pub use xz::{decode_xben_to_ben, xz_decompress};

#[cfg(test)]
mod tests;
