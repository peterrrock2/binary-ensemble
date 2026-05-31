use std::io;
use thiserror::Error;

/// Check whether a header prefix matches the XZ file signature.
///
/// # Arguments
///
/// * `h` - The bytes to inspect.
///
/// # Returns
///
/// Returns `true` when `h` begins with the standard XZ magic bytes.
fn is_xz_header(h: &[u8]) -> bool {
    h.len() >= 6 && &h[..6] == b"\xFD\x37\x7A\x58\x5A\x00"
}

/// Convert a byte slice into a space-separated uppercase hex string.
///
/// # Arguments
///
/// * `bytes` - The bytes to render.
///
/// # Returns
///
/// Returns the formatted hex string.
fn to_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{:02X}", b))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Format an `InvalidFileFormat` byte header into a human-readable error message.
fn format_invalid_file_format(header: &[u8]) -> String {
    if is_xz_header(header) {
        format!(
            "Invalid file format: Compressed header detected (hex: {}). \
             This reader expects an uncompressed .ben file. \
             Decompress this file using the BEN cli `ben -m decode <file_name>.xben` tool \
             or the `decode_xben_to_ben` function in this library.",
            to_hex(header)
        )
    } else {
        let lossy = String::from_utf8_lossy(header);
        format!(
            "Invalid file format. Found header (utf8-lossy: {lossy:?}, hex: {})",
            to_hex(header)
        )
    }
}

#[derive(Debug, Error)]
/// Errors produced while validating the header of a decoder input stream.
pub enum DecoderInitError {
    /// The leading bytes did not match any supported BEN banner.
    #[error("{}", format_invalid_file_format(.0))]
    InvalidFileFormat(Vec<u8>),

    /// The file mode string was not recognised.
    #[error("unknown BEN file mode {mode:?}; expected \"ben\" or \"xben\"")]
    UnknownMode { mode: String },

    /// An I/O error occurred while reading the header.
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
}

impl From<DecoderInitError> for io::Error {
    /// Convert a decoder initialization error into a plain I/O error.
    fn from(error: DecoderInitError) -> Self {
        match error {
            DecoderInitError::Io(e) => e,
            DecoderInitError::UnknownMode { .. } => {
                io::Error::new(io::ErrorKind::InvalidInput, error.to_string())
            }
            other => io::Error::new(io::ErrorKind::InvalidData, other.to_string()),
        }
    }
}
