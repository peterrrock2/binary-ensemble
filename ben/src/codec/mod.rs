//! Core format conversion logic for BEN-related representations.
//!
//! This module is split into three layers:
//! - [`encode`] for producing BEN or XBEN streams
//! - [`decode`] for recovering BEN, XBEN, or JSONL data
//! - [`translate`] for converting between BEN frames and their ben32 form

pub mod decode;
pub mod encode;
pub mod frames;
pub mod translate;

pub use frames::{BenDecodeFrame, BenEncodeFrame};
