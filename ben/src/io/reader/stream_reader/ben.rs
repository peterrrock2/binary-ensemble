//! Plain-BEN iteration logic for the unified stream reader.

use std::io::{self, Read};

use byteorder::ReadBytesExt;

use super::zero_count_frame_error;
use crate::codec::BenDecodeFrame;
use crate::io::reader::subsample::MkvRecord;
use crate::io::reader::twodelta::{BEN_TWODELTA_DELTA_TAG, BEN_TWODELTA_SNAPSHOT_TAG};
use crate::progress::Spinner;
use crate::BenVariant;

/// Read the next frame from the underlying BEN stream.
///
/// Every frame of a `TwoDelta` stream is prefixed with a 1-byte tag selecting its body layout: a
/// `BEN_TWODELTA_SNAPSHOT_TAG` frame is `MkvChain`-formatted and a `BEN_TWODELTA_DELTA_TAG` frame is
/// a delta. The tag is consumed here so the frame module stays variant-clean. Non-`TwoDelta`
/// streams carry no tag and read their fixed body directly.
pub(super) fn pop_frame_from_reader<R: Read>(
    reader: &mut R,
    variant: BenVariant,
) -> Option<io::Result<BenDecodeFrame>> {
    if variant != BenVariant::TwoDelta {
        return BenDecodeFrame::from_reader(reader, variant).transpose();
    }

    // A clean EOF *at the tag boundary* ends the stream; an EOF after the tag is a truncated frame.
    let tag = match reader.read_u8() {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return None,
        Err(e) => return Some(Err(e)),
    };
    let resolved = match tag {
        BEN_TWODELTA_SNAPSHOT_TAG => BenVariant::MkvChain,
        BEN_TWODELTA_DELTA_TAG => BenVariant::TwoDelta,
        other => {
            return Some(Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unknown TwoDelta frame tag byte {other:#04x}"),
            )))
        }
    };
    match BenDecodeFrame::from_reader(reader, resolved) {
        Ok(Some(frame)) => Some(Ok(frame)),
        Ok(None) => Some(Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "truncated TwoDelta frame: tag byte present but frame body missing",
        ))),
        Err(e) => Some(Err(e)),
    }
}

pub(super) fn for_each_assignment_ben<R: Read, F>(
    reader: &mut R,
    variant: BenVariant,
    previous_assignment: &mut Option<Vec<u16>>,
    sample_count: &mut usize,
    spinner: &mut Option<Spinner>,
    silent: bool,
    mut f: F,
) -> io::Result<()>
where
    F: FnMut(&[u16], u16) -> io::Result<bool>,
{
    loop {
        let frame = match pop_frame_from_reader(reader, variant) {
            Some(Ok(frame)) => frame,
            Some(Err(e)) => return Err(e),
            None => return Ok(()),
        };

        let count = frame.count();
        if count == 0 {
            return Err(zero_count_frame_error("BEN"));
        }

        let assignment = frame.expand(previous_assignment.take())?;

        let keep_going = f(&assignment, count)?;
        *previous_assignment = Some(assignment);
        *sample_count += count as usize;
        if !silent {
            spinner
                .get_or_insert_with(|| Spinner::new("Decoding sample"))
                .set_count(*sample_count as u64);
        }
        if !keep_going {
            return Ok(());
        }
    }
}

pub(super) fn next_record_ben<R: Read>(
    reader: &mut R,
    variant: BenVariant,
    previous_assignment: &mut Option<Vec<u16>>,
    sample_count: &mut usize,
    spinner: &mut Option<Spinner>,
    silent: bool,
) -> Option<io::Result<MkvRecord>> {
    let frame = match pop_frame_from_reader(reader, variant) {
        Some(Ok(frame)) => frame,
        Some(Err(e)) => return Some(Err(e)),
        None => return None,
    };
    let count = frame.count();
    if count == 0 {
        return Some(Err(zero_count_frame_error("BEN")));
    }
    let assignment = match frame.expand(previous_assignment.take()) {
        Ok(a) => a,
        Err(e) => return Some(Err(e)),
    };
    *previous_assignment = Some(assignment.clone());
    *sample_count += count as usize;
    if !silent {
        spinner
            .get_or_insert_with(|| Spinner::new("Decoding sample"))
            .set_count(*sample_count as u64);
    }
    Some(Ok((assignment, count)))
}

pub(super) fn count_samples_ben<R: Read>(mut reader: R, variant: BenVariant) -> io::Result<usize> {
    let mut total = 0usize;
    while let Some(frame_res) = pop_frame_from_reader(&mut reader, variant) {
        let count = frame_res?.count();
        if count == 0 {
            return Err(zero_count_frame_error("BEN"));
        }
        total += count as usize;
    }
    Ok(total)
}
