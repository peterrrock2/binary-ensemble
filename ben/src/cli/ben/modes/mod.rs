//! Encode/decode handlers for the `ben` CLI.
//!
//! The dispatcher in `super::run` matches on the parsed `Command` and forwards to one of these
//! handlers. Splitting one handler per file keeps each mode small and individually testable.

pub(super) mod decode;
pub(super) mod encode;
pub(super) mod lookup;
pub(super) mod xdecode;
pub(super) mod xencode;
pub(super) mod xz_compress;
pub(super) mod xz_decompress;
