//! Output-path resolution helpers for the `pcben` CLI.

use super::args::Mode;
use crate::cli::common::check_overwrite;
use std::io;

/// Resolve the output file path for a `pcben` mode.
pub(super) fn resolved_output_path(
    mode: Mode,
    input_file: Option<&str>,
    output_file: Option<&str>,
    overwrite: bool,
) -> io::Result<Option<String>> {
    let Some(path) = output_file
        .map(ToOwned::to_owned)
        .or_else(|| input_file.map(|input| derive_output_path(mode, input)))
    else {
        return Ok(None);
    };

    check_overwrite(&path, overwrite)?;
    Ok(Some(path))
}

/// Derive the default output file name for a `pcben` conversion mode.
pub(super) fn derive_output_path(mode: Mode, input_file: &str) -> String {
    match mode {
        Mode::BenToPc => input_file
            .strip_suffix(".ben")
            .map(|prefix| format!("{prefix}.pcompress"))
            .unwrap_or_else(|| format!("{input_file}.pcompress")),
        Mode::PcToBen => input_file
            .strip_suffix(".pcompress")
            .or_else(|| input_file.strip_suffix(".pc"))
            .map(|prefix| format!("{prefix}.ben"))
            .unwrap_or_else(|| format!("{input_file}.ben")),
        Mode::PcToXben => input_file
            .strip_suffix(".pcompress")
            .or_else(|| input_file.strip_suffix(".pc"))
            .map(|prefix| format!("{prefix}.xben"))
            .unwrap_or_else(|| format!("{input_file}.xben")),
    }
}
