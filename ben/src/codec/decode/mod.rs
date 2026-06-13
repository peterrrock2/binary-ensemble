//! Decoding routines for BEN and XBEN formats.

/// Upper bound on the *expanded* length of a single decoded assignment (the sum of a frame's run
/// lengths, i.e. the number of dual-graph nodes). Each 4-byte ben32 run can legally demand up to
/// 65,535 elements, so without a bound on the sum a small malformed frame could request a
/// multi-gigabyte expansion from a few kilobytes of input. The cap is a reader-side sanity bound,
/// not a wire-format limit: at ~134 million nodes it sits more than an order of magnitude above
/// any real dual graph (national census-block graphs run ~10 million nodes) while keeping the
/// worst-case single-assignment allocation at 256 MiB.
pub(crate) const MAX_ASSIGNMENT_LEN: u64 = 1 << 27;

mod ben;
mod ben32;
pub(crate) mod errors;
pub(crate) use errors::DecodeError;
mod jsonl;
pub mod path;
mod twodelta;
mod xz;

pub use ben::decode_ben_line;
pub(crate) use ben::MAX_FRAME_PAYLOAD_BYTES;
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
