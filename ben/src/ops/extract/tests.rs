use super::*;
use crate::codec::encode::encode_jsonl_to_xben;
use crate::BenVariant;
use serde_json::json;
use std::error::Error as _;
use std::io::BufReader;

#[test]
fn test_extract_assignment_ben() {
    let mut input: Vec<u8> = b"STANDARD BEN FILE".to_vec();
    input.extend(vec![
        3,
        3,
        0,
        0,
        0,
        3,
        0b001100_01,
        0b0100_0111,
        0b00_100100,
        2,
        3,
        0,
        0,
        0,
        4,
        0b10010_111,
        0b11_01001_1,
        0b0001_1100,
        0b1_0000000,
        4,
        1,
        0,
        0,
        0,
        7,
        0b00011_001,
        0b01_00111_0,
        0b1001_0101,
        0b1_01101_01,
        0b111_10001,
        0b10011_101,
        0b01_000000,
    ]);

    let mut reader = input.as_slice();

    assert_eq!(
        extract_assignment_ben(&mut reader, 1).unwrap(),
        vec![1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4]
    );

    let mut reader = input.as_slice();
    assert_eq!(
        extract_assignment_ben(&mut reader, 2).unwrap(),
        vec![2, 2, 3, 3, 3, 3, 3, 3, 3, 1, 2, 3]
    );

    let mut reader = input.as_slice();
    assert_eq!(
        extract_assignment_ben(&mut reader, 3).unwrap(),
        vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
    );
}

#[test]
fn test_extract_assignment_sample_too_large() {
    let mut input: Vec<u8> = b"STANDARD BEN FILE".to_vec();
    input.extend(vec![
        3,
        3,
        0,
        0,
        0,
        3,
        0b001100_01,
        0b0100_0111,
        0b00_100100,
        2,
        3,
        0,
        0,
        0,
        4,
        0b10010_111,
        0b11_01001_1,
        0b0001_1100,
        0b1_0000000,
        4,
        1,
        0,
        0,
        0,
        7,
        0b00011_001,
        0b01_00111_0,
        0b1001_0101,
        0b1_01101_01,
        0b111_10001,
        0b10011_101,
        0b01_000000,
    ]);

    let mut reader = input.as_slice();
    let sample_number = 4;

    let result = extract_assignment_ben(&mut reader, sample_number);

    match result {
        Err(SampleError {
            kind: SampleErrorKind::SampleNotFound { sample_number: 4 },
        }) => (),
        _ => panic!(
            "{}",
            format!("Expected SampleError::SampleNotFound, got {:?}", result)
        ),
    }
}

#[test]
fn test_extract_assignment_ben_rejects_zero_sample_number() {
    let err = extract_assignment_ben([].as_slice(), 0).unwrap_err();
    assert!(matches!(err.kind, SampleErrorKind::InvalidSampleNumber));
    assert_eq!(
        err.to_string(),
        "Invalid sample number. Sample number must be greater than 0"
    );
    assert!(err.source().is_none());
}

#[test]
fn test_extract_assignment_xben_roundtrip_and_errors() {
    let jsonl = [
        json!({"assignment":[1,1,2], "sample": 1}).to_string(),
        json!({"assignment":[3,3,4], "sample": 2}).to_string(),
        json!({"assignment":[3,3,4], "sample": 3}).to_string(),
    ]
    .join("\n")
        + "\n";

    let mut xben = Vec::new();
    encode_jsonl_to_xben(
        BufReader::new(jsonl.as_bytes()),
        &mut xben,
        BenVariant::MkvChain,
        Some(1),
        Some(0),
        None,
    )
    .unwrap();

    let assignment = extract_assignment_xben(xben.as_slice(), 3).unwrap();
    assert_eq!(assignment, vec![3, 3, 4]);

    let missing = extract_assignment_xben(xben.as_slice(), 4).unwrap_err();
    assert!(matches!(
        missing.kind,
        SampleErrorKind::SampleNotFound { sample_number: 4 }
    ));
    assert_eq!(
        missing.to_string(),
        "Sample number not found in file. Failed to find sample '4'. Last sample seems to be '3'"
    );
    assert!(missing.source().is_none());

    let zero = extract_assignment_xben(xben.as_slice(), 0).unwrap_err();
    assert!(matches!(zero.kind, SampleErrorKind::InvalidSampleNumber));
}

#[test]
fn test_sample_error_conversion_and_sources() {
    let io_err = io::Error::other("boom");
    let sample_err = SampleError::from(io_err);
    assert!(matches!(sample_err.kind, SampleErrorKind::IoError(_)));
    assert_eq!(sample_err.to_string(), "IO Error: boom");
    assert!(sample_err.source().is_some());

    let json_err = serde_json::from_str::<serde_json::Value>("{").unwrap_err();
    let sample_err = SampleError::from(json_err);
    assert!(matches!(sample_err.kind, SampleErrorKind::JsonError(_)));
    assert!(sample_err.to_string().starts_with("JSON Error: "));
    assert!(sample_err.source().is_some());
}
