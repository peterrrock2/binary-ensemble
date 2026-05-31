// Binary literals here are grouped by BEN bit-field boundaries (e.g. `0b01100_100` is a 5-bit
// value followed by a 3-bit value), not by even nibbles, so the grouping documents the packed
// layout under test.
#![allow(clippy::unusual_byte_groupings)]

use super::*;
use crate::codec::frames::BenEncodeFrame;
use crate::util::rle::rle_to_vec;
use crate::BenVariant;
use serde_json::json;
use serde_json::Value;
use std::io::{self, BufRead, Write};

#[test]
fn test_encode_jsonl_to_ben_underflow() {
    let rle_vec: Vec<(u16, u16)> = vec![(1, 4), (2, 1), (3, 3)];

    let mut buffer: Vec<u8> = Vec::new();
    let writer = &mut buffer;

    let assign_vec = rle_to_vec(rle_vec);

    let data = json!({
        "assignment": assign_vec,
        "sample": 1,
    });

    let mut expected_output: Vec<u8> = b"STANDARD BEN FILE".to_vec();
    expected_output.extend(vec![
        2,
        3,
        0,
        0,
        0,
        2, // N Bytes
        0b01100_100,
        0b01_11011_0,
    ]);

    let output = encode_jsonl_to_ben(
        json!(data).to_string().as_bytes(),
        writer,
        BenVariant::Standard,
    );
    if let Err(e) = output {
        panic!("Error: {}", e);
    }
    assert_eq!(buffer, expected_output);
}

#[test]
fn test_encode_jsonl_to_ben_exact() {
    let rle_vec: Vec<(u16, u16)> = vec![
        (1, 4),
        (2, 1),
        (3, 3),
        (2, 2),
        (3, 7),
        (1, 1),
        (2, 1),
        (3, 1),
    ];

    let mut buffer: Vec<u8> = Vec::new();
    let writer = &mut buffer;

    let assign_vec = rle_to_vec(rle_vec);

    let data = json!({
        "assignment": assign_vec,
        "sample": 1,
    });

    let mut expected_output: Vec<u8> = b"STANDARD BEN FILE".to_vec();
    expected_output.extend(vec![
        2, // Max Val Bits
        3, // Max Len Bits
        0,
        0,
        0,
        5, // N Bytes
        0b01100_100,
        0b01_11011_1,
        0b0010_1111,
        0b1_01001_10,
        0b001_11001_,
    ]);

    let output = encode_jsonl_to_ben(
        json!(data).to_string().as_bytes(),
        writer,
        BenVariant::Standard,
    );
    if let Err(e) = output {
        panic!("Error: {}", e);
    }
    assert_eq!(buffer, expected_output);
}

#[test]
fn test_encode_jsonl_to_ben_16_bit_val() {
    let rle_vec: Vec<(u16, u16)> = vec![(1, 4), (512, 1), (3, 3)];

    let mut buffer: Vec<u8> = Vec::new();
    let writer = &mut buffer;

    let assign_vec = rle_to_vec(rle_vec);

    let data = json!({
        "assignment": assign_vec,
        "sample": 1,
    });

    let mut expected_output: Vec<u8> = b"STANDARD BEN FILE".to_vec();
    expected_output.extend(vec![
        10, // Max Val Bits
        3,  // Max Len Bits
        0,
        0,
        0,
        5, // N Bytes
        0b00000000,
        0b01100_100,
        0b00000000,
        0b01_000000,
        0b0011011_0,
    ]);

    let output = encode_jsonl_to_ben(
        json!(data).to_string().as_bytes(),
        writer,
        BenVariant::Standard,
    );
    if let Err(e) = output {
        panic!("Error: {}", e);
    }
    assert_eq!(buffer, expected_output);
}

#[test]
fn test_encode_jsonl_to_ben_16_bit_len() {
    let rle_vec: Vec<(u16, u16)> = vec![(1, 4), (2, 512), (3, 3)];

    let mut buffer: Vec<u8> = Vec::new();
    let writer = &mut buffer;

    let assign_vec = rle_to_vec(rle_vec);

    let data = json!({
        "assignment": assign_vec,
        "sample": 1,
    });

    let mut expected_output: Vec<u8> = b"STANDARD BEN FILE".to_vec();
    expected_output.extend(vec![
        2,  // Max Val Bits
        10, // Max Len Bits
        0,
        0,
        0,
        5, // N Bytes
        0b01000000,
        0b0100_1010,
        0b00000000_,
        0b11000000,
        0b0011_0000,
    ]);

    let output = encode_jsonl_to_ben(
        json!(data).to_string().as_bytes(),
        writer,
        BenVariant::Standard,
    );
    if let Err(e) = output {
        panic!("Error: {}", e);
    }
    assert_eq!(buffer, expected_output);
}

#[test]
fn test_encode_jsonl_to_ben_max_val_65535() {
    let rle_vec: Vec<(u16, u16)> = vec![(23, 4), (65535, 15), (8, 3)];

    let mut buffer: Vec<u8> = Vec::new();
    let writer = &mut buffer;

    let assign_vec = rle_to_vec(rle_vec);

    let data = json!({
        "assignment": assign_vec,
        "sample": 1,
    });

    let mut expected_output: Vec<u8> = b"STANDARD BEN FILE".to_vec();
    expected_output.extend(vec![
        16, // Max Val Bits
        4,  // Max Len Bits
        0,
        0,
        0,
        8, // N Bytes
        0b00000000,
        0b00010111,
        0b0100_1111,
        0b11111111,
        0b11111111_,
        0b00000000,
        0b00001000,
        0b0011_0000,
    ]);

    let output = encode_jsonl_to_ben(
        json!(data).to_string().as_bytes(),
        writer,
        BenVariant::Standard,
    );
    if let Err(e) = output {
        panic!("Error: {}", e);
    }
    assert_eq!(buffer, expected_output);
}

#[test]
fn test_encode_jsonl_to_ben_len_65535() {
    let rle_vec: Vec<(u16, u16)> = vec![(23, 4), (60, 65535), (8, 3)];

    let mut buffer: Vec<u8> = Vec::new();
    let writer = &mut buffer;

    let assign_vec = rle_to_vec(rle_vec);

    let data = json!({
        "assignment": assign_vec,
        "sample": 1,
    });

    let mut expected_output: Vec<u8> = b"STANDARD BEN FILE".to_vec();
    expected_output.extend(vec![
        6,  // Max Val Bits
        16, // Max Len Bits
        0,
        0,
        0,
        9, // N Bytes
        0b01011100,
        0b00000000,
        0b000100_11,
        0b11001111,
        0b11111111,
        0b1111_0010,
        0b00000000,
        0b000000000,
        0b11_000000,
    ]);

    let output = encode_jsonl_to_ben(
        json!(data).to_string().as_bytes(),
        writer,
        BenVariant::Standard,
    );
    if let Err(e) = output {
        panic!("Error: {}", e);
    }
    assert_eq!(buffer, expected_output);
}

#[test]
fn test_encode_ben_vec_from_assign_matches_rle_entrypoint() {
    let assign_vec = vec![4u16, 4, 4, 1, 1, 3, 3, 3, 2];
    let direct = BenEncodeFrame::from_assignment(assign_vec.clone(), BenVariant::Standard, None);
    let via_rle = BenEncodeFrame::from_rle(
        crate::util::rle::assign_to_rle(assign_vec),
        BenVariant::Standard,
        None,
    );
    assert_eq!(direct, via_rle);
}

#[test]
fn encode_jsonl_to_ben_max_val_and_len_at_65535() {
    let rle_vec: Vec<(u16, u16)> = vec![(1, 3), (65535, 65535), (8, 4)];

    let mut buffer: Vec<u8> = Vec::new();
    let writer = &mut buffer;

    let assign_vec = rle_to_vec(rle_vec);

    let data = json!({
        "assignment": assign_vec,
        "sample": 1,
    });

    let mut expected_output: Vec<u8> = b"STANDARD BEN FILE".to_vec();
    expected_output.extend(vec![
        16, // Max Val Bits
        16, // Max Len Bits
        0,
        0,
        0,
        12, // N Bytes
        0b00000000,
        0b00000001,
        0b00000000,
        0b00000011_,
        0b11111111,
        0b11111111,
        0b11111111,
        0b11111111_,
        0b00000000,
        0b00001000,
        0b00000000,
        0b00000100_,
    ]);

    let output = encode_jsonl_to_ben(
        json!(data).to_string().as_bytes(),
        writer,
        BenVariant::Standard,
    );
    if let Err(e) = output {
        panic!("Error: {}", e);
    }
    assert_eq!(buffer, expected_output);
}

#[test]
fn encode_jsonl_to_ben_single_element() {
    let rle_vec: Vec<(u16, u16)> = vec![(23, 1)];

    let mut buffer: Vec<u8> = Vec::new();
    let writer = &mut buffer;

    let assign_vec = rle_to_vec(rle_vec);

    let data = json!({
        "assignment": assign_vec,
        "sample": 1,
    });

    let mut expected_output: Vec<u8> = b"STANDARD BEN FILE".to_vec();
    expected_output.extend(vec![
        5, // Max Val Bits
        1, // Max Len Bits
        0,
        0,
        0,
        1, // N Bytes
        0b101111_00,
    ]);

    let output = encode_jsonl_to_ben(
        json!(data).to_string().as_bytes(),
        writer,
        BenVariant::Standard,
    );
    if let Err(e) = output {
        panic!("Error: {}", e);
    }
    assert_eq!(buffer, expected_output);
}

#[test]
fn encode_jsonl_to_ben_single_zero() {
    let rle_vec: Vec<(u16, u16)> = vec![(0, 1)];

    let mut buffer: Vec<u8> = Vec::new();
    let writer = &mut buffer;

    let assign_vec = rle_to_vec(rle_vec);

    let data = json!({
        "assignment": assign_vec,
        "sample": 1,
    });

    let mut expected_output: Vec<u8> = b"STANDARD BEN FILE".to_vec();
    expected_output.extend(vec![
        1, // Max Val Bits
        1, // Max Len Bits
        0,
        0,
        0,
        1, // N Bytes
        0b01_000000,
    ]);

    let output = encode_jsonl_to_ben(
        json!(data).to_string().as_bytes(),
        writer,
        BenVariant::Standard,
    );
    if let Err(e) = output {
        panic!("Error: {}", e);
    }
    assert_eq!(buffer, expected_output);
}

#[test]
fn encode_jsonl_to_ben_multiple_simple_lines() {
    let rle_lst: Vec<Vec<(u16, u16)>> = vec![
        vec![(1, 4), (2, 4), (3, 4), (4, 4)],
        vec![(2, 2), (3, 7), (1, 1), (2, 1), (3, 1)],
        vec![
            (1, 1),
            (2, 1),
            (3, 1),
            (4, 1),
            (5, 1),
            (6, 1),
            (7, 1),
            (8, 1),
            (9, 1),
            (10, 1),
        ],
    ];

    let mut buffer: Vec<u8> = Vec::new();
    let writer = &mut buffer;

    let mut full_data = String::new();

    for (i, rle_vec) in rle_lst.into_iter().enumerate() {
        let assign_vec = rle_to_vec(rle_vec);

        let data = json!({
            "assignment": assign_vec,
            "sample": i+1,
        });

        full_data = full_data + &json!(data).to_string() + "\n";
    }

    let mut expected_output: Vec<u8> = b"STANDARD BEN FILE".to_vec();
    expected_output.extend(vec![
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

    let output = encode_jsonl_to_ben(full_data.as_bytes(), writer, BenVariant::Standard);
    if let Err(e) = output {
        panic!("Error {}", e);
    }
    assert_eq!(buffer, expected_output)
}

fn encode_jsonl_to_ben32<R: BufRead, W: Write>(reader: R, mut writer: W) -> std::io::Result<()> {
    let mut line_num = 1;

    writer.write_all("STANDARD BEN FILE".as_bytes())?;
    for line_result in reader.lines() {
        eprint!("Encoding line: {}\r", line_num);
        line_num += 1;
        let line = line_result?;
        let data: Value = serde_json::from_str(&line).expect("Error parsing JSON from line");

        writer.write_all(&encode_ben32_line(data)?)?;
    }
    eprintln!("Done!");
    Ok(())
}

#[test]
fn test_encode_jsonl_to_ben32_simple() {
    let rle_vec: Vec<(u16, u16)> = vec![(1, 4), (2, 1), (3, 3)];

    let mut buffer: Vec<u8> = Vec::new();
    let writer = &mut buffer;

    let assign_vec = rle_to_vec(rle_vec);

    let data = json!({
        "assignment": assign_vec,
        "sample": 1,
    });

    let mut expected_output: Vec<u8> = b"STANDARD BEN FILE".to_vec();
    expected_output.extend(vec![0, 1, 0, 4, 0, 2, 0, 1, 0, 3, 0, 3, 0, 0, 0, 0]);

    let output = encode_jsonl_to_ben32(json!(data).to_string().as_bytes(), writer);
    if let Err(e) = output {
        panic!("Error: {}", e);
    }
    assert_eq!(buffer, expected_output);
}

#[test]
fn test_encode_jsonl_to_ben32_16_bit_val() {
    let rle_vec: Vec<(u16, u16)> = vec![(1, 4), (512, 1), (3, 3)];

    let mut buffer: Vec<u8> = Vec::new();
    let writer = &mut buffer;

    let assign_vec = rle_to_vec(rle_vec);

    let data = json!({
        "assignment": assign_vec,
        "sample": 1,
    });

    let mut expected_output: Vec<u8> = b"STANDARD BEN FILE".to_vec();
    expected_output.extend(vec![0, 1, 0, 4, 2, 0, 0, 1, 0, 3, 0, 3, 0, 0, 0, 0]);

    let output = encode_jsonl_to_ben32(json!(data).to_string().as_bytes(), writer);
    if let Err(e) = output {
        panic!("Error: {}", e);
    }
    assert_eq!(buffer, expected_output);
}

#[test]
fn test_encode_jsonl_to_ben32_16_bit_len() {
    let rle_vec: Vec<(u16, u16)> = vec![(1, 4), (2, 512), (3, 3)];

    let mut buffer: Vec<u8> = Vec::new();
    let writer = &mut buffer;

    let assign_vec = rle_to_vec(rle_vec);

    let data = json!({
        "assignment": assign_vec,
        "sample": 1,
    });

    let mut expected_output = b"STANDARD BEN FILE".to_vec();
    expected_output.extend(vec![0, 1, 0, 4, 0, 2, 2, 0, 0, 3, 0, 3, 0, 0, 0, 0]);

    let output = encode_jsonl_to_ben32(json!(data).to_string().as_bytes(), writer);
    if let Err(e) = output {
        panic!("Error: {}", e);
    }
    assert_eq!(buffer, expected_output);
}

#[test]
fn test_encode_jsonl_to_ben32_max_val_65535() {
    let rle_vec: Vec<(u16, u16)> = vec![(23, 4), (65535, 15), (8, 3)];

    let mut buffer: Vec<u8> = Vec::new();
    let writer = &mut buffer;

    let assign_vec = rle_to_vec(rle_vec);

    let data = json!({
        "assignment": assign_vec,
        "sample": 1,
    });

    let mut expected_output = b"STANDARD BEN FILE".to_vec();
    expected_output.extend(vec![0, 23, 0, 4, 255, 255, 0, 15, 0, 8, 0, 3, 0, 0, 0, 0]);

    let output = encode_jsonl_to_ben32(json!(data).to_string().as_bytes(), writer);
    if let Err(e) = output {
        panic!("Error: {}", e);
    }
    assert_eq!(buffer, expected_output);
}

#[test]
fn test_encode_jsonl_to_ben32_len_65535() {
    let rle_vec: Vec<(u16, u16)> = vec![(23, 4), (60, 65535), (8, 3)];

    let mut buffer: Vec<u8> = Vec::new();
    let writer = &mut buffer;

    let assign_vec = rle_to_vec(rle_vec);

    let data = json!({
        "assignment": assign_vec,
        "sample": 1,
    });

    let mut expected_output = b"STANDARD BEN FILE".to_vec();
    expected_output.extend(vec![0, 23, 0, 4, 0, 60, 255, 255, 0, 8, 0, 3, 0, 0, 0, 0]);

    let output = encode_jsonl_to_ben32(json!(data).to_string().as_bytes(), writer);
    if let Err(e) = output {
        panic!("Error: {}", e);
    }
    assert_eq!(buffer, expected_output);
}

#[test]
fn encode_jsonl_to_ben32_single_element() {
    let rle_vec: Vec<(u16, u16)> = vec![(23, 1)];

    let mut buffer: Vec<u8> = Vec::new();
    let writer = &mut buffer;

    let assign_vec = rle_to_vec(rle_vec);

    let data = json!({
        "assignment": assign_vec,
        "sample": 1,
    });

    let mut expected_output = b"STANDARD BEN FILE".to_vec();
    expected_output.extend(vec![0, 23, 0, 1, 0, 0, 0, 0]);

    let output = encode_jsonl_to_ben32(json!(data).to_string().as_bytes(), writer);
    if let Err(e) = output {
        panic!("Error: {}", e);
    }
    assert_eq!(buffer, expected_output);
}

#[test]
fn encode_jsonl_to_ben32_multiple_simple_lines() {
    let rle_lst: Vec<Vec<(u16, u16)>> = vec![
        vec![(1, 4), (2, 4), (3, 4), (4, 4)],
        vec![(2, 2), (3, 7), (1, 1), (2, 1), (3, 1)],
        vec![
            (1, 1),
            (2, 1),
            (3, 1),
            (4, 1),
            (5, 1),
            (6, 1),
            (7, 1),
            (8, 1),
            (9, 1),
            (10, 1),
        ],
    ];

    let mut buffer: Vec<u8> = Vec::new();
    let writer = &mut buffer;

    let mut full_data = String::new();

    for (i, rle_vec) in rle_lst.into_iter().enumerate() {
        let assign_vec = rle_to_vec(rle_vec);

        let data = json!({
            "assignment": assign_vec,
            "sample": i+1,
        });

        full_data = full_data + &json!(data).to_string() + "\n";
    }

    let mut expected_output = b"STANDARD BEN FILE".to_vec();
    expected_output.extend(vec![
        0, 1, 0, 4, 0, 2, 0, 4, 0, 3, 0, 4, 0, 4, 0, 4, 0, 0, 0, 0, 0, 2, 0, 2, 0, 3, 0, 7, 0, 1,
        0, 1, 0, 2, 0, 1, 0, 3, 0, 1, 0, 0, 0, 0, 0, 1, 0, 1, 0, 2, 0, 1, 0, 3, 0, 1, 0, 4, 0, 1,
        0, 5, 0, 1, 0, 6, 0, 1, 0, 7, 0, 1, 0, 8, 0, 1, 0, 9, 0, 1, 0, 10, 0, 1, 0, 0, 0, 0,
    ]);

    let output = encode_jsonl_to_ben32(full_data.as_bytes(), writer);
    if let Err(e) = output {
        panic!("Error {}", e);
    }
    assert_eq!(buffer, expected_output)
}

#[test]
fn encode_ben32_line_missing_assignment_field() {
    let data = json!({"sample": 1});
    let err = encode_ben32_line(data).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert!(err.to_string().contains("assignment"));
}

#[test]
fn encode_ben32_line_non_integer_value() {
    let data = json!({"assignment": ["not_a_number"]});
    let err = encode_ben32_line(data).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn encode_ben32_line_value_too_large_for_u16() {
    let data = json!({"assignment": [100000]});
    let err = encode_ben32_line(data).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert!(err.to_string().contains("too large"));
}

#[test]
fn encode_ben32_assignments_empty_vec() {
    let result = encode_ben32_assignments(Vec::<u16>::new()).unwrap();
    // Empty vec produces only the terminator
    assert_eq!(result, vec![0, 0, 0, 0]);
}

#[test]
fn encode_ben32_assignments_single_element() {
    let result = encode_ben32_assignments(vec![5u16]).unwrap();
    // (5 << 16) | 1 = 0x00050001, then terminator
    assert_eq!(result, vec![0, 5, 0, 1, 0, 0, 0, 0]);
}

#[test]
fn encode_jsonl_to_ben_invalid_json_errors() {
    let input = b"not valid json\n";
    let mut output = Vec::new();
    let err = encode_jsonl_to_ben(input.as_slice(), &mut output, BenVariant::Standard).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn encode_jsonl_to_xben_roundtrip() {
    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[2,2,1,1],"sample":2}
"#;
    let mut xben = Vec::new();
    encode_jsonl_to_xben(
        jsonl.as_bytes(),
        &mut xben,
        BenVariant::Standard,
        Some(1),
        Some(1),
        None,
        None,
    )
    .unwrap();
    assert!(!xben.is_empty());
}

#[test]
fn encode_jsonl_to_xben_with_chunk_size() {
    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[2,2,1,1],"sample":2}
"#;
    let mut xben = Vec::new();
    encode_jsonl_to_xben(
        jsonl.as_bytes(),
        &mut xben,
        BenVariant::Standard,
        Some(1),
        Some(1),
        Some(2),
        None,
    )
    .unwrap();
    assert!(!xben.is_empty());
}

#[test]
fn encode_jsonl_to_xben_invalid_json_errors() {
    let input = b"not valid json\n";
    let mut output = Vec::new();
    let err = encode_jsonl_to_xben(
        input.as_slice(),
        &mut output,
        BenVariant::Standard,
        Some(1),
        Some(1),
        None,
        None,
    )
    .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn encode_jsonl_to_xben_mkv_variant() {
    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[1,1,2,2],"sample":2}
{"assignment":[2,2,1,1],"sample":3}
"#;
    let mut xben = Vec::new();
    encode_jsonl_to_xben(
        jsonl.as_bytes(),
        &mut xben,
        BenVariant::MkvChain,
        Some(1),
        Some(1),
        None,
        None,
    )
    .unwrap();
    assert!(!xben.is_empty());
}

#[test]
fn twodelta_encode_with_pair_and_mask_hints() {
    use crate::codec::encode::encode_twodelta_frame_with_hint;
    use std::collections::HashMap;

    let prev = vec![1u16, 1, 2, 2];
    let curr = vec![2u16, 1, 2, 1];
    let mut masks: HashMap<u16, Vec<usize>> = HashMap::new();
    masks.insert(1, vec![0, 1]);
    masks.insert(2, vec![2, 3]);

    let frame = encode_twodelta_frame_with_hint(&prev, &curr, Some((1, 2)), Some(&mut masks), None)
        .unwrap();
    assert_eq!(frame.pair().unwrap(), (2, 1));
    assert!(!frame.run_length_vector().unwrap().is_empty());
    // Verify masks were updated
    assert_eq!(masks[&2], vec![0, 2]);
    assert_eq!(masks[&1], vec![1, 3]);
}

#[test]
fn twodelta_encode_with_mask_hint_only() {
    use crate::codec::encode::encode_twodelta_frame_with_hint;
    use std::collections::HashMap;

    let prev = vec![1u16, 1, 2, 2];
    let curr = vec![2u16, 1, 2, 1];
    let mut masks: HashMap<u16, Vec<usize>> = HashMap::new();
    masks.insert(1, vec![0, 1]);
    masks.insert(2, vec![2, 3]);

    let frame =
        encode_twodelta_frame_with_hint(&prev, &curr, None, Some(&mut masks), None).unwrap();
    assert_eq!(frame.pair().unwrap(), (2, 1));
}

#[test]
fn twodelta_encode_length_mismatch() {
    use crate::codec::encode::encode_twodelta_frame_with_hint;

    let prev = vec![1u16, 1, 2];
    let curr = vec![2u16, 1, 2, 1];
    let err = encode_twodelta_frame_with_hint(&prev, &curr, None, None, None).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn twodelta_encode_hint_without_masks_errors() {
    use crate::codec::encode::encode_twodelta_frame_with_hint;

    let prev = vec![1u16, 1, 2, 2];
    let curr = vec![2u16, 1, 2, 1];
    let err = encode_twodelta_frame_with_hint(&prev, &curr, Some((1, 2)), None, None).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn twodelta_encode_identical_pair_hint_errors() {
    use crate::codec::encode::encode_twodelta_frame_with_hint;
    use std::collections::HashMap;

    let prev = vec![1u16, 1, 2, 2];
    let curr = vec![2u16, 1, 2, 1];
    let mut masks = HashMap::new();
    masks.insert(1u16, vec![0, 1]);

    let err = encode_twodelta_frame_with_hint(&prev, &curr, Some((1, 1)), Some(&mut masks), None)
        .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn twodelta_encode_identical_assignments_errors() {
    use crate::codec::encode::encode_twodelta_frame;

    let a = vec![1u16, 1, 2, 2];
    let err = encode_twodelta_frame(&a, &a, None).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn twodelta_encode_too_many_ids_errors() {
    use crate::codec::encode::encode_twodelta_frame;

    let prev = vec![1u16, 2, 3, 4];
    let curr = vec![2u16, 1, 4, 3]; // 4 ids changing
    let err = encode_twodelta_frame(&prev, &curr, None).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn twodelta_encode_mask_hint_identical_errors() {
    use crate::codec::encode::encode_twodelta_frame_with_hint;
    use std::collections::HashMap;

    let a = vec![1u16, 1, 2, 2];
    let mut masks: HashMap<u16, Vec<usize>> = HashMap::new();
    masks.insert(1, vec![0, 1]);
    masks.insert(2, vec![2, 3]);

    let err = encode_twodelta_frame_with_hint(&a, &a, None, Some(&mut masks), None).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn encode_error_io_passthrough() {
    let inner = io::Error::new(io::ErrorKind::BrokenPipe, "pipe broke");
    let encode_err = super::errors::EncodeError::Io(inner);
    let io_err: io::Error = encode_err.into();
    assert_eq!(io_err.kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(io_err.to_string(), "pipe broke");
}

#[test]
fn encode_error_non_io_becomes_invalid_data() {
    let encode_err = super::errors::EncodeError::TwoDeltaTooManyIds;
    let io_err: io::Error = encode_err.into();
    assert_eq!(io_err.kind(), io::ErrorKind::InvalidData);
    assert!(io_err.to_string().contains("two distinct"));
}

// ── XBEN roundtrip with content verification ────────────────────────────────

#[test]
fn encode_jsonl_to_xben_roundtrip_verifies_content() {
    use crate::codec::decode::decode_xben_to_jsonl;
    use serde_json::Value;
    use std::io::BufReader;

    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[2,2,1,1],"sample":2}
"#;
    let mut xben = Vec::new();
    encode_jsonl_to_xben(
        jsonl.as_bytes(),
        &mut xben,
        BenVariant::Standard,
        Some(1),
        Some(1),
        None,
        None,
    )
    .unwrap();

    let mut decoded = Vec::new();
    decode_xben_to_jsonl(BufReader::new(xben.as_slice()), &mut decoded).unwrap();
    let output_str = String::from_utf8(decoded).unwrap();
    let lines: Vec<&str> = output_str.trim().split('\n').collect();
    assert_eq!(lines.len(), 2);
    let v1: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(v1["assignment"], serde_json::json!([1, 1, 2, 2]));
    let v2: Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(v2["assignment"], serde_json::json!([2, 2, 1, 1]));
}

#[test]
fn encode_jsonl_to_xben_mkv_verifies_content() {
    use crate::codec::decode::decode_xben_to_jsonl;
    use serde_json::Value;
    use std::io::BufReader;

    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[1,1,2,2],"sample":2}
{"assignment":[2,2,1,1],"sample":3}
"#;
    let mut xben = Vec::new();
    encode_jsonl_to_xben(
        jsonl.as_bytes(),
        &mut xben,
        BenVariant::MkvChain,
        Some(1),
        Some(1),
        None,
        None,
    )
    .unwrap();

    let mut decoded = Vec::new();
    decode_xben_to_jsonl(BufReader::new(xben.as_slice()), &mut decoded).unwrap();
    let output_str = String::from_utf8(decoded).unwrap();
    let lines: Vec<&str> = output_str.trim().split('\n').collect();
    assert_eq!(lines.len(), 3);
    // First two should be identical (MkvChain de-duplication)
    let v1: Value = serde_json::from_str(lines[0]).unwrap();
    let v2: Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(v1["assignment"], serde_json::json!([1, 1, 2, 2]));
    assert_eq!(v2["assignment"], serde_json::json!([1, 1, 2, 2]));
    let v3: Value = serde_json::from_str(lines[2]).unwrap();
    assert_eq!(v3["assignment"], serde_json::json!([2, 2, 1, 1]));
}

// ── TwoDelta with explicit count parameter ──────────────────────────────────

#[test]
fn twodelta_encode_with_count() {
    use crate::codec::encode::encode_twodelta_frame;
    let prev = vec![1u16, 1, 2, 2];
    let next = vec![2u16, 1, 2, 1];
    let frame = encode_twodelta_frame(&prev, &next, Some(5)).unwrap();
    // Verify the count is embedded in the serialized frame's tail
    let raw = frame.as_slice();
    let count = u16::from_be_bytes([raw[raw.len() - 2], raw[raw.len() - 1]]);
    assert_eq!(count, 5);
}

// ── TwoDelta run_length_vector verification ─────────────────────────────────

#[test]
fn twodelta_encode_run_lengths_correct() {
    use crate::codec::encode::encode_twodelta_frame;
    // prev: [1,1,2,2], next: [2,1,2,1] pair positions (1 or 2): 0,1,2,3 In next: pos0=2, pos1=1,
    // pos2=2, pos3=1 → runs of (2,1,2,1) = [1,1,1,1] pair.0 = value at first pair position in next
    // = 2
    let prev = vec![1u16, 1, 2, 2];
    let next = vec![2u16, 1, 2, 1];
    let frame = encode_twodelta_frame(&prev, &next, None).unwrap();
    assert_eq!(frame.pair().unwrap(), (2, 1));
    assert_eq!(frame.run_length_vector().unwrap(), vec![1, 1, 1, 1]);
}

#[test]
fn twodelta_encode_run_lengths_with_non_pair_gaps() {
    use crate::codec::encode::encode_twodelta_frame;
    // prev: [1,3,2,3,1], next: [2,3,1,3,2] pair=(1,2), pair positions: 0,2,4 (positions with value
    // 1 or 2) In next: pos0=2, pos2=1, pos4=2 → runs [1,1,1]
    let prev = vec![1u16, 3, 2, 3, 1];
    let next = vec![2u16, 3, 1, 3, 2];
    let frame = encode_twodelta_frame(&prev, &next, None).unwrap();
    assert_eq!(frame.run_length_vector().unwrap(), vec![1, 1, 1]);
}

// ── TwoDelta encode→decode roundtrip ────────────────────────────────────────

#[test]
fn twodelta_encode_decode_roundtrip_via_codec() {
    use crate::codec::decode::decode_twodelta_frame;
    use crate::codec::encode::encode_twodelta_frame;

    let prev = vec![1u16, 1, 2, 2, 1, 2, 1, 2];
    let next = vec![2u16, 2, 1, 1, 1, 2, 1, 2]; // first 4 positions swap
    let frame = encode_twodelta_frame(&prev, &next, None).unwrap();
    let decoded = decode_twodelta_frame(prev, &frame).unwrap();
    assert_eq!(decoded, next);
}

// ── TwoDelta error variants ─────────────────────────────────────────────────

#[test]
fn twodelta_encode_missing_mask_errors() {
    use crate::codec::encode::encode_twodelta_frame_with_hint;
    use std::collections::HashMap;

    let prev = vec![1u16, 1, 2, 2];
    let curr = vec![2u16, 1, 2, 1];
    let mut masks: HashMap<u16, Vec<usize>> = HashMap::new();
    masks.insert(1, vec![0, 1]);
    // Missing mask for value 2

    let err = encode_twodelta_frame_with_hint(&prev, &curr, Some((1, 2)), Some(&mut masks), None)
        .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn twodelta_encode_empty_mask_errors() {
    use crate::codec::encode::encode_twodelta_frame_with_hint;
    use std::collections::HashMap;

    let prev = vec![1u16, 1, 2, 2];
    let curr = vec![2u16, 1, 2, 1];
    let mut masks: HashMap<u16, Vec<usize>> = HashMap::new();
    masks.insert(1, vec![0, 1]);
    masks.insert(2, vec![]); // Empty mask

    let err = encode_twodelta_frame_with_hint(&prev, &curr, Some((1, 2)), Some(&mut masks), None)
        .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn twodelta_encode_mask_out_of_pair_errors() {
    use crate::codec::encode::encode_twodelta_frame_with_hint;
    use std::collections::HashMap;

    // prev has value 3 at position 2, but mask claims it's part of pair (1,2)
    let prev = vec![1u16, 1, 3, 2];
    let curr = vec![2u16, 1, 3, 1];
    let mut masks: HashMap<u16, Vec<usize>> = HashMap::new();
    masks.insert(1, vec![0, 1]);
    masks.insert(2, vec![2, 3]); // position 2 in prev is actually 3, not 2

    let err = encode_twodelta_frame_with_hint(&prev, &curr, Some((1, 2)), Some(&mut masks), None)
        .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

// ── JSON encoding edge cases ────────────────────────────────────────────────

#[test]
fn encode_ben32_line_negative_value_errors() {
    let data = serde_json::json!({"assignment": [-1, 2, 3]});
    let err = encode_ben32_line(data).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn encode_ben32_line_float_value_errors() {
    let data = serde_json::json!({"assignment": [1.5, 2, 3]});
    let err = encode_ben32_line(data).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn encode_ben32_line_null_value_errors() {
    let data = serde_json::json!({"assignment": [null, 2, 3]});
    let err = encode_ben32_line(data).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn encode_ben32_line_value_at_u16_max() {
    let data = serde_json::json!({"assignment": [65535, 1]});
    let result = encode_ben32_line(data).unwrap();
    // (65535 << 16) | 1 → 0xFFFF0001 then (1 << 16) | 1 → 0x00010001 then terminator
    assert_eq!(result, vec![0xFF, 0xFF, 0, 1, 0, 1, 0, 1, 0, 0, 0, 0]);
}

// ── Encoding empty and single-element JSONL ─────────────────────────────────

#[test]
fn encode_jsonl_to_ben_empty_input() {
    let jsonl = b"";
    let mut output = Vec::new();
    encode_jsonl_to_ben(jsonl.as_slice(), &mut output, BenVariant::Standard).unwrap();
    // Should only have the banner
    assert_eq!(output, b"STANDARD BEN FILE");
}

#[test]
fn encode_jsonl_to_ben_single_sample() {
    use crate::codec::decode::decode_ben_to_jsonl;
    use serde_json::Value;

    let jsonl = b"{\"assignment\":[42],\"sample\":1}\n";
    let mut ben = Vec::new();
    encode_jsonl_to_ben(jsonl.as_slice(), &mut ben, BenVariant::Standard).unwrap();

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(ben.as_slice(), &mut decoded).unwrap();
    let v: Value = serde_json::from_slice(decoded.trim_ascii()).unwrap();
    assert_eq!(v["assignment"], serde_json::json!([42]));
}

// ── TwoDelta JSONL encoding edge cases ──────────────────────────────────────

#[test]
fn encode_jsonl_to_xben_twodelta_roundtrip() {
    use crate::codec::decode::decode_xben_to_jsonl;
    use serde_json::Value;
    use std::io::BufReader;

    let jsonl = r#"{"assignment":[1,1,2,2],"sample":1}
{"assignment":[2,1,2,1],"sample":2}
{"assignment":[2,2,1,1],"sample":3}
"#;
    let mut xben = Vec::new();
    encode_jsonl_to_xben(
        jsonl.as_bytes(),
        &mut xben,
        BenVariant::TwoDelta,
        Some(1),
        Some(1),
        None,
        None,
    )
    .unwrap();

    let mut decoded = Vec::new();
    decode_xben_to_jsonl(BufReader::new(xben.as_slice()), &mut decoded).unwrap();
    let output_str = String::from_utf8(decoded).unwrap();
    let lines: Vec<&str> = output_str.trim().split('\n').collect();
    assert_eq!(lines.len(), 3);
    let v1: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(v1["assignment"], serde_json::json!([1, 1, 2, 2]));
    let v2: Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(v2["assignment"], serde_json::json!([2, 1, 2, 1]));
    let v3: Value = serde_json::from_str(lines[2]).unwrap();
    assert_eq!(v3["assignment"], serde_json::json!([2, 2, 1, 1]));
}

#[test]
fn twodelta_encode_outside_pair_change_errors() {
    use super::twodelta::encode_twodelta_frame;

    // prev=[1,2,3,4], curr=[2,1,3,5] — positions 0,1 swap pair (1,2), but position 3 changes from
    // 4→5 which is outside the pair.
    let prev = vec![1u16, 2, 3, 4];
    let curr = vec![2u16, 1, 3, 5];
    let err = encode_twodelta_frame(&prev, &curr, None).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn twodelta_encode_missing_mask_for_pair0_errors() {
    use crate::codec::encode::encode_twodelta_frame_with_hint;
    use std::collections::HashMap;

    // Only the mask for pair.1 (2) is provided; pair.0 (1) is absent.
    let prev = vec![1u16, 1, 2, 2];
    let curr = vec![2u16, 1, 2, 1];
    let mut masks: HashMap<u16, Vec<usize>> = HashMap::new();
    masks.insert(2, vec![2, 3]);

    let err = encode_twodelta_frame_with_hint(&prev, &curr, Some((1, 2)), Some(&mut masks), None)
        .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn twodelta_encode_empty_mask_for_pair0_errors() {
    use crate::codec::encode::encode_twodelta_frame_with_hint;
    use std::collections::HashMap;

    // pair.0 (1) has an empty mask; pair.1 (2) is non-empty.
    let prev = vec![1u16, 1, 2, 2];
    let curr = vec![2u16, 1, 2, 1];
    let mut masks: HashMap<u16, Vec<usize>> = HashMap::new();
    masks.insert(1, vec![]);
    masks.insert(2, vec![2, 3]);

    let err = encode_twodelta_frame_with_hint(&prev, &curr, Some((1, 2)), Some(&mut masks), None)
        .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn twodelta_encode_new_val_out_of_pair_errors() {
    use crate::codec::encode::encode_twodelta_frame_with_hint;
    use std::collections::HashMap;

    // At position 2: prev=1 (in pair), but curr=3 (outside pair {1,2}).
    let prev = vec![1u16, 2, 1, 2];
    let curr = vec![2u16, 1, 3, 2];
    let mut masks: HashMap<u16, Vec<usize>> = HashMap::new();
    masks.insert(1, vec![0, 2]);
    masks.insert(2, vec![1, 3]);

    let err = encode_twodelta_frame_with_hint(&prev, &curr, Some((1, 2)), Some(&mut masks), None)
        .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn twodelta_encode_pair_mask_hint_identical_errors() {
    use crate::codec::encode::encode_twodelta_frame_with_hint;
    use std::collections::HashMap;

    // Explicit pair + mask hints, but prev == curr → TwoDeltaIdentical.
    let a = vec![1u16, 1, 2, 2];
    let mut masks: HashMap<u16, Vec<usize>> = HashMap::new();
    masks.insert(1, vec![0, 1]);
    masks.insert(2, vec![2, 3]);

    let err =
        encode_twodelta_frame_with_hint(&a, &a, Some((1, 2)), Some(&mut masks), None).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn twodelta_encode_pair_position_changes_to_third_id_errors() {
    use crate::codec::encode::encode_twodelta_frame;

    // At position 2: prev=1 ∈ {1,2}, but curr=3 ∉ {1,2} → TwoDeltaTooManyIds.
    let prev = vec![1u16, 2, 1, 2];
    let curr = vec![2u16, 1, 3, 2];
    let err = encode_twodelta_frame(&prev, &curr, None).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn twodelta_encode_pair_mask_run_exceeds_u16_max_errors() {
    use crate::codec::encode::encode_twodelta_frame_with_hint;
    use std::collections::HashMap;

    // 65538 positions: pair positions 0..65537 hold value 1 in prev, one more (65537) holds value
    // 2. In curr all pair positions hold value 2, so the run of value-2 positions reaches u16::MAX
    // and the encoder must error.
    let mut prev = vec![1u16; 65538];
    prev[65537] = 2;
    let mut curr = vec![2u16; 65538];
    curr[65537] = 1;

    let mut masks: HashMap<u16, Vec<usize>> = HashMap::new();
    masks.insert(1, (0..65537_usize).collect());
    masks.insert(2, vec![65537_usize]);

    let err = encode_twodelta_frame_with_hint(&prev, &curr, Some((1, 2)), Some(&mut masks), None)
        .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    assert!(err.to_string().contains("u16::MAX"));
}

#[test]
fn twodelta_encode_from_scratch_run_exceeds_u16_max_errors() {
    use crate::codec::encode::encode_twodelta_frame;

    // 65538 positions all in pair {1, 2}: first 65537 change 1→2, last 1 changes 2→1. The
    // from-scratch encoder hits u16::MAX consecutive positions with value 2 and errors.
    let mut prev = vec![1u16; 65538];
    prev[65537] = 2;
    let mut curr = vec![2u16; 65538];
    curr[65537] = 1;

    let err = encode_twodelta_frame(&prev, &curr, None).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    assert!(err.to_string().contains("u16::MAX"));
}

#[test]
fn ben32_encode_run_exceeding_u16_max_splits_correctly() {
    use super::ben::encode_ben32_assignments;

    // Build an assignment with 65537 identical values: the run reaches u16::MAX (65535) and must be
    // flushed early, then continues with a new run. encode_ben32_assignments appends a 4-byte zero
    // sentinel at the end.
    let assign: Vec<u16> = vec![7u16; 65537];
    let encoded = encode_ben32_assignments(&assign).unwrap();

    // Should be: (7, 65535) + (7, 2) + [0,0,0,0] sentinel = 12 bytes.
    assert_eq!(encoded.len(), 12);
    let first = u32::from_be_bytes(encoded[0..4].try_into().unwrap());
    let second = u32::from_be_bytes(encoded[4..8].try_into().unwrap());
    let sentinel = u32::from_be_bytes(encoded[8..12].try_into().unwrap());
    assert_eq!(first >> 16, 7u32);
    assert_eq!(first & 0xFFFF, 65535u32); // count = u16::MAX
    assert_eq!(second >> 16, 7u32);
    assert_eq!(second & 0xFFFF, 2u32); // remaining 2 elements
    assert_eq!(sentinel, 0u32); // always-present zero sentinel
}

// =====================================================================
// Label-value 0 round-trips for MkvChain and TwoDelta
// =====================================================================

/// MkvChain round-trip with label `0` in the assignment. The existing
/// `encode_jsonl_to_ben_single_zero` test covers Standard; MkvChain is structurally similar but
/// the count-byte plumbing and run-length code paths are distinct enough to warrant their own
/// pin.
#[test]
fn mkvchain_round_trip_with_label_zero() {
    use crate::io::reader::BenStreamReader;
    use crate::io::writer::BenStreamWriter;
    use std::io::Cursor;

    let assignments = vec![vec![0u16, 0, 1], vec![0u16, 1, 1], vec![0u16, 0, 0]];
    let mut ben = Vec::new();
    {
        let mut writer = BenStreamWriter::for_ben(&mut ben, BenVariant::MkvChain).unwrap();
        for a in &assignments {
            writer.write_assignment(a.clone()).unwrap();
        }
        writer.finish().unwrap();
    }
    let decoded: Vec<Vec<u16>> = BenStreamReader::from_ben(Cursor::new(ben))
        .unwrap()
        .silent(true)
        .flat_map(|r| {
            let (a, c) = r.unwrap();
            std::iter::repeat_n(a, c as usize)
        })
        .collect();
    assert_eq!(decoded, assignments);
}

/// TwoDelta round-trips with delta-frame pairs that contain `0`. The pair `(first, second)` is
/// computed by `twodelta_repeat_runs` from the first distinct values it sees; these fixtures
/// force both orderings — `(0, 1)` and `(1, 0)` — to confirm the bit-packing and unpacking
/// handle a zero-valued label on either side of the pair. Each fixture pairs an anchor with at
/// least one delta so the delta-frame path is exercised, not just the anchor.
///
/// The degenerate `(0, 0)` case is not testable through this writer path: `twodelta_repeat_runs`
/// guarantees `second != first` via a `first + 1` fallback when only one distinct value is
/// present, so an assignment of `[0, 0, 0, 0]` yields the pair `(0, 1)` rather than `(0, 0)`.
/// The all-identical case is covered by `writer_twodelta_all_identical_values` in
/// `io/writer/tests.rs` (using `vec![3u16; 8]`); the only zero-specific aspect missing there is
/// `vec![0u16; 8]`, which would exercise the same code path with the value field cleared.
#[test]
fn twodelta_round_trip_with_label_zero_pairs() {
    use crate::io::reader::BenStreamReader;
    use crate::io::writer::BenStreamWriter;
    use std::io::Cursor;

    let fixtures = vec![
        (
            "pair (0, 1)",
            vec![vec![0u16, 0, 1, 1], vec![0u16, 1, 0, 1]],
        ),
        (
            "pair (1, 0)",
            vec![vec![1u16, 1, 0, 0], vec![1u16, 0, 1, 0]],
        ),
    ];

    for (label, assignments) in fixtures {
        let mut ben = Vec::new();
        {
            let mut writer = BenStreamWriter::for_ben(&mut ben, BenVariant::TwoDelta).unwrap();
            for a in &assignments {
                writer.write_assignment(a.clone()).unwrap();
            }
            writer.finish().unwrap();
        }
        let decoded: Vec<Vec<u16>> = BenStreamReader::from_ben(Cursor::new(ben))
            .unwrap()
            .silent(true)
            .flat_map(|r| {
                let (a, c) = r.unwrap();
                std::iter::repeat_n(a, c as usize)
            })
            .collect();
        assert_eq!(
            decoded, assignments,
            "TwoDelta round-trip failed for fixture: {label}"
        );
    }
}

/// All-zero assignment in TwoDelta: exercises the repeat/anchor-only path on the value `0`. The
/// existing `writer_twodelta_all_identical_values` test in `io/writer/tests.rs` already covers
/// this shape with value `3`; this companion confirms value `0` survives the same path,
/// guarding against any future code path that uses `0` as a sentinel.
#[test]
fn twodelta_round_trip_all_zero_assignment() {
    use crate::io::reader::BenStreamReader;
    use crate::io::writer::BenStreamWriter;
    use std::io::Cursor;

    let assign = vec![0u16; 4];
    let assignments: Vec<Vec<u16>> = (0..5).map(|_| assign.clone()).collect();

    let mut ben = Vec::new();
    {
        let mut writer = BenStreamWriter::for_ben(&mut ben, BenVariant::TwoDelta).unwrap();
        for a in &assignments {
            writer.write_assignment(a.clone()).unwrap();
        }
        writer.finish().unwrap();
    }
    let decoded: Vec<Vec<u16>> = BenStreamReader::from_ben(Cursor::new(ben))
        .unwrap()
        .silent(true)
        .flat_map(|r| {
            let (a, c) = r.unwrap();
            std::iter::repeat_n(a, c as usize)
        })
        .collect();
    assert_eq!(decoded, assignments);
}

// =====================================================================
// Bit-packing boundary widths
// =====================================================================

/// Round-trip `assignment` through `BenStreamWriter::for_ben` + `BenStreamReader::from_ben`,
/// asserting the decoded result matches the input. Used by the bit-packing boundary-width sweep
/// below to confirm both encoder and decoder agree at every interesting width.
fn assert_ben_round_trip(assignment: Vec<u16>, variant: BenVariant) {
    use crate::io::reader::BenStreamReader;
    use crate::io::writer::BenStreamWriter;
    use std::io::Cursor;

    let mut ben = Vec::new();
    {
        let mut writer = BenStreamWriter::for_ben(&mut ben, variant).unwrap();
        writer.write_assignment(assignment.clone()).unwrap();
        if matches!(variant, BenVariant::TwoDelta) {
            // TwoDelta needs at least one delta frame after the anchor for the
            // delta-frame bit-packing path to be exercised at all; otherwise the round-trip only
            // tests the anchor frame (which is MkvChain-shaped).
            writer.write_assignment(assignment.clone()).unwrap();
        }
        writer.finish().unwrap();
    }
    let reader = BenStreamReader::from_ben(Cursor::new(&ben)).unwrap();
    let decoded: Vec<Vec<u16>> = reader
        .silent(true)
        .flat_map(|r| {
            let (a, c) = r.unwrap();
            std::iter::repeat_n(a, c as usize)
        })
        .collect();
    let expected = if matches!(variant, BenVariant::TwoDelta) {
        vec![assignment.clone(), assignment]
    } else {
        vec![assignment]
    };
    assert_eq!(
        decoded, expected,
        "BEN round-trip failed for variant {variant:?}"
    );
}

/// For each `width` in {1, 7, 8, 9, 16} — single-bit, just-under and just-over byte-aligned,
/// exactly byte-aligned, and the upper bound — exercise both the `mvb` and `mlb` sides of the
/// bit-packing code by constructing assignments whose encoded frame is forced to that width. The
/// encoder picks `max_val_bit_count = (16 - max_val.leading_zeros()).max(1)`, so a `max_val` of
/// `2.pow(width - 1)` (or 1 for width=1) lands on the requested width exactly.
///
/// The upper bound of `17` is not tested here because the encoder cannot produce it — `u16`
/// values cap bit widths at 16. Rejection of a hand-built frame with mvb=17 is already pinned by
/// `malformed_ben_bit_widths_return_invalid_data` in `tests/test_stress_edges.rs`.
///
/// `width=1` is a degenerate case: the only valid `max_val` is 1, so the assignment must contain
/// only the value 1, which collapses into a single run. mvb and mlb cannot be isolated
/// independently at this width, so it appears once via the `[1]` single-element fixture (mvb=1
/// and mlb=1 together).
#[test]
fn bit_packing_boundary_widths_round_trip() {
    // width=1: max_val=1 means all values are 1; the only way to keep mlb=1 too is a single-
    // element assignment. Both sides at width=1 are pinned by this one fixture.
    for variant in [
        BenVariant::Standard,
        BenVariant::MkvChain,
        BenVariant::TwoDelta,
    ] {
        assert_ben_round_trip(vec![1u16], variant);
    }

    for &width in &[7u8, 8, 9, 16] {
        let peak = 1u16 << (width - 1);

        // mvb sweep: alternating `1` and `peak` produces 4 runs of length 1, giving max_val=peak
        // (so mvb=width) and max_len=1 (so mlb=1). TwoDelta uses only mlb (its label pair is not
        // bit-packed against a max value), so it's skipped on this side.
        let mvb_assignment = vec![1u16, peak, 1, peak];
        for variant in [BenVariant::Standard, BenVariant::MkvChain] {
            assert_ben_round_trip(mvb_assignment.clone(), variant);
        }

        // mlb sweep: a single run of length `peak` of value 1. max_val=1 (mvb=1), max_len=peak
        // (mlb=width). All three variants exercise the run-length bit packing.
        let mlb_assignment = vec![1u16; peak as usize];
        for variant in [
            BenVariant::Standard,
            BenVariant::MkvChain,
            BenVariant::TwoDelta,
        ] {
            assert_ben_round_trip(mlb_assignment.clone(), variant);
        }
    }
}

/// Independently verify that the encoder actually picks the bit width we expect for each fixture
/// in `bit_packing_boundary_widths_round_trip`. If a future encoder change makes a different
/// width choice for the same input, the round-trip test above can still pass — this guards
/// against that drift.
#[test]
fn bit_packing_boundary_widths_pin_encoder_choice() {
    fn standard_widths(assignment: Vec<u16>) -> (u8, u8) {
        let frame = BenEncodeFrame::from_assignment(assignment, BenVariant::Standard, None);
        if let BenEncodeFrame::Standard {
            max_val_bit_count,
            max_len_bit_count,
            ..
        } = frame
        {
            (max_val_bit_count, max_len_bit_count)
        } else {
            panic!("expected Standard frame");
        }
    }

    // width=1: [1] → mvb=1, mlb=1.
    assert_eq!(standard_widths(vec![1u16]), (1, 1), "width=1 drift");

    for &width in &[7u8, 8, 9, 16] {
        let peak = 1u16 << (width - 1);

        // mvb side: [1, peak, 1, peak] → max_val=peak (mvb=width), max_len=1 (mlb=1).
        assert_eq!(
            standard_widths(vec![1u16, peak, 1, peak]),
            (width, 1),
            "mvb drift at width={width}"
        );

        // mlb side: [1; peak] → max_val=1 (mvb=1), max_len=peak (mlb=width).
        assert_eq!(
            standard_widths(vec![1u16; peak as usize]),
            (1, width),
            "mlb drift at width={width}"
        );
    }
}
