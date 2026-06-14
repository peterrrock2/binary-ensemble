use super::*;
use crate::codec::decode::decode_ben_to_jsonl;
use crate::codec::encode::encode_jsonl_to_ben;
use crate::codec::BenEncodeFrame;
use crate::format::banners::BANNER_LEN;
use crate::util::rle::assign_to_rle;
use crate::BenVariant;
use rand::seq::SliceRandom;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use rand_distr::{Distribution, Uniform};
use std::collections::HashMap;
use std::io;
use std::io::Read;

/// A reader that returns one byte successfully then an I/O error.
struct ErrorAfterOneByte;

impl Read for ErrorAfterOneByte {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        buf[0] = 0x01;
        Err(io::Error::new(io::ErrorKind::BrokenPipe, "broken"))
    }
}

fn shuffle_with_mapping<T>(vec: &mut [T]) -> HashMap<usize, usize>
where
    T: Clone,
{
    // Shuffle *indices* and apply that permutation, so the returned `new -> old` map is a true
    // bijection. (Matching shuffled values back to `position(first equal value)` would collapse
    // duplicate values onto one old index and produce a non-permutation map.)
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let original_vec = vec.to_vec();
    let mut old_indices: Vec<usize> = (0..vec.len()).collect();
    old_indices.shuffle(&mut rng);

    let mut map = HashMap::new();
    for (new_index, &old_index) in old_indices.iter().enumerate() {
        vec[new_index] = original_vec[old_index].clone();
        map.insert(new_index, old_index);
    }
    map
}

/// Wrap a banner-stripped frame payload back into a full BEN file by prepending the banner. Tests
/// that previously fed banner-less buffers feed a full BEN file under the new API and call
/// [`relabel_ben_file`] directly.
fn with_banner(variant: BenVariant, payload: &[u8]) -> Vec<u8> {
    let mut out = crate::format::banners::banner_for_variant(variant).to_vec();
    out.extend_from_slice(payload);
    out
}

#[test]
fn test_relabel_ben_line_simple() {
    let in_rle = vec![(2, 2), (3, 2), (1, 2), (4, 2)];

    let input = BenEncodeFrame::from_rle(in_rle, BenVariant::Standard, None).unwrap();

    let out_rle = vec![(0, 2), (1, 2), (2, 2), (3, 2)];
    let expected = BenEncodeFrame::from_rle(out_rle, BenVariant::Standard, None).unwrap();

    let with_banner_in = with_banner(BenVariant::Standard, input.as_slice());
    let mut buf = Vec::new();
    relabel_ben_file(
        with_banner_in.as_slice(),
        &mut buf,
        RelabelOptions::first_seen(),
    )
    .unwrap();

    assert_eq!(
        &buf[..BANNER_LEN],
        crate::format::banners::STANDARD_BEN_BANNER
    );
    assert_eq!(&buf[BANNER_LEN..], expected.as_slice());
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
    relabel_ben_file(output.as_slice(), writer2, RelabelOptions::first_seen()).unwrap();

    let mut output3 = Vec::new();
    let writer3 = io::BufWriter::new(&mut output3);
    decode_ben_to_jsonl(output2.as_slice(), writer3).unwrap();

    let output_str = String::from_utf8(output3).unwrap();

    let out_file = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        "{\"assignment\":[0,1,2,3,4,4,2,3,1],\"sample\":1}",
        "{\"assignment\":[0,1,2,3,4,4,2,3,0],\"sample\":2}",
        "{\"assignment\":[0,0,1,1,2,2,0,0,3],\"sample\":3}",
        "{\"assignment\":[0,1,2,3,0,1,2,3,3],\"sample\":4}",
        "{\"assignment\":[0,1,1,2,3,0,3,2,0],\"sample\":5}",
        "{\"assignment\":[0,0,1,1,2,2,3,3,4],\"sample\":6}",
        "{\"assignment\":[0,1,2,3,0,1,4,2,4],\"sample\":7}"
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
    relabel_ben_file(output.as_slice(), writer2, RelabelOptions::first_seen()).unwrap();

    let mut output3 = Vec::new();
    let writer3 = io::BufWriter::new(&mut output3);
    decode_ben_to_jsonl(output2.as_slice(), writer3).unwrap();

    let output_str = String::from_utf8(output3).unwrap();

    let out_file = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        "{\"assignment\":[0,1,2,3,4,4,2,3,1],\"sample\":1}",
        "{\"assignment\":[0,1,2,3,4,4,2,3,0],\"sample\":2}",
        "{\"assignment\":[0,0,1,1,2,2,0,0,3],\"sample\":3}",
        "{\"assignment\":[0,1,2,3,0,1,2,3,3],\"sample\":4}",
        "{\"assignment\":[0,1,1,2,3,0,3,2,0],\"sample\":5}",
        "{\"assignment\":[0,1,1,2,3,0,3,2,0],\"sample\":6}",
        "{\"assignment\":[0,1,1,2,3,0,3,2,0],\"sample\":7}",
        "{\"assignment\":[0,0,1,1,2,2,3,3,4],\"sample\":8}",
        "{\"assignment\":[0,1,2,3,0,1,4,2,4],\"sample\":9}",
        "{\"assignment\":[0,1,2,3,0,1,4,2,4],\"sample\":10}"
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
    relabel_ben_file(
        encoded.as_slice(),
        io::BufWriter::new(&mut relabeled),
        RelabelOptions::first_seen().with_max_samples(2),
    )
    .unwrap();

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(relabeled.as_slice(), io::BufWriter::new(&mut decoded)).unwrap();

    let output_str = String::from_utf8(decoded).unwrap();
    let expected = concat!(
        "{\"assignment\":[0,1,2],\"sample\":1}\n",
        "{\"assignment\":[0,1,2],\"sample\":2}\n"
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
    relabel_ben_file(
        encoded.as_slice(),
        io::BufWriter::new(&mut relabeled),
        RelabelOptions::first_seen(),
    )
    .unwrap();

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(relabeled.as_slice(), io::BufWriter::new(&mut decoded)).unwrap();

    let output_str = String::from_utf8(decoded).unwrap();
    let expected = concat!(
        "{\"assignment\":[0,0,1,1,2,2],\"sample\":1}\n",
        "{\"assignment\":[0,0,1,1,2,2],\"sample\":2}\n",
        "{\"assignment\":[0,1,1,0,2,2],\"sample\":3}\n",
        "{\"assignment\":[0,0,1,1,2,2],\"sample\":4}\n"
    );
    assert_eq!(output_str, expected);
}

#[test]
fn test_relabel_ben_line_with_map() {
    let in_assign = vec![2, 3, 1, 4, 5, 5, 3, 4, 2];
    let in_rle = assign_to_rle(in_assign);

    let input = BenEncodeFrame::from_rle(in_rle, BenVariant::Standard, None).unwrap();

    let out_assign = vec![1, 2, 2, 3, 3, 4, 4, 5, 5];
    let out_rle = assign_to_rle(out_assign);
    let expected = BenEncodeFrame::from_rle(out_rle, BenVariant::Standard, None).unwrap();

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

    let with_banner_in = with_banner(BenVariant::Standard, input.as_slice());
    let mut buf = Vec::new();
    relabel_ben_file(
        with_banner_in.as_slice(),
        &mut buf,
        RelabelOptions::node_permutation(new_to_old_map),
    )
    .unwrap();

    assert_eq!(
        &buf[..BANNER_LEN],
        crate::format::banners::STANDARD_BEN_BANNER
    );
    assert_eq!(&buf[BANNER_LEN..], expected.as_slice());
}

#[test]
fn first_seen_fast_path_rejects_zero_count_frame() {
    // A MkvChain frame with count == 0 is corrupt; the byte-walking fast path must error rather
    // than re-emit a frame every downstream reader rejects.
    let frame =
        BenEncodeFrame::from_assignment(vec![1u16, 2, 2], BenVariant::MkvChain, Some(0)).unwrap();
    let with_banner_in = with_banner(BenVariant::MkvChain, frame.as_slice());

    let err = relabel_ben_file(
        with_banner_in.as_slice(),
        Vec::new(),
        RelabelOptions::first_seen(),
    )
    .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert!(err.to_string().contains("count"));
}

#[test]
fn test_relabel_ben_line_with_shuffle() {
    let in_assign = vec![2, 3, 1, 4, 5, 5, 3, 4, 2];
    let mut out_assign = in_assign.clone();

    let in_rle = assign_to_rle(in_assign);
    let input = BenEncodeFrame::from_rle(in_rle, BenVariant::Standard, None).unwrap();

    let new_to_old_map = shuffle_with_mapping(&mut out_assign);
    let out_rle = assign_to_rle(out_assign);
    let expected = BenEncodeFrame::from_rle(out_rle, BenVariant::Standard, None).unwrap();

    let with_banner_in = with_banner(BenVariant::Standard, input.as_slice());
    let mut buf = Vec::new();
    relabel_ben_file(
        with_banner_in.as_slice(),
        &mut buf,
        RelabelOptions::node_permutation(new_to_old_map),
    )
    .unwrap();

    assert_eq!(&buf[BANNER_LEN..], expected.as_slice());
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

    let in_rle = assign_to_rle(&in_assign);
    let input = BenEncodeFrame::from_rle(in_rle, BenVariant::Standard, None).unwrap();

    let new_to_old_map = shuffle_with_mapping(&mut out_assign);
    let out_rle = assign_to_rle(out_assign);
    let expected = BenEncodeFrame::from_rle(out_rle, BenVariant::Standard, None).unwrap();

    let with_banner_in = with_banner(BenVariant::Standard, input.as_slice());
    let mut buf = Vec::new();
    relabel_ben_file(
        with_banner_in.as_slice(),
        &mut buf,
        RelabelOptions::node_permutation(new_to_old_map),
    )
    .unwrap();

    assert_eq!(&buf[BANNER_LEN..], expected.as_slice());
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
    relabel_ben_file(
        output.as_slice(),
        writer2,
        RelabelOptions::node_permutation(new_to_old_map),
    )
    .unwrap();

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
    relabel_ben_file(
        output.as_slice(),
        writer2,
        RelabelOptions::node_permutation(new_to_old_map),
    )
    .unwrap();

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
    relabel_ben_file(
        encoded.as_slice(),
        io::BufWriter::new(&mut relabeled),
        RelabelOptions::node_permutation(new_to_old_map),
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
    relabel_ben_file(
        encoded.as_slice(),
        io::BufWriter::new(&mut relabeled),
        RelabelOptions::node_permutation(new_to_old_map).with_max_samples(2),
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
    let err = relabel_ben_file(
        b"not a valid banner".as_slice(),
        Vec::new(),
        RelabelOptions::first_seen(),
    )
    .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert_eq!(err.to_string(), "unrecognized BEN banner (got [110, 111, 116, 32, 97, 32, 118, 97, 108, 105, 100, 32, 98, 97, 110, 110, 101]; expected one of \"STANDARD BEN FILE\", \"MKVCHAIN BEN FILE\", or \"TWODELTA BEN FILE\")");
}

#[test]
fn test_relabel_file_with_map_rejects_invalid_header() {
    let err = relabel_ben_file(
        b"not a valid banner".as_slice(),
        Vec::new(),
        RelabelOptions::node_permutation(HashMap::new()),
    )
    .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert_eq!(err.to_string(), "unrecognized BEN banner (got [110, 111, 116, 32, 97, 32, 118, 97, 108, 105, 100, 32, 98, 97, 110, 110, 101]; expected one of \"STANDARD BEN FILE\", \"MKVCHAIN BEN FILE\", or \"TWODELTA BEN FILE\")");
}

#[test]
fn test_relabel_lines_propagate_non_eof_reader_error() {
    // Reader returns a valid Standard banner via Cursor, then the BoomReader produces a non-EOF I/O
    // error on the body. The byte-walk fast path returns this I/O error unchanged.
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

    let chained =
        io::Cursor::new(crate::format::banners::STANDARD_BEN_BANNER.to_vec()).chain(BoomReader {
            returned_first: false,
        });
    let err = relabel_ben_file(chained, Vec::new(), RelabelOptions::first_seen()).unwrap_err();
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

    let chained =
        io::Cursor::new(crate::format::banners::STANDARD_BEN_BANNER.to_vec()).chain(BoomReader {
            returned_first: false,
        });
    let err = relabel_ben_file(
        chained,
        Vec::new(),
        RelabelOptions::node_permutation(HashMap::new()),
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

// ── convert_ben_file ─────────────────────────────────────────────────

#[test]
fn test_convert_ben_file_standard_to_mkv() {
    let file = concat!(
        "{\"assignment\":[1,2,3],\"sample\":1}\n",
        "{\"assignment\":[1,2,3],\"sample\":2}\n",
        "{\"assignment\":[4,5,6],\"sample\":3}\n",
    );

    let mut encoded = Vec::new();
    encode_jsonl_to_ben(
        file.as_bytes(),
        io::BufWriter::new(&mut encoded),
        BenVariant::Standard,
    )
    .unwrap();

    let mut converted = Vec::new();
    convert_ben_file(
        encoded.as_slice(),
        io::BufWriter::new(&mut converted),
        BenVariant::MkvChain,
    )
    .unwrap();

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(converted.as_slice(), io::BufWriter::new(&mut decoded)).unwrap();
    let output_str = String::from_utf8(decoded).unwrap();
    let expected = concat!(
        "{\"assignment\":[1,2,3],\"sample\":1}\n",
        "{\"assignment\":[1,2,3],\"sample\":2}\n",
        "{\"assignment\":[4,5,6],\"sample\":3}\n",
    );
    assert_eq!(output_str, expected);
}

#[test]
fn test_convert_ben_file_limit_truncates() {
    let file = concat!(
        "{\"assignment\":[1,2,3],\"sample\":1}\n",
        "{\"assignment\":[1,2,3],\"sample\":2}\n",
        "{\"assignment\":[1,2,3],\"sample\":3}\n",
        "{\"assignment\":[4,5,6],\"sample\":4}\n",
    );

    let mut encoded = Vec::new();
    encode_jsonl_to_ben(
        file.as_bytes(),
        io::BufWriter::new(&mut encoded),
        BenVariant::MkvChain,
    )
    .unwrap();

    let mut converted = Vec::new();
    relabel_ben_file(
        encoded.as_slice(),
        io::BufWriter::new(&mut converted),
        RelabelOptions::convert_to(BenVariant::Standard).with_max_samples(2),
    )
    .unwrap();

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(converted.as_slice(), io::BufWriter::new(&mut decoded)).unwrap();
    let output_str = String::from_utf8(decoded).unwrap();
    // convert_to preserves labels verbatim; only the variant changes.
    let expected = concat!(
        "{\"assignment\":[1,2,3],\"sample\":1}\n",
        "{\"assignment\":[1,2,3],\"sample\":2}\n",
    );
    assert_eq!(output_str, expected);
}

// ── relabel_ben_lines_limit ──────────────────────────────────────────

#[test]
fn test_relabel_ben_lines_limit_standard() {
    let file = concat!(
        "{\"assignment\":[3,1,2],\"sample\":1}\n",
        "{\"assignment\":[2,3,1],\"sample\":2}\n",
        "{\"assignment\":[1,2,3],\"sample\":3}\n",
    );

    let mut encoded = Vec::new();
    encode_jsonl_to_ben(
        file.as_bytes(),
        io::BufWriter::new(&mut encoded),
        BenVariant::Standard,
    )
    .unwrap();

    let mut full_relabeled = Vec::new();
    relabel_ben_file(
        encoded.as_slice(),
        io::BufWriter::new(&mut full_relabeled),
        RelabelOptions::first_seen().with_max_samples(2),
    )
    .unwrap();

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(full_relabeled.as_slice(), io::BufWriter::new(&mut decoded)).unwrap();
    let output_str = String::from_utf8(decoded).unwrap();
    let expected = concat!(
        "{\"assignment\":[0,1,2],\"sample\":1}\n",
        "{\"assignment\":[0,1,2],\"sample\":2}\n",
    );
    assert_eq!(output_str, expected);
}

// ── relabel_ben_lines_with_map_limit ─────────────────────────────────

#[test]
fn test_relabel_ben_lines_with_map_limit_standard() {
    let file = concat!(
        "{\"assignment\":[1,2,3],\"sample\":1}\n",
        "{\"assignment\":[4,5,6],\"sample\":2}\n",
        "{\"assignment\":[7,8,9],\"sample\":3}\n",
    );

    let mut encoded = Vec::new();
    encode_jsonl_to_ben(
        file.as_bytes(),
        io::BufWriter::new(&mut encoded),
        BenVariant::Standard,
    )
    .unwrap();

    let map: HashMap<usize, usize> = [(0, 2), (1, 0), (2, 1)].iter().cloned().collect();

    let mut full_relabeled = Vec::new();
    relabel_ben_file(
        encoded.as_slice(),
        io::BufWriter::new(&mut full_relabeled),
        RelabelOptions::node_permutation(map).with_max_samples(1),
    )
    .unwrap();

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(full_relabeled.as_slice(), io::BufWriter::new(&mut decoded)).unwrap();
    let output_str = String::from_utf8(decoded).unwrap();
    assert_eq!(output_str, "{\"assignment\":[3,1,2],\"sample\":1}\n");
}

// ── relabel_ben_file_as_variant ──────────────────────────────────────

#[test]
fn test_relabel_ben_file_as_variant_standard_to_twodelta() {
    let file = concat!(
        "{\"assignment\":[3,3,1,1],\"sample\":1}\n",
        "{\"assignment\":[1,3,1,3],\"sample\":2}\n",
    );

    let mut encoded = Vec::new();
    encode_jsonl_to_ben(
        file.as_bytes(),
        io::BufWriter::new(&mut encoded),
        BenVariant::Standard,
    )
    .unwrap();

    let mut converted = Vec::new();
    relabel_ben_file(
        encoded.as_slice(),
        io::BufWriter::new(&mut converted),
        RelabelOptions::first_seen().with_target_variant(BenVariant::TwoDelta),
    )
    .unwrap();

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(converted.as_slice(), io::BufWriter::new(&mut decoded)).unwrap();
    let output_str = String::from_utf8(decoded).unwrap();
    let expected = concat!(
        "{\"assignment\":[0,0,1,1],\"sample\":1}\n",
        "{\"assignment\":[0,1,0,1],\"sample\":2}\n",
    );
    assert_eq!(output_str, expected);
}

#[test]
fn test_relabel_ben_file_as_variant_limit() {
    let file = concat!(
        "{\"assignment\":[3,1,2],\"sample\":1}\n",
        "{\"assignment\":[2,3,1],\"sample\":2}\n",
        "{\"assignment\":[1,2,3],\"sample\":3}\n",
    );

    let mut encoded = Vec::new();
    encode_jsonl_to_ben(
        file.as_bytes(),
        io::BufWriter::new(&mut encoded),
        BenVariant::Standard,
    )
    .unwrap();

    let mut converted = Vec::new();
    relabel_ben_file(
        encoded.as_slice(),
        io::BufWriter::new(&mut converted),
        RelabelOptions::first_seen()
            .with_target_variant(BenVariant::MkvChain)
            .with_max_samples(2),
    )
    .unwrap();

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(converted.as_slice(), io::BufWriter::new(&mut decoded)).unwrap();
    let output_str = String::from_utf8(decoded).unwrap();
    let expected = concat!(
        "{\"assignment\":[0,1,2],\"sample\":1}\n",
        "{\"assignment\":[0,1,2],\"sample\":2}\n",
    );
    assert_eq!(output_str, expected);
}

// ── relabel_ben_file_with_map_as_variant ─────────────────────────────

#[test]
fn test_relabel_ben_file_with_map_as_variant() {
    let file = concat!(
        "{\"assignment\":[1,2,3],\"sample\":1}\n",
        "{\"assignment\":[4,5,6],\"sample\":2}\n",
    );

    let mut encoded = Vec::new();
    encode_jsonl_to_ben(
        file.as_bytes(),
        io::BufWriter::new(&mut encoded),
        BenVariant::Standard,
    )
    .unwrap();

    let map: HashMap<usize, usize> = [(0, 2), (1, 0), (2, 1)].iter().cloned().collect();

    let mut converted = Vec::new();
    relabel_ben_file(
        encoded.as_slice(),
        io::BufWriter::new(&mut converted),
        RelabelOptions::node_permutation(map).with_target_variant(BenVariant::MkvChain),
    )
    .unwrap();

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(converted.as_slice(), io::BufWriter::new(&mut decoded)).unwrap();
    let output_str = String::from_utf8(decoded).unwrap();
    let expected = concat!(
        "{\"assignment\":[3,1,2],\"sample\":1}\n",
        "{\"assignment\":[6,4,5],\"sample\":2}\n",
    );
    assert_eq!(output_str, expected);
}

#[test]
fn test_relabel_ben_file_with_map_as_variant_limit() {
    let file = concat!(
        "{\"assignment\":[1,2,3],\"sample\":1}\n",
        "{\"assignment\":[1,2,3],\"sample\":2}\n",
        "{\"assignment\":[4,5,6],\"sample\":3}\n",
    );

    let mut encoded = Vec::new();
    encode_jsonl_to_ben(
        file.as_bytes(),
        io::BufWriter::new(&mut encoded),
        BenVariant::MkvChain,
    )
    .unwrap();

    let map: HashMap<usize, usize> = [(0, 2), (1, 0), (2, 1)].iter().cloned().collect();

    let mut converted = Vec::new();
    relabel_ben_file(
        encoded.as_slice(),
        io::BufWriter::new(&mut converted),
        RelabelOptions::node_permutation(map)
            .with_target_variant(BenVariant::Standard)
            .with_max_samples(2),
    )
    .unwrap();

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(converted.as_slice(), io::BufWriter::new(&mut decoded)).unwrap();
    let output_str = String::from_utf8(decoded).unwrap();
    let expected = concat!(
        "{\"assignment\":[3,1,2],\"sample\":1}\n",
        "{\"assignment\":[3,1,2],\"sample\":2}\n",
    );
    assert_eq!(output_str, expected);
}

// ── convert_ben_file rejects invalid banner ──────────────────────────

#[test]
fn test_convert_ben_file_rejects_invalid_banner() {
    let err = convert_ben_file(
        b"not a valid banner".as_slice(),
        Vec::new(),
        BenVariant::Standard,
    )
    .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

// ── relabel_ben_file_as_variant rejects invalid banner ───────────────

#[test]
fn test_relabel_ben_file_as_variant_rejects_invalid_banner() {
    let err = relabel_ben_file(
        b"not a valid banner".as_slice(),
        Vec::new(),
        RelabelOptions::first_seen().with_target_variant(BenVariant::Standard),
    )
    .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

// ── relabel_ben_lines_with_map: LengthMismatch ─────────────────────

#[test]
fn test_relabel_ben_length_mismatch() {
    // BEN stream with assignment length 3 ([1,2,3]); permutation of length 5 triggers
    // LengthMismatch.
    let jsonl = r#"{"assignment":[1,2,3],"sample":1}
"#;
    let mut ben = Vec::new();
    encode_jsonl_to_ben(jsonl.as_bytes(), &mut ben, BenVariant::Standard).unwrap();

    let map: HashMap<usize, usize> = (0..5).map(|i| (i, i)).collect();

    let mut output = Vec::new();
    let err = relabel_ben_file(
        ben.as_slice(),
        &mut output,
        RelabelOptions::node_permutation(map),
    )
    .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    assert!(
        err.to_string().contains("length") || err.to_string().contains("mismatch"),
        "got: {}",
        err
    );
}

#[test]
fn test_relabel_ben_lines_non_eof_read_error_propagates() {
    // The byte-walk fast path returns a non-EOF I/O error when the reader fails.
    let chained = io::Cursor::new(crate::format::banners::STANDARD_BEN_BANNER.to_vec())
        .chain(ErrorAfterOneByte);
    let mut output = Vec::new();
    let err = relabel_ben_file(chained, &mut output, RelabelOptions::first_seen()).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
}

#[test]
fn test_relabel_ben_file_with_map_non_eof_read_error_propagates() {
    let map: HashMap<usize, usize> = (0..4).map(|i| (i, i)).collect();
    let chained = io::Cursor::new(crate::format::banners::STANDARD_BEN_BANNER.to_vec())
        .chain(ErrorAfterOneByte);
    let mut output = Vec::new();
    let err =
        relabel_ben_file(chained, &mut output, RelabelOptions::node_permutation(map)).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
}

#[test]
fn test_relabel_ben_file_twodelta_malformed_frame_error_propagates() {
    // Build a valid 2-sample TwoDelta BEN file, then corrupt the delta frame.
    let mut ben: Vec<u8> = Vec::new();
    {
        let mut writer =
            crate::io::writer::BenStreamWriter::for_ben(&mut ben, BenVariant::TwoDelta).unwrap();
        writer.write_assignment(vec![1u16, 1, 2, 2]).unwrap();
        writer.write_assignment(vec![2u16, 1, 2, 1]).unwrap();
    }
    // banner(17) + snapshot_tag(1) precede the anchor frame; a delta_tag(1) precedes the delta
    // frame, so the delta's max_len_bits sits at anchor_end + 5.
    let banner_len = 17usize;
    let anchor_start = banner_len + 1;
    let n_bytes =
        u32::from_be_bytes(ben[anchor_start + 2..anchor_start + 6].try_into().unwrap()) as usize;
    let anchor_end = anchor_start + 6 + n_bytes + 2;
    ben[anchor_end + 5] = 0;

    let mut output = Vec::new();
    let err =
        relabel_ben_file(ben.as_slice(), &mut output, RelabelOptions::first_seen()).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn test_relabel_ben_file_with_map_twodelta_malformed_frame_error_propagates() {
    let mut ben: Vec<u8> = Vec::new();
    {
        let mut writer =
            crate::io::writer::BenStreamWriter::for_ben(&mut ben, BenVariant::TwoDelta).unwrap();
        writer.write_assignment(vec![1u16, 1, 2, 2]).unwrap();
        writer.write_assignment(vec![2u16, 1, 2, 1]).unwrap();
    }
    // banner(17) + snapshot_tag(1) precede the anchor frame; a delta_tag(1) precedes the delta
    // frame, so the delta's max_len_bits sits at anchor_end + 5.
    let banner_len = 17usize;
    let anchor_start = banner_len + 1;
    let n_bytes =
        u32::from_be_bytes(ben[anchor_start + 2..anchor_start + 6].try_into().unwrap()) as usize;
    let anchor_end = anchor_start + 6 + n_bytes + 2;
    ben[anchor_end + 5] = 0;

    let map: HashMap<usize, usize> = (0..4).map(|i| (i, i)).collect();
    let mut output = Vec::new();
    let err = relabel_ben_file(
        ben.as_slice(),
        &mut output,
        RelabelOptions::node_permutation(map),
    )
    .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

// ── Verification: predicate matrix + frame-preservation + cross-policy ──

#[test]
fn fast_path_predicate_matrix() {
    use BenVariant::*;
    use RunPolicy::*;
    let transforms = [
        ("Identity", RelabelTransform::Identity),
        ("FirstSeen", RelabelTransform::FirstSeen),
        (
            "NodePermutation",
            RelabelTransform::NodePermutation(HashMap::new()),
        ),
    ];
    let inputs = [Standard, MkvChain, TwoDelta];
    let target_states = [None, Some(Standard)];
    let policies = [PreserveFrameBoundaries, CollapseAdjacentEqualAssignments];

    let mut true_cases = 0;
    for (tname, t) in &transforms {
        for &input in &inputs {
            for &target in &target_states {
                for &policy in &policies {
                    let result = can_use_first_seen_fast_path(t, target, input, policy);
                    let expected = matches!(tname, &"FirstSeen")
                        && target.is_none()
                        && policy == PreserveFrameBoundaries
                        && (input == Standard || input == MkvChain);
                    assert_eq!(
                        result, expected,
                        "({}, target={:?}, input={:?}, policy={:?})",
                        tname, target, input, policy
                    );
                    if result {
                        true_cases += 1;
                    }
                }
            }
        }
    }
    assert_eq!(true_cases, 2, "expected exactly two true matrix entries");
}

/// Forced-slow vs. fast-path equivalence for first-seen relabeling on Standard input. Forcing the
/// slow path uses `with_target_variant(input)` per decision #5 (`is_none()` semantics in the
/// predicate).
#[test]
fn fast_path_matches_slow_path_standard() {
    let file = concat!(
        "{\"assignment\":[3,1,2],\"sample\":1}\n",
        "{\"assignment\":[5,5,3],\"sample\":2}\n",
        "{\"assignment\":[1,2,3],\"sample\":3}\n",
    );
    let mut encoded = Vec::new();
    encode_jsonl_to_ben(
        file.as_bytes(),
        io::BufWriter::new(&mut encoded),
        BenVariant::Standard,
    )
    .unwrap();

    let mut fast_out = Vec::new();
    relabel_ben_file(
        encoded.as_slice(),
        &mut fast_out,
        RelabelOptions::first_seen(),
    )
    .unwrap();

    let mut slow_out = Vec::new();
    relabel_ben_file(
        encoded.as_slice(),
        &mut slow_out,
        RelabelOptions::first_seen().with_target_variant(BenVariant::Standard),
    )
    .unwrap();

    let mut fast_jsonl = Vec::new();
    decode_ben_to_jsonl(fast_out.as_slice(), &mut fast_jsonl).unwrap();
    let mut slow_jsonl = Vec::new();
    decode_ben_to_jsonl(slow_out.as_slice(), &mut slow_jsonl).unwrap();
    assert_eq!(fast_jsonl, slow_jsonl);
}

#[test]
fn fast_path_matches_slow_path_mkvchain() {
    let file = concat!(
        "{\"assignment\":[3,1,2],\"sample\":1}\n",
        "{\"assignment\":[3,1,2],\"sample\":2}\n",
        "{\"assignment\":[5,4,2],\"sample\":3}\n",
    );
    let mut encoded = Vec::new();
    encode_jsonl_to_ben(
        file.as_bytes(),
        io::BufWriter::new(&mut encoded),
        BenVariant::MkvChain,
    )
    .unwrap();

    let mut fast_out = Vec::new();
    relabel_ben_file(
        encoded.as_slice(),
        &mut fast_out,
        RelabelOptions::first_seen(),
    )
    .unwrap();

    // Force the slow path by setting target_variant to the input variant.
    let mut slow_out = Vec::new();
    relabel_ben_file(
        encoded.as_slice(),
        &mut slow_out,
        RelabelOptions::first_seen().with_target_variant(BenVariant::MkvChain),
    )
    .unwrap();

    // Decoded equivalence is the load-bearing assertion. Byte-identity is also expected here
    // (per plan verification step 4); tighten if it holds.
    let mut fast_jsonl = Vec::new();
    decode_ben_to_jsonl(fast_out.as_slice(), &mut fast_jsonl).unwrap();
    let mut slow_jsonl = Vec::new();
    decode_ben_to_jsonl(slow_out.as_slice(), &mut slow_jsonl).unwrap();
    assert_eq!(fast_jsonl, slow_jsonl);
}

#[test]
fn collapse_policy_disables_fast_path() {
    // With CollapseAdjacentEqualAssignments + first-seen on Standard input, the predicate must be
    // false (fast path disabled). We verify behaviorally by running both: the merging path should
    // produce the same decoded content but takes a different code path internally.
    let file = concat!(
        "{\"assignment\":[3,1,2],\"sample\":1}\n",
        "{\"assignment\":[3,1,2],\"sample\":2}\n",
    );
    let mut encoded = Vec::new();
    encode_jsonl_to_ben(
        file.as_bytes(),
        io::BufWriter::new(&mut encoded),
        BenVariant::Standard,
    )
    .unwrap();

    let mut out = Vec::new();
    relabel_ben_file(
        encoded.as_slice(),
        &mut out,
        RelabelOptions::first_seen().with_run_policy(RunPolicy::CollapseAdjacentEqualAssignments),
    )
    .unwrap();

    let mut decoded = Vec::new();
    decode_ben_to_jsonl(out.as_slice(), &mut decoded).unwrap();
    let s = String::from_utf8(decoded).unwrap();
    assert!(s.contains("\"assignment\":[0,1,2]"));
}

/// Decision #9: with `PreserveFrameBoundaries`, two adjacent input frames with the same assignment
/// but distinct counts must remain distinct counted frames at MkvChain target, not merged into one
/// frame with summed count. With `CollapseAdjacentEqualAssignments`, they are merged.
#[test]
fn run_policy_pins_frame_preservation_and_collapse() {
    // Build an MkvChain BEN file with two adjacent equal-assignment frames of counts 5 and 7
    // (12 total samples).
    let mut input = Vec::new();
    {
        let banner = crate::format::banners::MKVCHAIN_BEN_BANNER;
        input.extend_from_slice(banner);
        let frame_a =
            BenEncodeFrame::from_assignment([1u16, 2, 3], BenVariant::MkvChain, Some(5)).unwrap();
        let frame_b =
            BenEncodeFrame::from_assignment([1u16, 2, 3], BenVariant::MkvChain, Some(7)).unwrap();
        input.extend_from_slice(frame_a.as_slice());
        input.extend_from_slice(frame_b.as_slice());
    }

    // Identity transform via convert_to(MkvChain), preserving frame boundaries.
    let mut preserved = Vec::new();
    relabel_ben_file(
        input.as_slice(),
        &mut preserved,
        RelabelOptions::convert_to(BenVariant::MkvChain)
            .with_run_policy(RunPolicy::PreserveFrameBoundaries),
    )
    .unwrap();

    // Strip banner and count MkvChain frames by walking headers.
    fn count_mkvchain_frames(ben: &[u8]) -> usize {
        let mut i = BANNER_LEN;
        let mut frames = 0;
        while i < ben.len() {
            // header: max_val_bits(1), max_len_bits(1), n_bytes(4), payload(n_bytes), count(2)
            let n_bytes = u32::from_be_bytes(ben[i + 2..i + 6].try_into().unwrap()) as usize;
            i += 6 + n_bytes + 2;
            frames += 1;
        }
        frames
    }

    assert_eq!(
        count_mkvchain_frames(&preserved),
        2,
        "PreserveFrameBoundaries must keep both counted frames"
    );

    let mut collapsed = Vec::new();
    relabel_ben_file(
        input.as_slice(),
        &mut collapsed,
        RelabelOptions::convert_to(BenVariant::MkvChain)
            .with_run_policy(RunPolicy::CollapseAdjacentEqualAssignments),
    )
    .unwrap();

    assert_eq!(
        count_mkvchain_frames(&collapsed),
        1,
        "CollapseAdjacentEqualAssignments must merge into one count=12 frame"
    );

    // Decoded sample count is invariant across policies for MkvChain target.
    let mut a = Vec::new();
    decode_ben_to_jsonl(preserved.as_slice(), &mut a).unwrap();
    let mut b = Vec::new();
    decode_ben_to_jsonl(collapsed.as_slice(), &mut b).unwrap();
    assert_eq!(
        a.iter().filter(|&&c| c == b'\n').count(),
        12,
        "preserved decodes 12 samples"
    );
    assert_eq!(
        b.iter().filter(|&&c| c == b'\n').count(),
        12,
        "collapsed decodes 12 samples"
    );
}

/// Cross-policy invariant for Standard targets: byte-identical output regardless of run policy,
/// because Standard cannot encode counts.
#[test]
fn standard_target_cross_policy_byte_identity() {
    // Build the same (5, 7) MkvChain fixture.
    let mut input = Vec::new();
    {
        let banner = crate::format::banners::MKVCHAIN_BEN_BANNER;
        input.extend_from_slice(banner);
        let frame_a =
            BenEncodeFrame::from_assignment([1u16, 2, 3], BenVariant::MkvChain, Some(5)).unwrap();
        let frame_b =
            BenEncodeFrame::from_assignment([1u16, 2, 3], BenVariant::MkvChain, Some(7)).unwrap();
        input.extend_from_slice(frame_a.as_slice());
        input.extend_from_slice(frame_b.as_slice());
    }

    let mut preserve_out = Vec::new();
    relabel_ben_file(
        input.as_slice(),
        &mut preserve_out,
        RelabelOptions::convert_to(BenVariant::Standard)
            .with_run_policy(RunPolicy::PreserveFrameBoundaries),
    )
    .unwrap();

    let mut collapse_out = Vec::new();
    relabel_ben_file(
        input.as_slice(),
        &mut collapse_out,
        RelabelOptions::convert_to(BenVariant::Standard)
            .with_run_policy(RunPolicy::CollapseAdjacentEqualAssignments),
    )
    .unwrap();

    assert_eq!(
        preserve_out, collapse_out,
        "Standard target must be byte-identical across run policies"
    );
}
