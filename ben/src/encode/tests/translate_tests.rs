use super::*;
use crate::encode::*;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use rand_distr::{Distribution, Uniform};
use serde_json::{json, Value};
use std::io::BufRead;

fn encode_jsonl_to_ben32<R: BufRead, W: Write>(reader: R, mut writer: W) -> std::io::Result<()> {
    writer.write_all("STANDARD BEN FILE".as_bytes())?;
    for line_result in reader.lines() {
        let line = line_result?; // Handle potential I/O errors for each line
        let data: Value = serde_json::from_str(&line).expect("Error parsing JSON from line");

        writer.write_all(&encode_ben32_line(data))?;
    }
    Ok(())
}

fn translate_ben32_to_ben_file<R: Read, W: Write>(mut reader: R, mut writer: W) -> io::Result<()> {
    let mut check_buffer = [0u8; 17];
    reader.read_exact(&mut check_buffer)?;

    if &check_buffer != b"STANDARD BEN FILE" {
        return Err(Error::new(
            io::ErrorKind::InvalidData,
            "Invalid file format",
        ));
    }

    writer.write_all(b"STANDARD BEN FILE")?;
    ben32_to_ben_lines(reader, writer, BenVariant::Standard)
}

fn translate_ben_to_ben32_file<R: Read, W: Write>(mut reader: R, mut writer: W) -> io::Result<()> {
    let mut check_buffer = [0u8; 17];
    reader.read_exact(&mut check_buffer)?;

    if &check_buffer != b"STANDARD BEN FILE" {
        return Err(Error::new(
            io::ErrorKind::InvalidData,
            "Invalid file format",
        ));
    }

    writer.write_all(b"STANDARD BEN FILE")?;
    ben_to_ben32_lines(reader, writer, BenVariant::Standard)
}

#[test]
fn test_simple_translation_ben32_to_ben() {
    let rle_lst: Vec<Vec<(u16, u16)>> = vec![vec![(10, 6), (2, 2)], vec![(5, 3), (1, 10)]];

    let mut full_data = String::new();

    for (i, rle_vec) in rle_lst.into_iter().enumerate() {
        let assign_vec = rle_to_vec(rle_vec);

        let data = json!({
            "assignment": assign_vec,
            "sample": i+1,
        });

        full_data = full_data + &json!(data).to_string() + "\n";
    }

    let mut input: Vec<u8> = Vec::new();
    let input_writer = &mut input;

    encode_jsonl_to_ben32(full_data.as_bytes(), input_writer).unwrap();

    let mut reader = input.as_slice();
    let mut output: Vec<u8> = Vec::new();
    let mut writer = &mut output;

    if let Err(_) = translate_ben32_to_ben_file(&mut reader, &mut writer) {
        assert!(false)
    }

    let mut buffer: Vec<u8> = Vec::new();
    let writer2 = &mut buffer;

    encode_jsonl_to_ben(full_data.as_bytes(), writer2, BenVariant::Standard).unwrap();

    assert_eq!(writer, &buffer);
}

#[test]
fn test_random_translation_ben32_to_ben() {
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let uniform100 = Uniform::new(1, 101).expect("Could not make uniform sampler");
    let uniform10 = Uniform::new(1, 11).expect("Could not make uniform sampler");

    let mut rle_lst: Vec<Vec<(u16, u16)>> = Vec::new();

    for _ in 0..100 {
        let mut rle_vec: Vec<(u16, u16)> = Vec::new();
        let n = uniform100.sample(&mut rng);

        for _ in 0..n {
            let val = uniform10.sample(&mut rng);
            let len = uniform100.sample(&mut rng);
            rle_vec.push((val, len));
        }
        rle_lst.push(rle_vec);
    }

    let mut full_data = String::new();

    for (i, rle_vec) in rle_lst.into_iter().enumerate() {
        let assign_vec = rle_to_vec(rle_vec);

        let data = json!({
            "assignment": assign_vec,
            "sample": i+1,
        });

        full_data = full_data + &json!(data).to_string() + "\n";
    }

    let mut input: Vec<u8> = Vec::new();
    let input_writer = &mut input;

    encode_jsonl_to_ben32(full_data.as_bytes(), input_writer).unwrap();

    let mut reader = input.as_slice();
    let mut output: Vec<u8> = Vec::new();
    let mut writer = &mut output;

    if let Err(_) = translate_ben32_to_ben_file(&mut reader, &mut writer) {
        assert!(false)
    }

    let mut buffer: Vec<u8> = Vec::new();
    let writer2 = &mut buffer;

    encode_jsonl_to_ben(full_data.as_bytes(), writer2, BenVariant::Standard).unwrap();

    assert_eq!(writer, &buffer);
}

#[test]
fn test_simple_translation_ben_to_ben32() {
    let rle_lst: Vec<Vec<(u16, u16)>> = vec![vec![(10, 6), (2, 2)], vec![(5, 3), (1, 10)]];

    let mut full_data = String::new();

    for (i, rle_vec) in rle_lst.into_iter().enumerate() {
        let assign_vec = rle_to_vec(rle_vec);

        let data = json!({
            "assignment": assign_vec,
            "sample": i+1,
        });

        full_data = full_data + &json!(data).to_string() + "\n";
    }

    let mut input: Vec<u8> = Vec::new();
    let input_writer = &mut input;

    encode_jsonl_to_ben(full_data.as_bytes(), input_writer, BenVariant::Standard).unwrap();

    let mut reader = input.as_slice();
    let mut output: Vec<u8> = Vec::new();
    let mut writer = &mut output;

    if let Err(e) = translate_ben_to_ben32_file(&mut reader, &mut writer) {
        eprintln!("{:?}", e);
        assert!(false)
    }

    let mut buffer: Vec<u8> = Vec::new();
    let writer2 = &mut buffer;

    encode_jsonl_to_ben32(full_data.as_bytes(), writer2).unwrap();

    assert_eq!(writer, &buffer);
}

#[test]
fn test_random_translation_ben_to_ben32() {
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let uniform100 = Uniform::new(1, 101).expect("Could not make uniform sampler");
    let uniform10 = Uniform::new(1, 11).expect("Could not make uniform sampler");

    let mut rle_lst: Vec<Vec<(u16, u16)>> = Vec::new();

    for _ in 0..100 {
        let mut rle_vec: Vec<(u16, u16)> = Vec::new();
        let n = uniform100.sample(&mut rng);

        for _ in 0..n {
            let val = uniform10.sample(&mut rng);
            let len = uniform100.sample(&mut rng);
            rle_vec.push((val, len));
        }
        rle_lst.push(rle_vec);
    }

    let mut full_data = String::new();

    for (i, rle_vec) in rle_lst.into_iter().enumerate() {
        let assign_vec = rle_to_vec(rle_vec);

        let data = json!({
            "assignment": assign_vec,
            "sample": i+1,
        });

        full_data = full_data + &json!(data).to_string() + "\n";
    }

    let mut input: Vec<u8> = Vec::new();
    let input_writer = &mut input;

    encode_jsonl_to_ben(full_data.as_bytes(), input_writer, BenVariant::Standard).unwrap();

    let mut reader = input.as_slice();
    let mut output: Vec<u8> = Vec::new();
    let mut writer = &mut output;

    if let Err(_) = translate_ben_to_ben32_file(&mut reader, &mut writer) {
        assert!(false)
    }

    let mut buffer: Vec<u8> = Vec::new();
    let writer2 = &mut buffer;

    encode_jsonl_to_ben32(full_data.as_bytes(), writer2).unwrap();

    assert_eq!(writer, &buffer);
}
