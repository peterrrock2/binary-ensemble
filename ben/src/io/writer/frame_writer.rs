use super::assignment_writer::twodelta_repeat_frame;
use crate::codec::encode::encode_twodelta_frame_with_hint;
use crate::codec::BenEncodeFrame;
use crate::format::banners::banner_for_variant;
use crate::BenVariant;
use std::collections::HashMap;
use std::io::{self, Write};

/// A writer that emits one BEN frame per call, preserving input frame
/// boundaries instead of merging adjacent identical assignments.
///
/// This sidesteps the merge buffer in [`super::AssignmentWriter`]: callers
/// supply a `(assignment, count)` pair and receive one counted frame on the
/// wire. For [`BenVariant::Standard`] targets, which cannot encode
/// repetition counts, a count of `N` is expanded into `N` one-sample frames.
///
/// For [`BenVariant::TwoDelta`], the writer maintains its own
/// `previous_sample` and `previous_masks` so subsequent frames encode delta
/// transitions identically to `AssignmentWriter`.
pub(crate) struct FrameWriter<W: Write> {
    writer: W,
    variant: BenVariant,
    previous_sample: Vec<u16>,
    previous_masks: HashMap<u16, Vec<usize>>,
}

impl<W: Write> FrameWriter<W> {
    pub(crate) fn new(mut writer: W, variant: BenVariant) -> io::Result<Self> {
        writer.write_all(banner_for_variant(variant))?;
        Ok(Self {
            writer,
            variant,
            previous_sample: Vec::new(),
            previous_masks: HashMap::new(),
        })
    }

    pub(crate) fn write_frame(&mut self, assignment: Vec<u16>, count: u16) -> io::Result<()> {
        if count == 0 {
            return Ok(());
        }
        match self.variant {
            BenVariant::Standard => {
                let frame =
                    BenEncodeFrame::from_assignment(&assignment, BenVariant::Standard, None);
                for _ in 0..count {
                    self.writer.write_all(frame.as_slice())?;
                }
            }
            BenVariant::MkvChain => {
                let frame = BenEncodeFrame::from_assignment(
                    &assignment,
                    BenVariant::MkvChain,
                    Some(count),
                );
                self.writer.write_all(frame.as_slice())?;
            }
            BenVariant::TwoDelta => {
                if self.previous_sample.is_empty() {
                    for (idx, &val) in assignment.iter().enumerate() {
                        self.previous_masks.entry(val).or_default().push(idx);
                    }
                    let frame = BenEncodeFrame::from_assignment(
                        &assignment,
                        BenVariant::MkvChain,
                        Some(count),
                    );
                    self.writer.write_all(frame.as_slice())?;
                } else if self.previous_sample == assignment {
                    let frame = twodelta_repeat_frame(&assignment, count)?;
                    self.writer.write_all(frame.as_slice())?;
                } else {
                    let frame = encode_twodelta_frame_with_hint(
                        &self.previous_sample,
                        &assignment,
                        None,
                        Some(&mut self.previous_masks),
                        Some(count),
                    )?;
                    self.writer.write_all(frame.as_slice())?;
                }
                self.previous_sample = assignment;
            }
        }
        Ok(())
    }
}
