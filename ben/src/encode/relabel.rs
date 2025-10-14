//! This module contains the main functions that are used in the `reben` binary
//! for relabeling the assignment vectors in a BEN file. The relabeling is done
//! can be doe either so that the values are in ascending order or according to
//! a mapping provided by the user in a map file.

use crate::decode::*;
use crate::encode::*;
use byteorder::{BigEndian, ReadBytesExt};
use std::collections::HashMap;
use std::io::Error;

/// Relabels each of the assignment vectors in a BEN file so that the values are
/// in ascending order.
///
/// # Arguments
///
/// * `reader` - A reader that implements the `Read` trait containing the BEN file to
/// be relabeled.
/// * `writer` - A writer that implements the `Write` trait and which will contain the
/// relabeled BEN file.
///
/// # Errors
///
/// Returns an error if the file format is invalid or if there is an issue reading or writing
/// the file.
pub fn relabel_ben_lines<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    variant: BenVariant,
) -> io::Result<()> {
    let mut sample_number = 0;
    loop {
        let mut tmp_buffer = [0u8];
        let max_val_bits = match reader.read_exact(&mut tmp_buffer) {
            Ok(_) => tmp_buffer[0],
            Err(e) => {
                if e.kind() == io::ErrorKind::UnexpectedEof {
                    break;
                }
                return Err(e);
            }
        };

        let max_len_bits = reader.read_u8()?;
        let n_bytes = reader.read_u32::<BigEndian>()?;

        let mut ben_line = decode_ben_line(&mut reader, max_val_bits, max_len_bits, n_bytes)?;

        // relabel the line
        let mut label = 0;
        let mut label_map = HashMap::new();
        for (val, _len) in ben_line.iter_mut() {
            let new_val = match label_map.get(val) {
                Some(v) => *v,
                None => {
                    label += 1;
                    label_map.insert(*val, label);
                    label
                }
            };
            *val = new_val;
        }

        let relabeled = encode_ben_vec_from_rle(ben_line);
        writer.write_all(&relabeled)?;

        let count_occurrences = if variant == BenVariant::MkvChain {
            let count = reader.read_u16::<BigEndian>()?;
            writer.write_all(&count.to_be_bytes())?;
            count
        } else {
            1
        };

        sample_number += count_occurrences as usize;

        log!("Relabeling line: {}\r", sample_number);
    }
    logln!();
    logln!("Done!");

    Ok(())
}

/// Relabels the values in a BEN file so that the assignment vector values are
/// in ascending order. So , if the assignment vector is [2, 3, 1, 4, 5, 5, 3, 4, 2]
/// the relabeled assignment vector will be [1, 2, 3, 4, 5, 5, 2, 4, 1].
///
/// # Arguments
///
/// * `reader` - A reader that implements the `Read` trait containing the BEN file to
/// be relabeled.
/// * `writer` - A writer that implements the `Write` trait and which will contain the
/// relabeled BEN file.
///
/// # Errors
///
/// Returns an error if the file format is invalid or if there is an issue reading or writing
/// the file.
pub fn relabel_ben_file<R: Read, W: Write>(mut reader: R, mut writer: W) -> io::Result<()> {
    let mut check_buffer = [0u8; 17];
    reader.read_exact(&mut check_buffer)?;

    let variant = match &check_buffer {
        b"STANDARD BEN FILE" => BenVariant::Standard,
        b"MKVCHAIN BEN FILE" => BenVariant::MkvChain,
        _ => {
            return Err(Error::new(
                io::ErrorKind::InvalidData,
                "Invalid file format",
            ));
        }
    };

    writer.write_all(&check_buffer)?;

    relabel_ben_lines(&mut reader, &mut writer, variant)?;

    Ok(())
}

/// Relabels the values in a BEN file so that the assignment vector values are
/// sorted according to a mapping. The mapping is a HashMap where the key is the
/// position in the new assignment vector and the value is the position in the old
/// assignment vector.
///
/// # Arguments
///
/// * `reader` - A reader that implements the `Read` trait containing the BEN file to
/// be relabeled.
/// * `writer` - A writer that implements the `Write` trait and which will contain the
/// relabeled BEN file.
/// * `new_to_old_node_map` - A HashMap where the key is the position in the new assignment
/// vector and the value is the position in the old assignment vector.
///
/// # Errors
///
/// Returns an error if the file format is invalid or if there is an issue reading or writing
/// the file.
pub fn relabel_ben_lines_with_map<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    new_to_old_node_map: HashMap<usize, usize>,
    variant: BenVariant,
) -> io::Result<()> {
    let mut sample_number = 0;
    loop {
        let mut tmp_buffer = [0u8];
        let max_val_bits = match reader.read_exact(&mut tmp_buffer) {
            Ok(_) => tmp_buffer[0],
            Err(e) => {
                if e.kind() == io::ErrorKind::UnexpectedEof {
                    break;
                }
                return Err(e);
            }
        };

        let max_len_bits = reader.read_u8()?;
        let n_bytes = reader.read_u32::<BigEndian>()?;

        let ben_line = decode_ben_line(&mut reader, max_val_bits, max_len_bits, n_bytes)?;

        let assignment_vec = rle_to_vec(ben_line);
        let new_assignment_vec = assignment_vec
            .iter()
            .enumerate()
            .map(|(i, _)| {
                // position of the new value in the old assignment
                let new_val_pos = new_to_old_node_map.get(&i).unwrap();
                // get the new value from the old assignment
                let new_val = assignment_vec[*new_val_pos];
                new_val
            })
            .collect::<Vec<u16>>();

        let new_rle = assign_to_rle(new_assignment_vec);

        let relabeled = encode_ben_vec_from_rle(new_rle);
        writer.write_all(&relabeled)?;

        let count_occurrences = if variant == BenVariant::MkvChain {
            let count = reader.read_u16::<BigEndian>()?;
            writer.write_all(&count.to_be_bytes())?;
            count
        } else {
            1
        };

        sample_number += count_occurrences as usize;
        log!("Relabeling line: {}\r", sample_number);
    }
    logln!();
    logln!("Done!");

    Ok(())
}

/// Relabels the values in a BEN file so that the assignment vector values are
/// sorted according to a mapping. The mapping is a HashMap where the key is the
/// position in the new assignment vector and the value is the position in the old
/// assignment vector.
///
/// # Arguments
///
/// * `reader` - A reader that implements the `Read` trait containing the BEN file to
/// be relabeled.
/// * `writer` - A writer that implements the `Write` trait and which will contain the
/// relabeled BEN file.
/// * `new_to_old_node_map` - A HashMap where the key is the position in the new assignment
/// vector and the value is the position in the old assignment vector.
///
/// # Errors
///
/// Returns an error if the file format is invalid or if there is an issue reading or writing
/// the file.d according to a mapping. The mapping is a HashMap where the key is the
pub fn relabel_ben_file_with_map<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    new_to_old_node_map: HashMap<usize, usize>,
) -> io::Result<()> {
    let mut check_buffer = [0u8; 17];
    reader.read_exact(&mut check_buffer)?;

    let variant = match &check_buffer {
        b"STANDARD BEN FILE" => BenVariant::Standard,
        b"MKVCHAIN BEN FILE" => BenVariant::MkvChain,
        _ => {
            return Err(Error::new(
                io::ErrorKind::InvalidData,
                "Invalid file format",
            ));
        }
    };

    writer.write_all(&check_buffer)?;

    relabel_ben_lines_with_map(&mut reader, &mut writer, new_to_old_node_map, variant)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::seq::SliceRandom;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;
    use rand_distr::{Distribution, Uniform};

    fn shuffle_with_mapping<T>(vec: &mut Vec<T>) -> HashMap<usize, usize>
    where
        T: Clone + std::cmp::PartialEq,
    {
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let original_vec = vec.clone(); // Clone the original vector to preserve initial values
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

        let input = encode_ben_vec_from_rle(in_rle);

        let out_rle = vec![(1, 2), (2, 2), (3, 2), (4, 2)];
        let expected = encode_ben_vec_from_rle(out_rle);

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
    fn test_relabel_ben_line_with_map() {
        let in_assign = vec![2, 3, 1, 4, 5, 5, 3, 4, 2];
        let in_rle = assign_to_rle(in_assign);

        let input = encode_ben_vec_from_rle(in_rle);

        let out_assign = vec![1, 2, 2, 3, 3, 4, 4, 5, 5];
        let out_rle = assign_to_rle(out_assign);
        let expected = encode_ben_vec_from_rle(out_rle);

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
        let input = encode_ben_vec_from_rle(in_rle);

        let new_to_old_map = shuffle_with_mapping(&mut out_assign);
        let out_rle = assign_to_rle(out_assign);
        let expected = encode_ben_vec_from_rle(out_rle);

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
        let input = encode_ben_vec_from_rle(in_rle);

        let new_to_old_map = shuffle_with_mapping(&mut out_assign);
        let out_rle = assign_to_rle(out_assign);
        let expected = encode_ben_vec_from_rle(out_rle);

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
}
