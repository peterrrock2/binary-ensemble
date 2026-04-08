use super::*;
use crate::codec::decode::decode_ben_to_jsonl;
use crate::codec::encode::encode_jsonl_to_ben;
use crate::codec::{BenConstruct, BenEncodeFrame};
use crate::util::rle::assign_to_rle;
use rand::seq::SliceRandom;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use rand_distr::{Distribution, Uniform};
use std::collections::HashMap;
use std::io;

fn shuffle_with_mapping<T>(vec: &mut Vec<T>) -> HashMap<usize, usize>
where
    T: Clone + std::cmp::PartialEq,
{
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let original_vec = vec.clone();
    vec.shuffle(&mut rng);

    let mut map = HashMap::new();
    for (new_index, item) in vec.iter().enumerate() {
        let original_index = original_vec.iter().position(|i| i == item).unwrap();
        map.insert(new_index, original_index);
    }
    map
}

#[test]
fn test_relabel_ben_line_simple() {
    let in_rle = vec![(2, 2), (3, 2), (1, 2), (4, 2)];

    let input = BenEncodeFrame::from_rle(in_rle, None);

    let out_rle = vec![(1, 2), (2, 2), (3, 2), (4, 2)];
    let expected = BenEncodeFrame::from_rle(out_rle, None);

    let mut buf = Vec::new();
    relabel_ben_lines(input.as_slice(), &mut buf, BenVariant::Standard).unwrap();

    assert_eq!(buf, expected);
}

#[test]
fn test_relabel_simple_file() {
    let file = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        "{\"assignment\":[1,2,3,4,5,5,3,4,2],\"sample\":1}",
        "{\"assignment\":[2,1,3,4,5,5,3,4,2],\"sample\":2}",
        "{\"assignment\":[3,3,1,1,2,2,3,3,4],\"sample\":3}",
        "{\"assignment\":[4,3,2,1,4,3,2,1,1],\"sample\":4}",
        "{\"assignment\":[3,2,2,4,1,3,1,4,3],\"sample\":5}",
        "{\"assignment\":[2,2,3,3,4,4,5,5,1],\"sample\":6}",
        "{\"assignment\":[2,4,1,5,2,4,3,1,3],\"sample\":7}"
    );

    let input = file.as_bytes();

    let mut output = Vec::new();
    let writer = io::BufWriter::new(&mut output);

    encode_jsonl_to_ben(input, writer, BenVariant::Standard).unwrap();

    let mut output2 = Vec::new();
    let writer2 = io::BufWriter::new(&mut output2);
    relabel_ben_file(output.as_slice(), writer2).unwrap();

    let mut output3 = Vec::new();
    let writer3 = io::BufWriter::new(&mut output3);
    decode_ben_to_jsonl(output2.as_slice(), writer3).unwrap();

    let output_str = String::from_utf8(output3).unwrap();

    let out_file = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        "{\"assignment\":[1,2,3,4,5,5,3,4,2],\"sample\":1}",
        "{\"assignment\":[1,2,3,4,5,5,3,4,1],\"sample\":2}",
        "{\"assignment\":[1,1,2,2,3,3,1,1,4],\"sample\":3}",
        "{\"assignment\":[1,2,3,4,1,2,3,4,4],\"sample\":4}",
        "{\"assignment\":[1,2,2,3,4,1,4,3,1],\"sample\":5}",
        "{\"assignment\":[1,1,2,2,3,3,4,4,5],\"sample\":6}",
        "{\"assignment\":[1,2,3,4,1,2,5,3,5],\"sample\":7}"
    );

    assert_eq!(output_str, out_file);
}

#[test]
fn test_relabel_simple_file_mkv() {
    let file = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        "{\"assignment\":[1,2,3,4,5,5,3,4,2],\"sample\":1}",
        "{\"assignment\":[2,1,3,4,5,5,3,4,2],\"sample\":2}",
        "{\"assignment\":[3,3,1,1,2,2,3,3,4],\"sample\":3}",
        "{\"assignment\":[4,3,2,1,4,3,2,1,1],\"sample\":4}",
        "{\"assignment\":[3,2,2,4,1,3,1,4,3],\"sample\":5}",
        "{\"assignment\":[3,2,2,4,1,3,1,4,3],\"sample\":6}",
        "{\"assignment\":[3,2,2,4,1,3,1,4,3],\"sample\":7}",
        "{\"assignment\":[2,2,3,3,4,4,5,5,1],\"sample\":8}",
        "{\"assignment\":[2,4,1,5,2,4,3,1,3],\"sample\":9}",
        "{\"assignment\":[2,4,1,5,2,4,3,1,3],\"sample\":10}"
    );

    let input = file.as_bytes();

    let mut output = Vec::new();
    let writer = io::BufWriter::new(&mut output);

    encode_jsonl_to_ben(input, writer, BenVariant::MkvChain).unwrap();

    let mut output2 = Vec::new();
    let writer2 = io::BufWriter::new(&mut output2);
    relabel_ben_file(output.as_slice(), writer2).unwrap();

    let mut output3 = Vec::new();
    let writer3 = io::BufWriter::new(&mut output3);
    decode_ben_to_jsonl(output2.as_slice(), writer3).unwrap();

    let output_str = String::from_utf8(output3).unwrap();

    let out_file = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        "{\"assignment\":[1,2,3,4,5,5,3,4,2],\"sample\":1}",
        "{\"assignment\":[1,2,3,4,5,5,3,4,1],\"sample\":2}",
        "{\"assignment\":[1,1,2,2,3,3,1,1,4],\"sample\":3}",
        "{\"assignment\":[1,2,3,4,1,2,3,4,4],\"sample\":4}",
        "{\"assignment\":[1,2,2,3,4,1,4,3,1],\"sample\":5}",
        "{\"assignment\":[1,2,2,3,4,1,4,3,1],\"sample\":6}",
        "{\"assignment\":[1,2,2,3,4,1,4,3,1],\"sample\":7}",
        "{\"assignment\":[1,1,2,2,3,3,4,4,5],\"sample\":8}",
        "{\"assignment\":[1,2,3,4,1,2,5,3,5],\"sample\":9}",
        "{\"assignment\":[1,2,3,4,1,2,5,3,5],\"sample\":10}"
    );

    assert_eq!(output_str, out_file);
}

#[test]
fn test_relabel_simple_file_mkv_with_limit() {
    let file = concat!(
        "{\"assignment\":[1,2,3],\"sample\":1}\n",
        "{\"assignment\":[1,2,3],\"sample\":2}\n",
        "{\"assignment\":[1,2,3],\"sample\":3}\n",
        "{\"assignment\":[2,3,1],\"sample\":4}\n"
    );

    let mut encoded = Vec::new();
    encode_jsonl_to_ben(
        file.as_bytes(),
        io::BufWriter::new(&mut encoded),
        BenVariant::MkvChain,
    )
    .unwrap();

    let mut relabeled = Vec::new();
    relabel_ben_file_limit(encoded.as_slice(), io::BufWriter::new(&mut relabeled), 2).unwrap();

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(relabeled.as_slice(), io::BufWriter::new(&mut decoded)).unwrap();

    let output_str = String::from_utf8(decoded).unwrap();
    let expected = concat!(
        "{\"assignment\":[1,2,3],\"sample\":1}\n",
        "{\"assignment\":[1,2,3],\"sample\":2}\n"
    );
    assert_eq!(output_str, expected);
}

#[test]
fn test_relabel_simple_file_twodelta() {
    let file = concat!(
        "{\"assignment\":[1,1,2,2,3,3],\"sample\":1}\n",
        "{\"assignment\":[1,1,2,2,3,3],\"sample\":2}\n",
        "{\"assignment\":[1,2,2,1,3,3],\"sample\":3}\n",
        "{\"assignment\":[2,2,1,1,3,3],\"sample\":4}\n"
    );

    let mut encoded = Vec::new();
    encode_jsonl_to_ben(
        file.as_bytes(),
        io::BufWriter::new(&mut encoded),
        BenVariant::TwoDelta,
    )
    .unwrap();

    let mut relabeled = Vec::new();
    relabel_ben_file(encoded.as_slice(), io::BufWriter::new(&mut relabeled)).unwrap();

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(relabeled.as_slice(), io::BufWriter::new(&mut decoded)).unwrap();

    let output_str = String::from_utf8(decoded).unwrap();
    let expected = concat!(
        "{\"assignment\":[1,1,2,2,3,3],\"sample\":1}\n",
        "{\"assignment\":[1,1,2,2,3,3],\"sample\":2}\n",
        "{\"assignment\":[1,2,2,1,3,3],\"sample\":3}\n",
        "{\"assignment\":[1,1,2,2,3,3],\"sample\":4}\n"
    );
    assert_eq!(output_str, expected);
}

#[test]
fn test_relabel_ben_line_with_map() {
    let in_assign = vec![2, 3, 1, 4, 5, 5, 3, 4, 2];
    let in_rle = assign_to_rle(in_assign);

    let input = BenEncodeFrame::from_rle(in_rle, None);

    let out_assign = vec![1, 2, 2, 3, 3, 4, 4, 5, 5];
    let out_rle = assign_to_rle(out_assign);
    let expected = BenEncodeFrame::from_rle(out_rle, None);

    let mut new_to_old_map = HashMap::new();
    new_to_old_map.insert(0, 2);
    new_to_old_map.insert(1, 0);
    new_to_old_map.insert(2, 8);
    new_to_old_map.insert(3, 1);
    new_to_old_map.insert(4, 6);
    new_to_old_map.insert(5, 3);
    new_to_old_map.insert(6, 7);
    new_to_old_map.insert(7, 4);
    new_to_old_map.insert(8, 5);

    let mut buf = Vec::new();
    relabel_ben_lines_with_map(
        input.as_slice(),
        &mut buf,
        new_to_old_map,
        BenVariant::Standard,
    )
    .unwrap();

    assert_eq!(buf, expected);
}

#[test]
fn test_relabel_ben_line_with_shuffle() {
    let in_assign = vec![2, 3, 1, 4, 5, 5, 3, 4, 2];
    let mut out_assign = in_assign.clone();

    let in_rle = assign_to_rle(in_assign);
    let input = BenEncodeFrame::from_rle(in_rle, None);

    let new_to_old_map = shuffle_with_mapping(&mut out_assign);
    let out_rle = assign_to_rle(out_assign);
    let expected = BenEncodeFrame::from_rle(out_rle, None);

    let mut buf = Vec::new();
    relabel_ben_lines_with_map(
        input.as_slice(),
        &mut buf,
        new_to_old_map,
        BenVariant::Standard,
    )
    .unwrap();

    assert_eq!(buf, expected);
}

#[test]
fn test_relabel_ben_line_with_large_shuffle() {
    let seed = 129530786u64;
    let mut rng = ChaCha8Rng::seed_from_u64(seed);

    let mu = Uniform::new(1, 21).expect("Could not make uniform sampler");

    let in_assign = (0..100_000)
        .map(|_| mu.sample(&mut rng) as u16)
        .collect::<Vec<u16>>();
    let mut out_assign = in_assign.clone();

    let in_rle = assign_to_rle(in_assign.to_vec());
    let input = BenEncodeFrame::from_rle(in_rle, None);

    let new_to_old_map = shuffle_with_mapping(&mut out_assign);
    let out_rle = assign_to_rle(out_assign);
    let expected = BenEncodeFrame::from_rle(out_rle, None);

    let mut buf = Vec::new();
    relabel_ben_lines_with_map(
        input.as_slice(),
        &mut buf,
        new_to_old_map,
        BenVariant::Standard,
    )
    .unwrap();

    assert_eq!(buf, expected);
}

#[test]
fn test_relabel_simple_file_with_map() {
    let file = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        "{\"assignment\":[1,2,3,4,5,5,3,4,2],\"sample\":1}",
        "{\"assignment\":[2,1,3,4,5,5,3,4,2],\"sample\":2}",
        "{\"assignment\":[3,3,1,1,2,2,3,3,4],\"sample\":3}",
        "{\"assignment\":[4,3,2,1,4,3,2,1,1],\"sample\":4}",
        "{\"assignment\":[3,2,2,4,1,3,1,4,3],\"sample\":5}",
        "{\"assignment\":[2,2,3,3,4,4,5,5,1],\"sample\":6}",
        "{\"assignment\":[2,4,1,5,2,4,3,1,3],\"sample\":7}"
    );

    let new_to_old_map: HashMap<usize, usize> = [
        (0, 2),
        (1, 3),
        (2, 4),
        (3, 5),
        (4, 6),
        (5, 7),
        (6, 8),
        (7, 0),
        (8, 1),
    ]
    .iter()
    .cloned()
    .collect();

    let input = file.as_bytes();

    let mut output = Vec::new();
    let writer = io::BufWriter::new(&mut output);

    encode_jsonl_to_ben(input, writer, BenVariant::Standard).unwrap();

    let mut output2 = Vec::new();
    let writer2 = io::BufWriter::new(&mut output2);
    relabel_ben_file_with_map(output.as_slice(), writer2, new_to_old_map).unwrap();

    let mut output3 = Vec::new();
    let writer3 = io::BufWriter::new(&mut output3);
    decode_ben_to_jsonl(output2.as_slice(), writer3).unwrap();

    let output_str = String::from_utf8(output3).unwrap();

    let out_file = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        "{\"assignment\":[3,4,5,5,3,4,2,1,2],\"sample\":1}",
        "{\"assignment\":[3,4,5,5,3,4,2,2,1],\"sample\":2}",
        "{\"assignment\":[1,1,2,2,3,3,4,3,3],\"sample\":3}",
        "{\"assignment\":[2,1,4,3,2,1,1,4,3],\"sample\":4}",
        "{\"assignment\":[2,4,1,3,1,4,3,3,2],\"sample\":5}",
        "{\"assignment\":[3,3,4,4,5,5,1,2,2],\"sample\":6}",
        "{\"assignment\":[1,5,2,4,3,1,3,2,4],\"sample\":7}"
    );

    assert_eq!(output_str, out_file);
}

#[test]
fn test_relabel_simple_file_with_map_mkv() {
    let file = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        "{\"assignment\":[1,2,3,4,5,5,3,4,2],\"sample\":1}",
        "{\"assignment\":[1,2,3,4,5,5,3,4,2],\"sample\":2}",
        "{\"assignment\":[1,2,3,4,5,5,3,4,2],\"sample\":3}",
        "{\"assignment\":[1,2,3,4,5,5,3,4,2],\"sample\":4}",
        "{\"assignment\":[1,2,3,4,5,5,3,4,2],\"sample\":5}",
        "{\"assignment\":[1,2,3,4,5,5,3,4,2],\"sample\":6}",
        "{\"assignment\":[2,1,3,4,5,5,3,4,2],\"sample\":7}",
        "{\"assignment\":[2,1,3,4,5,5,3,4,2],\"sample\":8}",
        "{\"assignment\":[2,1,3,4,5,5,3,4,2],\"sample\":9}",
        "{\"assignment\":[2,4,1,5,2,4,3,1,3],\"sample\":10}",
    );

    let new_to_old_map: HashMap<usize, usize> = [
        (0, 2),
        (1, 3),
        (2, 4),
        (3, 5),
        (4, 6),
        (5, 7),
        (6, 8),
        (7, 0),
        (8, 1),
    ]
    .iter()
    .cloned()
    .collect();

    let input = file.as_bytes();

    let mut output = Vec::new();
    let writer = io::BufWriter::new(&mut output);

    encode_jsonl_to_ben(input, writer, BenVariant::MkvChain).unwrap();

    let mut output2 = Vec::new();
    let writer2 = io::BufWriter::new(&mut output2);
    relabel_ben_file_with_map(output.as_slice(), writer2, new_to_old_map).unwrap();

    let mut output3 = Vec::new();
    let writer3 = io::BufWriter::new(&mut output3);
    decode_ben_to_jsonl(output2.as_slice(), writer3).unwrap();

    let output_str = String::from_utf8(output3).unwrap();

    let out_file = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        "{\"assignment\":[3,4,5,5,3,4,2,1,2],\"sample\":1}",
        "{\"assignment\":[3,4,5,5,3,4,2,1,2],\"sample\":2}",
        "{\"assignment\":[3,4,5,5,3,4,2,1,2],\"sample\":3}",
        "{\"assignment\":[3,4,5,5,3,4,2,1,2],\"sample\":4}",
        "{\"assignment\":[3,4,5,5,3,4,2,1,2],\"sample\":5}",
        "{\"assignment\":[3,4,5,5,3,4,2,1,2],\"sample\":6}",
        "{\"assignment\":[3,4,5,5,3,4,2,2,1],\"sample\":7}",
        "{\"assignment\":[3,4,5,5,3,4,2,2,1],\"sample\":8}",
        "{\"assignment\":[3,4,5,5,3,4,2,2,1],\"sample\":9}",
        "{\"assignment\":[1,5,2,4,3,1,3,2,4],\"sample\":10}",
    );

    assert_eq!(output_str, out_file);
}

#[test]
fn test_relabel_simple_file_with_map_twodelta() {
    let file = concat!(
        "{\"assignment\":[1,1,2,2,3,3],\"sample\":1}\n",
        "{\"assignment\":[1,1,2,2,3,3],\"sample\":2}\n",
        "{\"assignment\":[1,2,2,1,3,3],\"sample\":3}\n",
        "{\"assignment\":[2,2,1,1,3,3],\"sample\":4}\n"
    );

    let new_to_old_map: HashMap<usize, usize> = [(0, 2), (1, 3), (2, 0), (3, 1), (4, 4), (5, 5)]
        .iter()
        .cloned()
        .collect();

    let mut encoded = Vec::new();
    encode_jsonl_to_ben(
        file.as_bytes(),
        io::BufWriter::new(&mut encoded),
        BenVariant::TwoDelta,
    )
    .unwrap();

    let mut relabeled = Vec::new();
    relabel_ben_file_with_map(
        encoded.as_slice(),
        io::BufWriter::new(&mut relabeled),
        new_to_old_map,
    )
    .unwrap();

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(relabeled.as_slice(), io::BufWriter::new(&mut decoded)).unwrap();

    let output_str = String::from_utf8(decoded).unwrap();
    let expected = concat!(
        "{\"assignment\":[2,2,1,1,3,3],\"sample\":1}\n",
        "{\"assignment\":[2,2,1,1,3,3],\"sample\":2}\n",
        "{\"assignment\":[2,1,1,2,3,3],\"sample\":3}\n",
        "{\"assignment\":[1,1,2,2,3,3],\"sample\":4}\n"
    );
    assert_eq!(output_str, expected);
}

#[test]
fn test_relabel_simple_file_with_map_mkv_limit_truncates_counts() {
    let file = concat!(
        "{\"assignment\":[1,2,3],\"sample\":1}\n",
        "{\"assignment\":[1,2,3],\"sample\":2}\n",
        "{\"assignment\":[1,2,3],\"sample\":3}\n",
        "{\"assignment\":[3,1,2],\"sample\":4}\n"
    );

    let new_to_old_map: HashMap<usize, usize> = [(0, 1), (1, 2), (2, 0)].iter().cloned().collect();

    let mut encoded = Vec::new();
    encode_jsonl_to_ben(
        file.as_bytes(),
        io::BufWriter::new(&mut encoded),
        BenVariant::MkvChain,
    )
    .unwrap();

    let mut relabeled = Vec::new();
    relabel_ben_file_with_map_limit(
        encoded.as_slice(),
        io::BufWriter::new(&mut relabeled),
        new_to_old_map,
        2,
    )
    .unwrap();

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(relabeled.as_slice(), io::BufWriter::new(&mut decoded)).unwrap();

    let output_str = String::from_utf8(decoded).unwrap();
    let expected = concat!(
        "{\"assignment\":[2,3,1],\"sample\":1}\n",
        "{\"assignment\":[2,3,1],\"sample\":2}\n"
    );
    assert_eq!(output_str, expected);
}

#[test]
fn test_relabel_file_rejects_invalid_header() {
    let err = relabel_ben_file(b"not a valid banner".as_slice(), Vec::new()).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert_eq!(err.to_string(), "unrecognized BEN banner (got [110, 111, 116, 32, 97, 32, 118, 97, 108, 105, 100, 32, 98, 97, 110, 110, 101]; expected one of \"STANDARD BEN FILE\", \"MKVCHAIN BEN FILE\", or \"TWODELTA BEN FILE\")");
}

#[test]
fn test_relabel_file_with_map_rejects_invalid_header() {
    let err =
        relabel_ben_file_with_map(b"not a valid banner".as_slice(), Vec::new(), HashMap::new())
            .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert_eq!(err.to_string(), "unrecognized BEN banner (got [110, 111, 116, 32, 97, 32, 118, 97, 108, 105, 100, 32, 98, 97, 110, 110, 101]; expected one of \"STANDARD BEN FILE\", \"MKVCHAIN BEN FILE\", or \"TWODELTA BEN FILE\")");
}

#[test]
fn test_relabel_lines_propagate_non_eof_reader_error() {
    struct BoomReader {
        returned_first: bool,
    }

    impl io::Read for BoomReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.returned_first {
                return Err(io::Error::other("boom"));
            }
            self.returned_first = true;
            buf[0] = 1;
            Ok(1)
        }
    }

    let err = relabel_ben_lines(
        BoomReader {
            returned_first: false,
        },
        Vec::new(),
        BenVariant::Standard,
    )
    .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::Other);
}

#[test]
fn test_relabel_lines_with_map_propagate_non_eof_reader_error() {
    struct BoomReader {
        returned_first: bool,
    }

    impl io::Read for BoomReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.returned_first {
                return Err(io::Error::other("boom"));
            }
            self.returned_first = true;
            buf[0] = 1;
            Ok(1)
        }
    }

    let err = relabel_ben_lines_with_map(
        BoomReader {
            returned_first: false,
        },
        Vec::new(),
        HashMap::new(),
        BenVariant::Standard,
    )
    .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::Other);
}

#[test]
fn relabel_error_io_passthrough() {
    let inner = io::Error::new(io::ErrorKind::BrokenPipe, "pipe broke");
    let relabel_err = super::errors::RelabelError::Io(inner);
    let io_err: io::Error = relabel_err.into();
    assert_eq!(io_err.kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(io_err.to_string(), "pipe broke");
}

#[test]
fn relabel_error_non_io_becomes_invalid_input() {
    let relabel_err = super::errors::RelabelError::NonContiguousMap {
        max_key: 10,
        missing: 3,
    };
    let io_err: io::Error = relabel_err.into();
    assert_eq!(io_err.kind(), io::ErrorKind::InvalidInput);
    assert!(io_err.to_string().contains("contiguous"));
}
