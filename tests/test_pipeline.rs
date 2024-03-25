use ben::decode::*;
use ben::encode::*;
use ben::utils::*;
use serde_json::json;
use std::io::{Cursor, Read, Write};

use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use rand_distr::{Distribution, Gamma, Uniform};

#[test]
fn test_ben_pipeline() {
    let seed = 129530786u64;
    let mut rng = ChaCha8Rng::seed_from_u64(seed);

    let n_samples = 100;

    let shape = 2.0;
    let scale = 50.0;
    let gamma = Gamma::new(shape, scale).unwrap();

    let mu = Uniform::new(1, 51);

    // In-memory buffer for streaming
    let mut buffer = Cursor::new(Vec::new());

    eprintln!();
    for i in 0..n_samples {
        print!("Generating sample: {}\r", i + 1);
        let mut rle_vec = Vec::new();
        while rle_vec.len() < 500 {
            rle_vec.push((mu.sample(&mut rng) as u16, gamma.sample(&mut rng) as u16));
        }

        // Directly write each JSON line to the buffer
        writeln!(
            &mut buffer,
            "{}",
            json!({
                "assignment": rle_to_vec(rle_vec),
                "sample": i+1
            })
        )
        .unwrap();
    }

    eprintln!();

    // Reset buffer cursor to the start
    buffer.set_position(0);

    let mut input_writer = Vec::new();
    let mut output_writer = Vec::new();

    // Assume these functions are adapted to work with streams
    jsonl_encode_ben(&mut buffer, &mut input_writer).unwrap();
    buffer.set_position(0); // Reset if needed for reuse
    jsonl_decode_ben(&input_writer[..], &mut output_writer).unwrap();

    // Reset buffer to compare
    buffer.set_position(0);
    let mut original_data = Vec::new();
    buffer.read_to_end(&mut original_data).unwrap();

    assert_eq!(original_data, output_writer);
}

#[test]
fn test_xben_pipeline() {
    let seed = 129530786u64;
    let mut rng = ChaCha8Rng::seed_from_u64(seed);

    let n_samples = 100;

    let shape = 2.0;
    let scale = 200.0;
    let gamma = Gamma::new(shape, scale).unwrap();

    let mu = Uniform::new(1, 51);

    // In-memory buffer for streaming
    let mut buffer = Vec::new();
    let mut sample_writer = Cursor::new(&mut buffer);

    eprintln!();
    for i in 0..n_samples {
        print!("Generating sample: {}\r", i + 1);
        let mut rle_vec = Vec::new();
        while rle_vec.len() < 500 {
            rle_vec.push((
                mu.sample(&mut rng) as u16,
                gamma.sample(&mut rng) as u16 + 1,
            ));
        }

        let line = json!({
            "assignment": rle_to_vec(rle_vec),
            "sample": i+1
        })
        .to_string()
            + "\n";

        sample_writer.write_all(&line.as_bytes()).unwrap();
    }
    eprintln!();

    sample_writer.set_position(0);
    let mut original_data = Vec::new();
    sample_writer.read_to_end(&mut original_data).unwrap();

    sample_writer.set_position(0);

    let mut input_writer = Vec::new();
    let mut output_writer = Vec::new();

    // Assume these functions are adapted to work with streams
    jsonl_encode_xben(sample_writer, &mut input_writer).unwrap();
    decode_xben_to_ben(&input_writer[..], &mut output_writer).unwrap();

    let mut xoutput_writer = Vec::new();
    jsonl_decode_ben(&output_writer[..], &mut xoutput_writer).unwrap();

    assert_eq!(original_data, xoutput_writer);
}
