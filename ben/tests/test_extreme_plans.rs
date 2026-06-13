//! Round-trip coverage for an extreme-but-legal plan shape at realistic scale: a 770,000-node
//! dual graph split into two districts, one of which holds a single node.
//!
//! This geometry stresses every run-length limit at once: the lone node leaves monochrome
//! stretches of ~385k+ nodes, far beyond the `u16` run limit, so layer-1 RLE must split runs in
//! every BEN frame, the BEN32 body must split its 4-byte runs, and every TwoDelta transition's
//! pair projection spans all positions, forcing the long-run snapshot fallback on each move.
//! The sample sequence mixes accepted moves of the lone node with repeats (rejected proposals)
//! so MkvChain/TwoDelta count merging is exercised too.

use binary_ensemble::io::bundle::format::AssignmentFormat;
use binary_ensemble::io::bundle::reader::BendlReader;
use binary_ensemble::io::bundle::writer::BendlWriter;
use binary_ensemble::io::reader::BenStreamReader;
use binary_ensemble::io::writer::{BenStreamWriter, XzEncodeOptions};
use binary_ensemble::BenVariant;
use std::io::Cursor;

const N: usize = 770_000;

/// District 2 everywhere except a single district-1 node at `lone`.
fn plan(lone: usize) -> Vec<u16> {
    let mut a = vec![2u16; N];
    a[lone] = 1;
    a
}

/// Accepted moves of the lone node interleaved with repeats.
fn samples() -> Vec<Vec<u16>> {
    vec![
        plan(0),
        plan(0), // repeat
        plan(1), // 2-swap move
        plan(1),
        plan(2),       // 2-swap move
        plan(385_000), // jump into the middle
        plan(385_000),
    ]
}

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

#[test]
fn extreme_two_district_plan_round_trips_every_variant_and_wire_format() {
    let samples = samples();
    for variant in [
        BenVariant::Standard,
        BenVariant::MkvChain,
        BenVariant::TwoDelta,
    ] {
        let ben = encode_ben(&samples, variant);
        assert_eq!(
            expand(BenStreamReader::from_ben(ben.as_slice()).unwrap()),
            samples,
            "{variant:?} plain BEN diverged"
        );

        let xben = encode_xben(&samples, variant);
        assert_eq!(
            expand(BenStreamReader::from_xben(Cursor::new(xben)).unwrap()),
            samples,
            "{variant:?} XBEN diverged"
        );
    }
}

#[test]
fn extreme_two_district_plan_round_trips_through_a_bendl_bundle() {
    let samples = samples();
    let xben = encode_xben(&samples, BenVariant::TwoDelta);

    let mut backing = Cursor::new(Vec::<u8>::new());
    {
        let writer = BendlWriter::new(&mut backing, AssignmentFormat::Xben).unwrap();
        let mut session = writer.into_stream_session().unwrap();
        std::io::Write::write_all(&mut session, &xben).unwrap();
        let writer = session.finish_into_writer(samples.len() as i64);
        writer.finish().unwrap();
    }

    let mut reader = BendlReader::open(Cursor::new(backing.into_inner())).unwrap();
    assert_eq!(reader.sample_count(), Some(samples.len() as i64));
    reader.verify_stream_checksum().unwrap();

    let verified = reader.open_assignment_reader().unwrap().silent(true);
    let decoded: Vec<Vec<u16>> = verified
        .flat_map(|r| {
            let (a, c) = r.unwrap();
            std::iter::repeat_n(a, c as usize)
        })
        .collect();
    assert_eq!(decoded, samples);
}
