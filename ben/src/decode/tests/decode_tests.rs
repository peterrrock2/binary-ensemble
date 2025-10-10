use super::*;
use serde_json::{json, Value};

#[test]
fn test_jsonl_decode_ben_underflow() {
    let mut input: Vec<u8> = b"STANDARD BEN FILE".to_vec();
    input.extend(vec![
        2,
        3,
        0,
        0,
        0,
        2, // N Bytes
        0b01100_100,
        0b01_11011_0,
    ]);

    let mut reader = input.as_slice();
    let mut output: Vec<u8> = Vec::new();
    let writer = &mut output;

    let result = decode_ben_to_jsonl(&mut reader, writer);
    if let Err(e) = result {
        panic!("Error: {}", e);
    }

    let rle_assign = vec![(1, 4), (2, 1), (3, 3)];

    let expected_output = json!({
        "assignment": rle_to_vec(rle_assign).iter().map(|x| json!(x)).collect::<Vec<Value>>(),
        "sample": 1
    });

    assert_eq!(output, (expected_output.to_string() + "\n").as_bytes());
}

#[test]
fn test_jsonl_decode_ben_exact() {
    let mut input: Vec<u8> = b"STANDARD BEN FILE".to_vec();
    input.extend(vec![
        2,
        3,
        0,
        0,
        0,
        5,
        0b01100_100,
        0b01_11011_1,
        0b0010_1111,
        0b1_01001_10,
        0b001_11001_,
    ]);

    let mut reader = input.as_slice();
    let mut output: Vec<u8> = Vec::new();
    let writer = &mut output;

    let result = decode_ben_to_jsonl(&mut reader, writer);
    if let Err(e) = result {
        panic!("Error: {}", e);
    }

    let rle_assign = vec![
        (1, 4),
        (2, 1),
        (3, 3),
        (2, 2),
        (3, 7),
        (1, 1),
        (2, 1),
        (3, 1),
    ];

    let expected_output = json!({
        "assignment": rle_to_vec(rle_assign).iter().map(|x| json!(x)).collect::<Vec<Value>>(),
        "sample": 1
    });

    assert_eq!(output, (expected_output.to_string() + "\n").as_bytes());
}

#[test]
fn test_jsonl_decode_ben_16_bit_val() {
    let mut input: Vec<u8> = b"STANDARD BEN FILE".to_vec();
    input.extend(vec![
        10,
        3,
        0,
        0,
        0,
        5,
        0b00000000,
        0b01100_100,
        0b00000000,
        0b01_000000,
        0b0011011_0,
    ]);

    let mut reader = input.as_slice();
    let mut output: Vec<u8> = Vec::new();
    let writer = &mut output;

    let result = decode_ben_to_jsonl(&mut reader, writer);
    if let Err(e) = result {
        panic!("Error: {}", e);
    }

    let rle_assign = vec![(1, 4), (512, 1), (3, 3)];

    let expected_output = json!({
        "assignment": rle_to_vec(rle_assign).iter().map(|x| json!(x)).collect::<Vec<Value>>(),
        "sample": 1
    });

    assert_eq!(output, (expected_output.to_string() + "\n").as_bytes());
}

#[test]
fn test_jsonl_decode_ben_16_bit_len() {
    let mut input: Vec<u8> = b"STANDARD BEN FILE".to_vec();
    input.extend(vec![
        2,
        10,
        0,
        0,
        0,
        5,
        0b01000000,
        0b0100_1010,
        0b00000000_,
        0b11000000,
        0b0011_0000,
    ]);

    let mut reader = input.as_slice();
    let mut output: Vec<u8> = Vec::new();
    let writer = &mut output;

    let result = decode_ben_to_jsonl(&mut reader, writer);
    if let Err(e) = result {
        panic!("Error: {}", e);
    }

    let rle_assign = vec![(1, 4), (2, 512), (3, 3)];

    let expected_output = json!({
        "assignment": rle_to_vec(rle_assign).iter().map(|x| json!(x)).collect::<Vec<Value>>(),
        "sample": 1
    });

    assert_eq!(output, (expected_output.to_string() + "\n").as_bytes());
}

#[test]
fn test_jsonl_decode_ben_max_val_65535() {
    let mut input: Vec<u8> = b"STANDARD BEN FILE".to_vec();
    input.extend(vec![
        16,
        4,
        0,
        0,
        0,
        8,
        0b00000000,
        0b00010111,
        0b0100_1111,
        0b11111111,
        0b11111111_,
        0b00000000,
        0b00001000,
        0b0011_0000,
    ]);

    let mut reader = input.as_slice();
    let mut output: Vec<u8> = Vec::new();
    let writer = &mut output;

    let result = decode_ben_to_jsonl(&mut reader, writer);
    if let Err(e) = result {
        panic!("Error: {}", e);
    }

    let rle_assign = vec![(23, 4), (65535, 15), (8, 3)];

    let expected_output = json!({
        "assignment": rle_to_vec(rle_assign).iter().map(|x| json!(x)).collect::<Vec<Value>>(),
        "sample": 1
    });

    assert_eq!(output, (expected_output.to_string() + "\n").as_bytes());
}

#[test]
fn test_jsonl_decode_ben_max_len_65535() {
    let mut input: Vec<u8> = b"STANDARD BEN FILE".to_vec();
    input.extend(vec![
        6,
        16,
        0,
        0,
        0,
        9,
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

    let mut reader = input.as_slice();
    let mut output: Vec<u8> = Vec::new();
    let writer = &mut output;

    let result = decode_ben_to_jsonl(&mut reader, writer);
    if let Err(e) = result {
        panic!("Error: {}", e);
    }

    let rle_assign = vec![(23, 4), (60, 65535), (8, 3)];

    let expected_output = json!({
        "assignment": rle_to_vec(rle_assign).iter().map(|x| json!(x)).collect::<Vec<Value>>(),
        "sample": 1
    });

    assert_eq!(output, (expected_output.to_string() + "\n").as_bytes());
}

#[test]
fn test_decode_ben_max_val_and_len_at_65535() {
    let mut input: Vec<u8> = b"STANDARD BEN FILE".to_vec();
    input.extend(vec![
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

    let mut reader = input.as_slice();
    let mut output: Vec<u8> = Vec::new();

    let result = decode_ben_to_jsonl(&mut reader, &mut output);
    if let Err(e) = result {
        panic!("Error: {}", e);
    }

    let rle_assign = vec![(1, 3), (65535, 65535), (8, 4)];

    let expected_output = json!({
        "assignment": rle_to_vec(rle_assign).iter().map(|x| json!(x)).collect::<Vec<Value>>(),
        "sample": 1
    });

    assert_eq!(output, (expected_output.to_string() + "\n").as_bytes());
}

#[test]
fn test_decode_ben_single_element() {
    let mut input: Vec<u8> = b"STANDARD BEN FILE".to_vec();
    input.extend(vec![5, 1, 0, 0, 0, 1, 0b101111_00]);

    let mut reader = input.as_slice();
    let mut output: Vec<u8> = Vec::new();
    let writer = &mut output;

    let result = decode_ben_to_jsonl(&mut reader, writer);
    if let Err(e) = result {
        panic!("Error: {}", e);
    }

    let expected_output = json!({
        "assignment": vec![json!(23)],
        "sample": 1
    });

    assert_eq!(output, (expected_output.to_string() + "\n").as_bytes());
}

#[test]
fn test_decode_ben_single_one() {
    let mut input: Vec<u8> = b"STANDARD BEN FILE".to_vec();
    input.extend(vec![1, 1, 0, 0, 0, 1, 0b11_000000]);

    let mut reader = input.as_slice();
    let mut output: Vec<u8> = Vec::new();
    let writer = &mut output;

    let result = decode_ben_to_jsonl(&mut reader, writer);
    if let Err(e) = result {
        panic!("Error: {}", e);
    }

    let expected_output = json!({
        "assignment": vec![json!(1)],
        "sample": 1
    });

    assert_eq!(output, (expected_output.to_string() + "\n").as_bytes());
}

#[test]
fn test_decode_ben_multiple_simple_lines() {
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
    let mut output: Vec<u8> = Vec::new();
    let writer = &mut output;

    let result = decode_ben_to_jsonl(&mut reader, writer);
    if let Err(e) = result {
        panic!("Error: {}", e);
    }

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

    let mut expected_output: Vec<String> = Vec::new();

    for (i, rle_vec) in rle_lst.into_iter().enumerate() {
        let assign_vec = rle_to_vec(rle_vec)
            .iter()
            .map(|x| json!(x))
            .collect::<Vec<Value>>();

        let data = json!({
            "assignment": assign_vec,
            "sample": i+1,
        });

        expected_output.push(data.to_string() + "\n");
    }

    assert_eq!(output, expected_output.concat().as_bytes());
}

#[test]
fn test_jsonl_decode_ben32_simple() {
    let input = vec![0, 1, 0, 4, 0, 2, 0, 1, 0, 3, 0, 3, 0, 0, 0, 0];

    let mut reader = input.as_slice();
    let mut output: Vec<u8> = Vec::new();
    let writer = &mut output;

    let result = jsonl_decode_ben32(&mut reader, writer, 0, BenVariant::Standard);

    if let Err(e) = result {
        panic!("Error: {}", e);
    }

    let rle_assign = vec![(1, 4), (2, 1), (3, 3)];

    let expected_output = json!({
        "assignment": rle_to_vec(rle_assign).iter().map(|x| json!(x)).collect::<Vec<Value>>(),
        "sample": 1
    });

    assert_eq!(output, (expected_output.to_string() + "\n").as_bytes());
}

#[test]
fn test_jsonl_decode_ben32_16_bit_val() {
    let input = vec![0, 1, 0, 4, 2, 0, 0, 1, 0, 3, 0, 3, 0, 0, 0, 0];

    let mut reader = input.as_slice();
    let mut output: Vec<u8> = Vec::new();
    let writer = &mut output;

    let result = jsonl_decode_ben32(&mut reader, writer, 0, BenVariant::Standard);
    if let Err(e) = result {
        panic!("Error: {}", e);
    }

    let rle_assign = vec![(1, 4), (512, 1), (3, 3)];

    let expected_output = json!({
        "assignment": rle_to_vec(rle_assign).iter().map(|x| json!(x)).collect::<Vec<Value>>(),
        "sample": 1
    });

    assert_eq!(output, (expected_output.to_string() + "\n").as_bytes());
}

#[test]
fn test_jsonl_decode_ben32_16_bit_len() {
    let input = vec![0, 1, 0, 4, 0, 2, 2, 0, 0, 3, 0, 3, 0, 0, 0, 0];

    let mut reader = input.as_slice();
    let mut output: Vec<u8> = Vec::new();
    let writer = &mut output;

    let result = jsonl_decode_ben32(&mut reader, writer, 0, BenVariant::Standard);
    if let Err(e) = result {
        panic!("Error: {}", e);
    }

    let rle_assign = vec![(1, 4), (2, 512), (3, 3)];

    let expected_output = json!({
        "assignment": rle_to_vec(rle_assign).iter().map(|x| json!(x)).collect::<Vec<Value>>(),
        "sample": 1
    });

    assert_eq!(output, (expected_output.to_string() + "\n").as_bytes());
}

#[test]
fn test_jsonl_decode_ben32_max_val_65535() {
    let input = vec![0, 23, 0, 4, 255, 255, 0, 15, 0, 8, 0, 3, 0, 0, 0, 0];

    let mut reader = input.as_slice();
    let mut output: Vec<u8> = Vec::new();
    let writer = &mut output;

    let result = jsonl_decode_ben32(&mut reader, writer, 0, BenVariant::Standard);
    if let Err(e) = result {
        panic!("Error: {}", e);
    }

    let rle_assign = vec![(23, 4), (65535, 15), (8, 3)];

    let expected_output = json!({
        "assignment": rle_to_vec(rle_assign).iter().map(|x| json!(x)).collect::<Vec<Value>>(),
        "sample": 1
    });

    assert_eq!(output, (expected_output.to_string() + "\n").as_bytes());
}

#[test]
fn test_jsonl_decode_ben32_max_len_65535() {
    let input = vec![0, 23, 0, 4, 0, 60, 255, 255, 0, 8, 0, 3, 0, 0, 0, 0];

    let mut reader = input.as_slice();
    let mut output: Vec<u8> = Vec::new();
    let writer = &mut output;

    let result = jsonl_decode_ben32(&mut reader, writer, 0, BenVariant::Standard);
    if let Err(e) = result {
        panic!("Error: {}", e);
    }

    let rle_assign = vec![(23, 4), (60, 65535), (8, 3)];

    let expected_output = json!({
        "assignment": rle_to_vec(rle_assign).iter().map(|x| json!(x)).collect::<Vec<Value>>(),
        "sample": 1
    });

    assert_eq!(output, (expected_output.to_string() + "\n").as_bytes());
}

#[test]
fn test_decode_ben32_single_element() {
    let input: Vec<u8> = vec![0, 23, 0, 1, 0, 0, 0, 0];

    let mut reader = input.as_slice();
    let mut output: Vec<u8> = Vec::new();
    let writer = &mut output;

    let result = jsonl_decode_ben32(&mut reader, writer, 0, BenVariant::Standard);
    println!("result {:?}", result);
    if let Err(e) = result {
        panic!("Error: {}", e);
    }

    let expected_output = json!({
        "assignment": vec![json!(23)],
        "sample": 1
    });

    assert_eq!(output, (expected_output.to_string() + "\n").as_bytes());
}

#[test]
fn test_decode_ben32_multiple_simple_lines() {
    let input = vec![
        0, 1, 0, 4, 0, 2, 0, 4, 0, 3, 0, 4, 0, 4, 0, 4, 0, 0, 0, 0, 0, 2, 0, 2, 0, 3, 0, 7, 0, 1,
        0, 1, 0, 2, 0, 1, 0, 3, 0, 1, 0, 0, 0, 0, 0, 1, 0, 1, 0, 2, 0, 1, 0, 3, 0, 1, 0, 4, 0, 1,
        0, 5, 0, 1, 0, 6, 0, 1, 0, 7, 0, 1, 0, 8, 0, 1, 0, 9, 0, 1, 0, 10, 0, 1, 0, 0, 0, 0,
    ];

    let mut reader = input.as_slice();
    let mut output: Vec<u8> = Vec::new();
    let writer = &mut output;

    let result = jsonl_decode_ben32(&mut reader, writer, 0, BenVariant::Standard);
    if let Err(e) = result {
        panic!("Error: {}", e);
    }

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

    let mut expected_output: Vec<String> = Vec::new();

    for (i, rle_vec) in rle_lst.into_iter().enumerate() {
        let assign_vec = rle_to_vec(rle_vec)
            .iter()
            .map(|x| json!(x))
            .collect::<Vec<Value>>();

        let data = json!({
            "assignment": assign_vec,
            "sample": i+1,
        });

        expected_output.push(data.to_string() + "\n");
    }

    assert_eq!(output, expected_output.concat().as_bytes());
}
