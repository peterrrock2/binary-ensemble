//! The writers preserve arbitrary positive-integer district labels verbatim: the value `0` and
//! non-consecutive ids (gaps) survive an encode -> decode round trip unchanged, with no implicit
//! canonicalization. This pins the guarantee for every variant and both wire formats, including the
//! XBEN/ben32 path where a `0` value could in principle collide with the all-zero frame sentinel.

use binary_ensemble::io::reader::BenStreamReader;
use binary_ensemble::io::writer::{BenStreamWriter, XzEncodeOptions};
use binary_ensemble::BenVariant;
use std::io::Cursor;

const VARIANTS: [BenVariant; 3] = [
    BenVariant::Standard,
    BenVariant::MkvChain,
    BenVariant::TwoDelta,
];

fn encode_ben(samples: &[Vec<u16>], variant: BenVariant) -> Vec<u8> {
    let mut ben = Vec::new();
    let mut writer = BenStreamWriter::for_ben(&mut ben, variant).unwrap();
    for s in samples {
        writer.write_assignment(s.clone()).unwrap();
    }
    writer.finish().unwrap();
    drop(writer);
    ben
}

fn encode_xben(samples: &[Vec<u16>], variant: BenVariant) -> Vec<u8> {
    let mut xben = Vec::new();
    let mut writer =
        BenStreamWriter::for_xben(&mut xben, variant, XzEncodeOptions::default()).unwrap();
    for s in samples {
        writer.write_assignment(s.clone()).unwrap();
    }
    writer.finish().unwrap();
    drop(writer);
    xben
}

fn expand<R: std::io::Read>(reader: BenStreamReader<R>) -> Vec<Vec<u16>> {
    reader
        .silent(true)
        .flat_map(|r| {
            let (a, c) = r.unwrap();
            std::iter::repeat_n(a, c as usize)
        })
        .collect()
}

/// Assert `samples` round-trips byte-for-byte through every variant and both wire formats.
fn assert_round_trips_everywhere(samples: &[Vec<u16>]) {
    for variant in VARIANTS {
        let ben = encode_ben(samples, variant);
        assert_eq!(
            expand(BenStreamReader::from_ben(ben.as_slice()).unwrap()),
            samples,
            "{variant:?} plain BEN diverged"
        );

        let xben = encode_xben(samples, variant);
        assert_eq!(
            expand(BenStreamReader::from_xben(Cursor::new(xben)).unwrap()),
            samples,
            "{variant:?} XBEN diverged"
        );
    }
}

#[test]
fn zeros_embedded_in_the_assignment_survive() {
    // The trailing 0-run is the case the user raised. In XBEN its run word is `0x00000002`
    // (value 0, count 2), which must not be mistaken for the all-zero frame sentinel.
    assert_round_trips_everywhere(&[vec![1u16, 2, 3, 5, 5, 5, 0, 0]]);
}

#[test]
fn all_zero_assignment_survives() {
    // max_val = 0 forces the `.max(1)` bit-width floor in the BEN frame header.
    assert_round_trips_everywhere(&[vec![0u16, 0, 0, 0]]);
}

#[test]
fn non_consecutive_labels_survive() {
    // Gappy ids with no 1-based canonical relabeling applied.
    assert_round_trips_everywhere(&[vec![3u16, 4, 8, 9, 3, 9, 8]]);
}

#[test]
fn mixed_ensemble_with_gaps_and_zeros_across_frames_survives() {
    // Consecutive samples differ in more than two districts, so TwoDelta takes the snapshot
    // fallback on every transition; the labels must still survive untouched.
    assert_round_trips_everywhere(&[
        vec![0u16, 1, 2, 2, 5, 5, 0],
        vec![3u16, 3, 8, 8, 9, 0, 0],
        vec![0u16, 0, 0, 7, 7, 7, 4],
    ]);
}

#[test]
fn twodelta_delta_path_with_district_zero_survives() {
    // A sequence of clean 2-swaps between districts 0 and 7. Only two ids ever appear, so TwoDelta
    // encodes each transition as a pair delta (not a snapshot), exercising `0` through the pair
    // header and the paint loop rather than just the anchor frame.
    assert_round_trips_everywhere(&[
        vec![0u16, 0, 7, 7, 0, 7],
        vec![0u16, 7, 7, 7, 0, 7], // pos 1: 0 -> 7
        vec![0u16, 7, 0, 7, 0, 7], // pos 2: 7 -> 0
        vec![0u16, 7, 0, 7, 7, 7], // pos 4: 0 -> 7
    ]);
}

#[test]
fn twodelta_delta_path_with_non_consecutive_districts_survives() {
    // The same 2-swap structure between gappy ids 3 and 9.
    assert_round_trips_everywhere(&[
        vec![3u16, 3, 9, 9, 3, 9],
        vec![3u16, 9, 9, 9, 3, 9], // pos 1: 3 -> 9
        vec![3u16, 9, 3, 9, 3, 9], // pos 2: 9 -> 3
    ]);
}
