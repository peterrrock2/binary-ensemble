//! Encoding routines for BEN and XBEN formats.

mod ben;
pub mod errors;
mod jsonl;
mod twodelta;
mod xz;

pub(crate) use ben::encode_ben32_assignments;
pub use errors::EncodeError;
pub use twodelta::encode_twodelta_frame;
pub(crate) use twodelta::encode_twodelta_frame_with_hint;

#[cfg(test)]
pub(crate) use ben::encode_ben32_line;
pub use jsonl::{encode_jsonl_to_ben, encode_jsonl_to_xben};
pub use xz::{encode_ben_to_xben, xz_compress};

#[cfg(test)]
mod tests;
