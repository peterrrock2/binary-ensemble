use super::frames::BufferedBenFrame;
use super::utils::{analyze_twodelta_transition, is_repeated_assignment, parse_json_assignment};
use crate::codec::encode::encode_twodelta_frame_with_hint;
use crate::codec::{BenEncodeFrame, FromAssign};
use crate::format::banners::banner_for_variant;
use crate::BenVariant;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{self, Result, Write};

/// A struct to make the writing of BEN files easier and more ergonomic.
pub struct BenEncoder<W: Write> {
    writer: W,
    previous_sample: Vec<u16>,
    previous_masks: HashMap<u16, Vec<usize>>,
    previous_encoded_sample: Option<BufferedBenFrame>,
    sample_count: u16,
    variant: BenVariant,
    complete: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct AssignmentHints {
    pub is_repeated: bool,
    pub delta_pair: Option<(u16, u16)>,
}

impl<W: Write> BenEncoder<W> {
    /// Create a new BEN writer and immediately emit the BEN banner.
    ///
    /// # Arguments
    ///
    /// * `writer` - The destination that will receive the BEN stream.
    /// * `variant` - The BEN variant to encode.
    ///
    /// # Returns
    ///
    /// Returns a new encoder ready to accept assignments or RLE frames.
    pub fn new(mut writer: W, variant: BenVariant) -> io::Result<Self> {
        writer.write_all(banner_for_variant(variant))?;

        Ok(BenEncoder {
            writer,
            previous_sample: Vec::new(),
            previous_masks: HashMap::new(),
            previous_encoded_sample: None,
            sample_count: 0,
            complete: false,
            variant,
        })
    }

    /// Rebuild the value-to-position index map from the current previous sample.
    fn rebuild_previous_masks(&mut self) {
        self.previous_masks.clear();
        for (idx, &assignment) in self.previous_sample.iter().enumerate() {
            self.previous_masks.entry(assignment).or_default().push(idx);
        }
    }

    /// Store a new previous sample along with its encoded frame and repetition count.
    ///
    /// # Arguments
    ///
    /// * `sample` - The assignment vector to cache.
    /// * `encoded` - The already-encoded frame for this assignment.
    /// * `sample_count` - The initial repetition count for this sample.
    fn set_previous_sample(
        &mut self,
        sample: Vec<u16>,
        encoded: BufferedBenFrame,
        sample_count: u16,
    ) {
        self.previous_sample = sample;
        self.rebuild_previous_masks();
        self.previous_encoded_sample = Some(encoded);
        self.sample_count = sample_count;
    }

    /// Encode and write an assignment vector using pre-computed transition hints.
    ///
    /// The encoding strategy depends on the configured `BenVariant`. Repeated
    /// assignments may be deduplicated or counted, and two-delta hints enable
    /// compact delta frames when applicable.
    ///
    /// # Arguments
    ///
    /// * `assign_vec` - The assignment vector to encode.
    /// * `hints` - Pre-computed hints about repetition and delta-pair eligibility.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` after the assignment has been queued or written.
    fn write_assignment_with_hints(
        &mut self,
        assign_vec: Vec<u16>,
        hints: AssignmentHints,
    ) -> Result<()> {
        match self.variant {
            BenVariant::Standard => {
                let repeated = is_repeated_assignment(&self.previous_sample, &assign_vec);
                if hints.is_repeated {
                    if let Some(encoded) = self.previous_encoded_sample.as_ref() {
                        self.writer.write_all(encoded.as_slice())?;
                        self.previous_sample = assign_vec;
                        return Ok(());
                    }
                }

                if repeated {
                    if let Some(encoded) = self.previous_encoded_sample.as_ref() {
                        self.writer.write_all(encoded.as_slice())?;
                        self.previous_sample = assign_vec;
                        return Ok(());
                    }
                }

                let encoded = BenEncodeFrame::from_assignment(&assign_vec, None);
                self.writer.write_all(encoded.as_slice())?;
                self.set_previous_sample(assign_vec, BufferedBenFrame::Ben(encoded), 0);
                Ok(())
            }
            BenVariant::MkvChain => {
                if is_repeated_assignment(&self.previous_sample, &assign_vec) {
                    self.sample_count += 1;
                    return Ok(());
                }

                if self.sample_count > 0 {
                    self.flush_pending_frame()?;
                }

                let encoded = BenEncodeFrame::from_assignment(&assign_vec, None);
                self.set_previous_sample(assign_vec, BufferedBenFrame::Ben(encoded), 1);
                Ok(())
            }
            BenVariant::TwoDelta => {
                if self.previous_sample.is_empty() {
                    let encoded = BenEncodeFrame::from_assignment(&assign_vec, None);
                    self.set_previous_sample(assign_vec, BufferedBenFrame::Ben(encoded), 1);
                    return Ok(());
                }

                if hints.is_repeated {
                    self.sample_count += 1;
                    return Ok(());
                }

                let encoded = encode_twodelta_frame_with_hint(
                    &self.previous_sample,
                    &assign_vec,
                    hints.delta_pair,
                    Some(&mut self.previous_masks),
                )?;
                self.flush_pending_frame()?;

                self.previous_sample = assign_vec;
                self.rebuild_previous_masks();
                self.previous_encoded_sample = Some(BufferedBenFrame::TwoDelta(encoded));
                self.sample_count = 1;
                Ok(())
            }
        }
    }

    /// Flush the buffered frame and its repetition count to the underlying writer.
    ///
    /// For MkvChain and TwoDelta variants, the repetition count is appended
    /// after the encoded frame. This is a no-op when no samples are pending.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` once the pending frame has been written.
    fn flush_pending_frame(&mut self) -> Result<()> {
        if self.sample_count == 0 {
            return Ok(());
        }

        let encoded = self.previous_encoded_sample.as_ref().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "missing previous BEN frame")
        })?;
        self.writer.write_all(encoded.as_slice())?;

        if matches!(self.variant, BenVariant::MkvChain | BenVariant::TwoDelta) {
            self.writer.write_all(&self.sample_count.to_be_bytes())?;
        }

        Ok(())
    }

    /// Record additional repetitions of the most recently written assignment.
    ///
    /// For MkvChain and TwoDelta variants the repetition count is incremented
    /// directly. For Standard, the cached encoded frame is re-emitted once per
    /// additional repeat.
    ///
    /// # Arguments
    ///
    /// * `additional` - The number of extra copies beyond the one already written.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` after all additional repeats have been recorded.
    pub fn repeat_previous(&mut self, additional: u16) -> Result<()> {
        match self.variant {
            BenVariant::Standard => {
                if let Some(encoded) = self.previous_encoded_sample.as_ref() {
                    for _ in 0..additional {
                        self.writer.write_all(encoded.as_slice())?;
                    }
                }
            }
            BenVariant::MkvChain | BenVariant::TwoDelta => {
                self.sample_count += additional;
            }
        }
        Ok(())
    }

    /// Encode and write a full assignment vector.
    ///
    /// # Arguments
    ///
    /// * `assign_vec` - The full assignment vector to encode.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` after the assignment has been queued or written.
    pub fn write_assignment(&mut self, assign_vec: Vec<u16>) -> Result<()> {
        let hints = if self.variant == BenVariant::TwoDelta {
            let masks = if self.previous_masks.is_empty() {
                None
            } else {
                Some(&self.previous_masks)
            };
            analyze_twodelta_transition(&self.previous_sample, &assign_vec, masks)
        } else {
            AssignmentHints::default()
        };
        self.write_assignment_with_hints(assign_vec, hints)
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

    /// Flush any buffered repetition state to the underlying writer.
    ///
    /// This matters for [`BenVariant::MkvChain`], where repeated consecutive
    /// samples are emitted only once together with their repetition count.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` once any buffered repetition state has been flushed.
    pub fn finish(&mut self) -> Result<()> {
        if self.complete {
            return Ok(());
        }
        self.flush_pending_frame()?;
        self.complete = true;
        Ok(())
    }
}

impl<W: Write> Drop for BenEncoder<W> {
    /// Flush any buffered BEN state during drop.
    fn drop(&mut self) {
        let _ = self.finish();
    }
}
