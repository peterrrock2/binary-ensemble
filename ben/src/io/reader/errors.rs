use std::io;

#[derive(Debug)]
/// Errors produced while validating the header of a decoder input stream.
pub enum DecoderInitError {
    /// The leading bytes did not match any supported BEN banner.
    InvalidFileFormat(Vec<u8>),
    /// An I/O error occurred while reading the header.
    Io(io::Error),
}

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

impl std::fmt::Display for DecoderInitError {
    /// Format the decoder initialization error for display.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "IO error: {e}"),
            Self::InvalidFileFormat(header) => {
                if is_xz_header(header) {
                    write!(
                        f,
                        "Invalid file format: Compressed header detected (hex: {}). \
                     This reader expects an uncompressed .ben file. \
                     Decompress this file using the BEN cli `ben -m decode <file_name>.xben` tool \
                     or the `decode_xben_to_ben` function in this library.",
                        to_hex(header)
                    )
                } else {
                    let lossy = String::from_utf8_lossy(header);
                    write!(
                        f,
                        "Invalid file format. Found header (utf8-lossy: {lossy:?}, hex: {})",
                        to_hex(header)
                    )
                }
            }
        }
    }
}

impl std::error::Error for DecoderInitError {
    /// Return the underlying source error when one exists.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DecoderInitError::Io(e) => Some(e),
            DecoderInitError::InvalidFileFormat(_) => None,
        }
    }
}

impl From<io::Error> for DecoderInitError {
    /// Wrap a plain I/O error as a decoder initialization error.
    fn from(error: io::Error) -> Self {
        DecoderInitError::Io(error)
    }
}

impl From<DecoderInitError> for io::Error {
    /// Convert a decoder initialization error into a plain I/O error.
    fn from(error: DecoderInitError) -> Self {
        match error {
            DecoderInitError::Io(e) => e,
            DecoderInitError::InvalidFileFormat(msg) => {
                io::Error::new(io::ErrorKind::InvalidData, format!("{msg:?}"))
            }
        }
    }
}
