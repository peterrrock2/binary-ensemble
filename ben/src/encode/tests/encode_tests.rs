use super::*;
use serde_json::json;

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
        let line = line_result?; // Handle potential I/O errors for each line
        let data: Value = serde_json::from_str(&line).expect("Error parsing JSON from line");

        writer.write_all(&encode_ben32_line(data))?;
    }
    eprintln!("Done!"); // Print newline after progress bar
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
