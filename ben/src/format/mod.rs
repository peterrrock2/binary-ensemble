//! Shared on-disk format metadata for BEN and XBEN streams.

pub mod banners;
pub mod errors;
pub(crate) mod tags;
pub use errors::FormatError;

#[cfg(test)]
mod tests;
