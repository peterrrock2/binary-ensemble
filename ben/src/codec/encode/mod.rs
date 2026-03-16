//! Encoding routines for BEN and XBEN formats.

mod ben;
mod jsonl;
mod types;
mod xz;

pub(crate) use ben::encode_ben32_line;
pub use ben::{encode_ben_vec_from_assign, encode_ben_vec_from_rle, encode_twodelta_vec};
pub use jsonl::{encode_jsonl_to_ben, encode_jsonl_to_xben};
pub use types::{BenFrame, IdItem, IdVec, TwoDeltaFrame};
pub use xz::{encode_ben_to_xben, xz_compress};

#[cfg(test)]
mod tests;
