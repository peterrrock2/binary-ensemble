//! `ben pcompress`: convert between BEN/XBEN and the foreign PCOMPRESS format.

mod modes;
mod paths;
mod translate;

#[cfg(test)]
mod tests;

use super::args::{Globals, PcompressArgs, PcompressDirection};
use crate::cli::common::CliResult;

/// Dispatch a `pcompress` conversion to the handler for the requested direction.
pub(super) fn run(args: PcompressArgs, g: &Globals) -> CliResult {
    match args.direction {
        PcompressDirection::FromBen(io) => modes::from_ben::run(io, g),
        PcompressDirection::ToBen(io) => modes::to_ben::run(io, g),
        PcompressDirection::ToXben(io) => modes::to_xben::run(io, g),
    }
}
