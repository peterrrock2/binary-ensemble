use super::utils::parse_json_assignment;
use crate::codec::encode::encode_twodelta_frame_with_hint;
use crate::codec::{BenConstruct, BenEncodeFrame, MkvBenEncodeFrame};
use crate::format::banners::banner_for_variant;
use crate::BenVariant;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{self, Result, Write};

/// A struct to make the writing of BEN files easier and more ergonomic.
pub struct AssignmentWriter<W: Write> {
    writer: W,
    previous_sample: Vec<u16>,
    previous_masks: HashMap<u16, Vec<usize>>,
    pending_sample: Option<Vec<u16>>,
    sample_count: u16,
    variant: BenVariant,
    complete: bool,
}

impl<W: Write> AssignmentWriter<W> {
    /// Create a new BEN writer and immediately emit the BEN banner.
    ///
    /// # Arguments
    ///
    /// * `writer` - The destination that will receive the BEN stream.
    /// * `variant` - The BEN variant to encode.
    ///
    /// # Returns
    ///
    /// Returns a new encoder ready to accept assignments.
    pub fn new(mut writer: W, variant: BenVariant) -> io::Result<Self> {
        writer.write_all(banner_for_variant(variant))?;

        Ok(AssignmentWriter {
            writer,
            previous_sample: Vec::new(),
            previous_masks: HashMap::new(),
            pending_sample: None,
            sample_count: 0,
            complete: false,
            variant,
        })
    }

    /// Encode and write the pending assignment with the accumulated repetition count.
    ///
    /// For TwoDelta, the first frame is written as an MkvBen frame. Subsequent
    /// frames are written as TwoDelta frames encoding the transition from
    /// `previous_sample`. This is a no-op when no sample is pending.
    ///
    /// Note: That on the first call to `flush_pending_frame` when `self.pending_sample` is `None`,
    /// the method will simply return `Ok(())` without writing anything. Flushing only happens
    /// when there is a pending sample to write.
    fn flush_pending_frame(&mut self) -> Result<()> {
        let pending_sample = match self.pending_sample.take() {
            Some(p) => p,
            None => return Ok(()),
        };

        match self.variant {
            BenVariant::Standard => {
                let frame = BenEncodeFrame::from_assignment(&pending_sample, None);
                for _ in 0..self.sample_count {
                    self.writer.write_all(frame.as_slice())?;
                }
            }
            BenVariant::MkvChain => {
                let frame =
                    MkvBenEncodeFrame::from_assignment(&pending_sample, Some(self.sample_count));
                self.writer.write_all(frame.as_slice())?;
            }
            BenVariant::TwoDelta => {
                if self.previous_sample.is_empty() {
                    // First frame: encode as MkvBen and build the initial masks.
                    for (idx, &val) in pending_sample.iter().enumerate() {
                        self.previous_masks.entry(val).or_default().push(idx);
                    }
                    let frame = MkvBenEncodeFrame::from_assignment(
                        &pending_sample,
                        Some(self.sample_count),
                    );
                    self.writer.write_all(frame.as_slice())?;
                } else {
                    let frame = encode_twodelta_frame_with_hint(
                        &self.previous_sample,
                        &pending_sample,
                        None,
                        Some(&mut self.previous_masks),
                        Some(self.sample_count),
                    )?;
                    self.writer.write_all(frame.as_slice())?;
                }
            }
        }

        self.previous_sample = pending_sample;
        Ok(())
    }

    /// Encode and write a full assignment vector.
    ///
    /// Consecutive identical assignments are counted and written as a single
    /// frame with the accumulated count for MkvChain and TwoDelta variants.
    ///
    /// # Arguments
    ///
    /// * `assign_vec` - The full assignment vector to encode.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` after the assignment has been queued or written.
    pub fn write_assignment(&mut self, assign_vec: Vec<u16>) -> Result<()> {
        if self.pending_sample.as_deref() == Some(assign_vec.as_slice()) {
            self.sample_count += 1;
            return Ok(());
        }
        self.flush_pending_frame()?;
        self.pending_sample = Some(assign_vec);
        self.sample_count = 1;
        Ok(())
    }

    /// Encode and write a JSON assignment record.
    ///
    /// The input must contain an `assignment` array of integers. Other fields
    /// are ignored.
    ///
    /// # Arguments
    ///
    /// * `data` - A JSON object containing an `assignment` array.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` after the record has been validated and encoded.
    pub fn write_json_value(&mut self, data: Value) -> Result<()> {
        let new_assign = parse_json_assignment(data)?;
        self.write_assignment(new_assign)
    }

    /// Flush any buffered state to the underlying writer.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` once any buffered state has been flushed.
    pub fn finish(&mut self) -> Result<()> {
        if self.complete {
            return Ok(());
        }
        self.flush_pending_frame()?;
        self.complete = true;
        Ok(())
    }
}

impl<W: Write> Drop for AssignmentWriter<W> {
    /// Flush any buffered BEN state during drop.
    fn drop(&mut self) {
        let _ = self.finish();
    }
}
