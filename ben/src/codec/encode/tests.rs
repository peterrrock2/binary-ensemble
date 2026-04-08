use super::*;
use crate::codec::frames::{BenConstruct, BenEncodeFrame};
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
    let direct = BenEncodeFrame::from_assignment(assign_vec.clone(), None);
    let via_rle = BenEncodeFrame::from_rle(crate::util::rle::assign_to_rle(assign_vec), None);
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

    let frame =
        encode_twodelta_frame_with_hint(&prev, &curr, Some((1, 2)), Some(&mut masks), None)
            .unwrap();
    assert_eq!(frame.pair, (2, 1));
    assert!(!frame.run_length_vector.is_empty());
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

    let frame = encode_twodelta_frame_with_hint(&prev, &curr, None, Some(&mut masks), None)
        .unwrap();
    assert_eq!(frame.pair, (2, 1));
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
    let err =
        encode_twodelta_frame_with_hint(&prev, &curr, Some((1, 2)), None, None).unwrap_err();
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

    let err =
        encode_twodelta_frame_with_hint(&prev, &curr, Some((1, 1)), Some(&mut masks), None)
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

    let err =
        encode_twodelta_frame_with_hint(&a, &a, None, Some(&mut masks), None).unwrap_err();
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
