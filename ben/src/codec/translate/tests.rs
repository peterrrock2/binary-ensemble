use super::*;
use crate::codec::encode::{encode_ben32_line, encode_jsonl_to_ben};
use crate::util::rle::rle_to_vec;
use crate::{BenVariant, XBenVariant};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use rand_distr::{Distribution, Uniform};
use serde_json::{json, Value};
use std::io::{self, BufRead, Error, Read, Write};

fn encode_jsonl_to_ben32<R: BufRead, W: Write>(reader: R, mut writer: W) -> std::io::Result<()> {
    writer.write_all("STANDARD BEN FILE".as_bytes())?;
    for line_result in reader.lines() {
        let line = line_result?;
        let data: Value = serde_json::from_str(&line).expect("Error parsing JSON from line");

        writer.write_all(&encode_ben32_line(data)?)?;
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
    ben32_to_ben_lines(reader, writer, XBenVariant::Standard)
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
    ben_to_ben32_lines(reader, writer, XBenVariant::Standard)
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

#[test]
fn test_ben_to_ben32_lines_non_eof_error_on_frame_boundary() {
    // Provide a valid BEN frame followed by a read that errors with a non-EOF error at exactly the
    // point where the next frame's first byte would be read. This exercises the `return Err(e)`
    // branch (line ~191) in the `read_exact → match → Err(e) → not UnexpectedEof` path.
    struct FailOnSecondFrame {
        data: Vec<u8>,
        pos: usize,
        frame_boundary: usize,
    }

    impl Read for FailOnSecondFrame {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.pos >= self.frame_boundary {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "pipe broke on boundary",
                ));
            }
            let available = (self.frame_boundary - self.pos).min(buf.len());
            let end = self.pos + available;
            buf[..available].copy_from_slice(&self.data[self.pos..end]);
            self.pos = end;
            Ok(available)
        }
    }

    // Build a valid BEN Standard stream (without banner) containing one frame.
    let jsonl = r#"{"assignment":[1,2],"sample":1}
"#;
    let mut ben = Vec::new();
    encode_jsonl_to_ben(jsonl.as_bytes(), &mut ben, BenVariant::Standard).unwrap();
    let body = ben[17..].to_vec(); // strip banner
    let boundary = body.len(); // error right after the first frame

    let reader = FailOnSecondFrame {
        data: body,
        pos: 0,
        frame_boundary: boundary,
    };

    let mut output = Vec::new();
    let err = ben_to_ben32_lines(reader, &mut output, XBenVariant::Standard).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
}

#[test]
fn test_ben32_to_ben_line_rejects_invalid_length() {
    let err = ben32_to_ben_line(vec![1, 2, 3], XBenVariant::Standard, 0).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert_eq!(
        err.to_string(),
        "ben32 frame payload length 3 is not a multiple of 4"
    );
}

#[test]
fn test_ben32_to_ben_line_rejects_missing_terminator() {
    let err =
        ben32_to_ben_line(vec![0, 1, 0, 2, 0, 0, 0, 1], XBenVariant::Standard, 0).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert_eq!(
        err.to_string(),
        "ben32 frame missing 4-byte zero end-of-line sentinel at offset 8 (got [0, 0, 0, 1])"
    );
}

#[test]
fn test_ben32_to_ben_lines_preserves_mkv_counts() {
    let input = [
        0, 7, 0, 3, 0, 0, 0, 0, 0, 5, // one ben32 record and count=5
    ];

    let mut output = Vec::new();
    ben32_to_ben_lines(&input[..], &mut output, XBenVariant::MkvChain).unwrap();

    let count = u16::from_be_bytes([output[output.len() - 2], output[output.len() - 1]]);
    assert_eq!(count, 5);
}

#[test]
fn test_ben_to_ben32_lines_propagates_non_eof_read_errors() {
    struct FailAfterFirstByte {
        data: Vec<u8>,
        reads: usize,
    }

    impl Read for FailAfterFirstByte {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.reads += 1;
            if self.reads == 1 {
                buf[0] = self.data[0];
                Ok(1)
            } else {
                Err(io::Error::other("boom"))
            }
        }
    }

    let mut output = Vec::new();
    let err = ben_to_ben32_lines(
        FailAfterFirstByte {
            data: vec![1],
            reads: 0,
        },
        &mut output,
        XBenVariant::Standard,
    )
    .unwrap_err();

    assert_eq!(err.kind(), io::ErrorKind::Other);
    assert_eq!(err.to_string(), "boom");
}

#[test]
fn test_ben32_to_ben_lines_propagates_non_eof_read_errors() {
    struct BoomReader;

    impl Read for BoomReader {
        fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("boom"))
        }
    }

    let err = ben32_to_ben_lines(BoomReader, Vec::new(), XBenVariant::Standard).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::Other);
    assert_eq!(err.to_string(), "boom");
}

#[test]
fn test_ben_to_ben32_lines_mkv_roundtrip() {
    let jsonl = r#"{"assignment":[4,4,4],"sample":1}
{"assignment":[4,4,4],"sample":2}
{"assignment":[7,8],"sample":3}
"#;

    let mut ben = Vec::new();
    encode_jsonl_to_ben(jsonl.as_bytes(), &mut ben, BenVariant::MkvChain).unwrap();

    let mut ben32 = Vec::new();
    ben_to_ben32_lines(&ben[17..], &mut ben32, XBenVariant::MkvChain).unwrap();

    let mut round = Vec::new();
    ben32_to_ben_lines(ben32.as_slice(), &mut round, XBenVariant::MkvChain).unwrap();

    assert_eq!(round, ben[17..]);
}

#[test]
fn test_xben_variant_try_from_rejects_twodelta() {
    use crate::TwoDeltaNotXBenError;
    assert_eq!(
        XBenVariant::try_from(BenVariant::Standard).unwrap(),
        XBenVariant::Standard
    );
    assert_eq!(
        XBenVariant::try_from(BenVariant::MkvChain).unwrap(),
        XBenVariant::MkvChain
    );
    assert_eq!(
        XBenVariant::try_from(BenVariant::TwoDelta).unwrap_err(),
        TwoDeltaNotXBenError
    );
}

#[test]
fn test_translate_error_io_passthrough() {
    let inner = io::Error::new(io::ErrorKind::BrokenPipe, "pipe broke");
    let translate_err = super::errors::TranslateError::Io(inner);
    let io_err: io::Error = translate_err.into();
    assert_eq!(io_err.kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(io_err.to_string(), "pipe broke");
}
