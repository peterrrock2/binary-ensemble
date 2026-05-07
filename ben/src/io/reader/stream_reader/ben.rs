//! Plain-BEN iteration logic for the unified stream reader.

use std::io::{self, Read};

use super::zero_count_frame_error;
use crate::codec::BenDecodeFrame;
use crate::io::reader::subsample::MkvRecord;
use crate::progress::Spinner;
use crate::BenVariant;

/// Read the next frame from the underlying BEN stream.
///
/// In a `TwoDelta` stream the first frame is encoded in `MkvChain` wire
/// format; this helper tracks that state so the frame module stays
/// variant-clean.
pub(super) fn pop_frame_from_reader<R: Read>(
    reader: &mut R,
    variant: BenVariant,
    twodelta_consumed_first_frame: &mut bool,
) -> Option<io::Result<BenDecodeFrame>> {
    let read_variant = if variant == BenVariant::TwoDelta && !*twodelta_consumed_first_frame {
        *twodelta_consumed_first_frame = true;
        BenVariant::MkvChain
    } else {
        variant
    };

    BenDecodeFrame::from_reader(reader, read_variant).transpose()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn for_each_assignment_ben<R: Read, F>(
    reader: &mut R,
    variant: BenVariant,
    previous_assignment: &mut Option<Vec<u16>>,
    twodelta_consumed_first_frame: &mut bool,
    sample_count: &mut usize,
    spinner: &mut Option<Spinner>,
    silent: bool,
    mut f: F,
) -> io::Result<()>
where
    F: FnMut(&[u16], u16) -> io::Result<bool>,
{
    loop {
        let frame = match pop_frame_from_reader(reader, variant, twodelta_consumed_first_frame) {
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

#[allow(clippy::too_many_arguments)]
pub(super) fn next_record_ben<R: Read>(
    reader: &mut R,
    variant: BenVariant,
    previous_assignment: &mut Option<Vec<u16>>,
    twodelta_consumed_first_frame: &mut bool,
    sample_count: &mut usize,
    spinner: &mut Option<Spinner>,
    silent: bool,
) -> Option<io::Result<MkvRecord>> {
    let frame = match pop_frame_from_reader(reader, variant, twodelta_consumed_first_frame) {
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

pub(super) fn count_samples_ben<R: Read>(
    mut reader: R,
    variant: BenVariant,
) -> io::Result<usize> {
    let mut twodelta_consumed_first_frame = false;
    let mut total = 0usize;
    while let Some(frame_res) =
        pop_frame_from_reader(&mut reader, variant, &mut twodelta_consumed_first_frame)
    {
        let count = frame_res?.count();
        if count == 0 {
            return Err(zero_count_frame_error("BEN"));
        }
        total += count as usize;
    }
    Ok(total)
}
