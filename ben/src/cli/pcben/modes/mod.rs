//! Per-mode handlers for the `pcben` CLI.
//!
//! The dispatcher in `super::run` matches on the parsed `Mode` enum and
//! forwards to one of these handlers. Splitting one handler per file keeps
//! each mode under ~40 lines and makes them individually testable.

pub(super) mod ben_to_pc;
pub(super) mod pc_to_ben;
pub(super) mod pc_to_xben;
