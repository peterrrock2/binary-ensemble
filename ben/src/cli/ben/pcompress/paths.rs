//! Output-path resolution for the `ben pcompress` directions.

use crate::cli::common::check_overwrite;
use std::io;

/// Which way a `pcompress` conversion runs, used to derive default output extensions.
#[derive(Clone, Copy)]
pub(super) enum PcDirection {
    /// BEN -> PCOMPRESS.
    FromBen,
    /// PCOMPRESS -> BEN.
    ToBen,
    /// PCOMPRESS -> XBEN.
    ToXben,
}

/// Resolve the output file path for a `pcompress` direction.
pub(super) fn resolved_output_path(
    dir: PcDirection,
    input_file: Option<&str>,
    output_file: Option<&str>,
    overwrite: bool,
) -> io::Result<Option<String>> {
    let Some(path) = output_file
        .map(ToOwned::to_owned)
        .or_else(|| input_file.map(|input| derive_output_path(dir, input)))
    else {
        return Ok(None);
    };

    check_overwrite(&path, overwrite)?;
    Ok(Some(path))
}

/// Derive the default output file name for a `pcompress` direction.
pub(super) fn derive_output_path(dir: PcDirection, input_file: &str) -> String {
    match dir {
        PcDirection::FromBen => input_file
            .strip_suffix(".ben")
            .map(|prefix| format!("{prefix}.pcompress"))
            .unwrap_or_else(|| format!("{input_file}.pcompress")),
        PcDirection::ToBen => input_file
            .strip_suffix(".pcompress")
            .or_else(|| input_file.strip_suffix(".pc"))
            .map(|prefix| format!("{prefix}.ben"))
            .unwrap_or_else(|| format!("{input_file}.ben")),
        PcDirection::ToXben => input_file
            .strip_suffix(".pcompress")
            .or_else(|| input_file.strip_suffix(".pc"))
            .map(|prefix| format!("{prefix}.xben"))
            .unwrap_or_else(|| format!("{input_file}.xben")),
    }
}
