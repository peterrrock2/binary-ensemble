use super::*;

#[test]
fn test_extract_assignment_ben() {
    // [1,1,1,1,2,2,2,2,3,3,3,3,4,4,4,4],
    // [2,2,3,3,3,3,3,3,3,1,2,3]
    // [1,2,3,4,5,6,7,8,9,10]
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
    // [1,1,1,1,2,2,2,2,3,3,3,3,4,4,4,4],
    // [2,2,3,3,3,3,3,3,3,1,2,3]
    // [1,2,3,4,5,6,7,8,9,10]
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
