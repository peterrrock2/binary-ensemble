use crate::io::bundle::format::AssignmentFormat;
use crate::io::bundle::writer::BendlAppender;
use crate::io::bundle::{AddAssetOptions, BendlWriteError, BendlWriter};
use std::io::{Read, Seek, Write};
use std::path::Path;

/// Detect the container format of `path` from its extension.
pub(super) fn format_from_path(path: &Path) -> Result<AssignmentFormat, String> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("ben") => Ok(AssignmentFormat::Ben),
        Some("xben") => Ok(AssignmentFormat::Xben),
        other => Err(format!(
            "unable to determine assignment format from extension {other:?}; \
             expected .ben or .xben"
        )),
    }
}

/// `mode` argument expected by `count_samples_from_file`.
pub(super) fn mode_str(format: AssignmentFormat) -> &'static str {
    match format {
        AssignmentFormat::Ben => "ben",
        AssignmentFormat::Xben => "xben",
    }
}

pub(super) fn add_file_asset<W: Write + Seek>(
    writer: &mut BendlWriter<W>,
    asset_type: u16,
    name: &str,
    path: &Path,
    options: AddAssetOptions,
) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("failed to read {path:?}: {e}"))?;
    writer
        .add_asset(asset_type, name, &bytes, options)
        .map_err(|e: BendlWriteError| format!("failed to add asset {name:?}: {e}"))
}

pub(super) fn append_file_asset<
    W: Read + Write + Seek + crate::io::bundle::writer::BendlTruncate,
>(
    appender: &mut BendlAppender<W>,
    asset_type: u16,
    name: &str,
    path: &Path,
    options: AddAssetOptions,
) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("failed to read {path:?}: {e}"))?;
    appender
        .add_asset(asset_type, name, &bytes, options)
        .map_err(|e: BendlWriteError| format!("failed to add asset {name:?}: {e}"))
}
