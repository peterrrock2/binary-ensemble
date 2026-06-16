use std::io;

#[test]
fn format_error_io_passthrough() {
    let inner = io::Error::new(io::ErrorKind::BrokenPipe, "pipe broke");
    let format_err = super::FormatError::Io(inner);
    let io_err: io::Error = format_err.into();
    assert_eq!(io_err.kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(io_err.to_string(), "pipe broke");
}

#[test]
fn format_error_unknown_banner_becomes_invalid_data() {
    let format_err = super::FormatError::UnknownBanner {
        actual: b"GARBAGE BANNER!!!".to_vec(),
    };
    let io_err: io::Error = format_err.into();
    assert_eq!(io_err.kind(), io::ErrorKind::InvalidData);
    assert!(io_err.to_string().contains("unrecognized BEN banner"));
}
