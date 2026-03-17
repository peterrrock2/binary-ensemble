use crate::codec::encode::{
    encode_ben32_line, encode_ben_vec_from_assign, encode_twodelta_vec_with_hint, BenFrame, IdVec,
    TwoDeltaFrame,
};
use crate::codec::translate::ben_to_ben32_lines;
use crate::BenVariant;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{self, BufRead, Result, Write};
use xz2::write::XzEncoder;

enum BufferedBenFrame {
    Ben(BenFrame),
    TwoDelta(TwoDeltaFrame),
}

impl BufferedBenFrame {
    fn as_slice(&self) -> &[u8] {
        match self {
            Self::Ben(frame) => frame.as_slice(),
            Self::TwoDelta(frame) => frame.as_slice(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct AssignmentHints {
    is_repeated: bool,
    delta_pair: Option<(u16, u16)>,
}

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
    pub fn new(mut writer: W, variant: BenVariant) -> Self {
        match variant {
            BenVariant::Standard => writer.write_all(b"STANDARD BEN FILE").unwrap(),
            BenVariant::MkvChain => writer.write_all(b"MKVCHAIN BEN FILE").unwrap(),
            BenVariant::TwoDelta => writer.write_all(b"TWODELTA BEN FILE").unwrap(),
        };

        BenEncoder {
            writer,
            previous_sample: Vec::new(),
            previous_masks: HashMap::new(),
            previous_encoded_sample: None,
            sample_count: 0,
            complete: false,
            variant,
        }
    }

    fn rebuild_previous_masks(&mut self) {
        self.previous_masks.clear();
        for (idx, &assignment) in self.previous_sample.iter().enumerate() {
            self.previous_masks.entry(assignment).or_default().push(idx);
        }
    }

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

    fn analyze_assignment_transition(
        previous_sample: &[u16],
        assign_vec: &[u16],
    ) -> AssignmentHints {
        Self::analyze_twodelta_transition(previous_sample, assign_vec)
    }

    fn is_repeated_assignment(previous_sample: &[u16], assign_vec: &[u16]) -> bool {
        if previous_sample.is_empty() || previous_sample.len() != assign_vec.len() {
            return false;
        }

        for (&previous, &current) in previous_sample.iter().zip(assign_vec.iter()) {
            if previous != current {
                return false;
            }
        }

        true
    }

    fn analyze_twodelta_transition(previous_sample: &[u16], assign_vec: &[u16]) -> AssignmentHints {
        if previous_sample.is_empty() || previous_sample.len() != assign_vec.len() {
            return AssignmentHints::default();
        }

        let Some(first_mismatch) = previous_sample
            .iter()
            .zip(assign_vec.iter())
            .position(|(&previous, &current)| previous != current)
        else {
            return AssignmentHints {
                is_repeated: true,
                delta_pair: None,
            };
        };

        let pair = (previous_sample[first_mismatch], assign_vec[first_mismatch]);

        for (&previous, &current) in previous_sample
            .iter()
            .zip(assign_vec.iter())
            .skip(first_mismatch + 1)
        {
            if previous == current {
                continue;
            }

            if previous != pair.0 && previous != pair.1 {
                return AssignmentHints {
                    is_repeated: false,
                    delta_pair: None,
                };
            }

            if current != pair.0 && current != pair.1 {
                return AssignmentHints {
                    is_repeated: false,
                    delta_pair: None,
                };
            }
        }

        AssignmentHints {
            is_repeated: false,
            delta_pair: Some(pair),
        }
    }

    fn write_assignment_with_hints(
        &mut self,
        assign_vec: Vec<u16>,
        hints: AssignmentHints,
    ) -> Result<()> {
        match self.variant {
            BenVariant::Standard => {
                let repeated = Self::is_repeated_assignment(&self.previous_sample, &assign_vec);
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

                let encoded = encode_ben_vec_from_assign(&assign_vec);
                self.writer.write_all(encoded.as_slice())?;
                self.set_previous_sample(assign_vec, BufferedBenFrame::Ben(encoded), 0);
                Ok(())
            }
            BenVariant::MkvChain => {
                if Self::is_repeated_assignment(&self.previous_sample, &assign_vec) {
                    self.sample_count += 1;
                    return Ok(());
                }

                if self.sample_count > 0 {
                    self.flush_pending_frame()?;
                }

                let encoded = encode_ben_vec_from_assign(&assign_vec);
                self.set_previous_sample(assign_vec, BufferedBenFrame::Ben(encoded), 1);
                Ok(())
            }
            BenVariant::TwoDelta => {
                if self.previous_sample.is_empty() {
                    let encoded = encode_ben_vec_from_assign(&assign_vec);
                    self.set_previous_sample(assign_vec, BufferedBenFrame::Ben(encoded), 1);
                    return Ok(());
                }

                if hints.is_repeated {
                    self.sample_count += 1;
                    return Ok(());
                }

                let encoded = encode_twodelta_vec_with_hint(
                    &self.previous_sample,
                    &assign_vec,
                    hints.delta_pair,
                    Some(&self.previous_masks),
                )?;
                self.flush_pending_frame()?;
                self.set_previous_sample(assign_vec, BufferedBenFrame::TwoDelta(encoded), 1);
                Ok(())
            }
        }
    }

    fn flush_pending_frame(&mut self) -> Result<()> {
        if self.sample_count == 0 {
            return Ok(());
        }

        let encoded = self
            .previous_encoded_sample
            .as_ref()
            .expect("missing previous BEN frame");
        self.writer.write_all(encoded.as_slice())?;

        if matches!(self.variant, BenVariant::MkvChain | BenVariant::TwoDelta) {
            self.writer.write_all(&self.sample_count.to_be_bytes())?;
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
            Self::analyze_assignment_transition(&self.previous_sample, &assign_vec)
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
        let assign_vec = data["assignment"].as_array().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "'assignment' field either missing or is not an array of integers",
            )
        })?;
        let previous_len = self.previous_sample.len();
        let can_compare = previous_len == assign_vec.len();
        let mut hints = AssignmentHints::default();
        let mut mismatch_pair: Option<(u16, u16)> = None;
        let mut twodelta_valid = true;
        let track_repeated = matches!(self.variant, BenVariant::Standard | BenVariant::MkvChain)
            && can_compare
            && !self.previous_sample.is_empty();
        let track_twodelta = self.variant == BenVariant::TwoDelta && can_compare;
        let mut twodelta_is_repeated = track_twodelta && !self.previous_sample.is_empty();
        let mut is_repeated = track_repeated;

        let converted_vec = assign_vec
            .iter()
            .enumerate()
            .map(|(idx, x)| {
                let u = x.as_u64().ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "The value '{}' could not be unwrapped as an unsigned 64 bit integer.",
                            x
                        ),
                    )
                })?;

                u16::try_from(u)
                    .map_err(|_| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("The value '{}' is too large to fit in a u16.", u),
                        )
                    })
                    .inspect(|value| {
                        if track_repeated && is_repeated && self.previous_sample[idx] != *value {
                            is_repeated = false;
                        }

                        if track_twodelta {
                            let previous = self.previous_sample[idx];
                            if previous != *value {
                                twodelta_is_repeated = false;
                                if let Some(pair) = mismatch_pair {
                                    if previous != pair.0 && previous != pair.1
                                        || *value != pair.0 && *value != pair.1
                                    {
                                        twodelta_valid = false;
                                    }
                                } else {
                                    mismatch_pair = Some((previous, *value));
                                }
                            }
                        }
                    })
            })
            .collect::<Result<Vec<u16>>>()?;

        if track_repeated {
            hints.is_repeated = is_repeated;
        } else if track_twodelta {
            hints.is_repeated = twodelta_is_repeated;
        } else if self.variant == BenVariant::Standard || self.variant == BenVariant::MkvChain {
            hints.is_repeated = false;
        }

        if track_twodelta && !hints.is_repeated && twodelta_valid {
            hints.delta_pair = mismatch_pair;
        }

        self.write_assignment_with_hints(converted_vec, hints)
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
        self.flush_pending_frame()
            .expect("Error while flushing trailing BEN frame");
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

/// A struct to make the writing of XBEN files easier and more ergonomic.
pub struct XBenEncoder<W: Write> {
    encoder: XzEncoder<W>,
    previous_sample: IdVec,
    count: u16,
    variant: BenVariant,
}

impl<W: Write> XBenEncoder<W> {
    /// Create a new XBEN writer around an already-configured XZ encoder.
    ///
    /// # Arguments
    ///
    /// * `encoder` - The configured XZ encoder that will receive the ben32
    ///   payload.
    /// * `variant` - The BEN variant to encode inside the compressed stream.
    ///
    /// # Returns
    ///
    /// Returns a new XBEN encoder ready to accept assignments or BEN frames.
    pub fn new(mut encoder: XzEncoder<W>, variant: BenVariant) -> Self {
        match variant {
            BenVariant::Standard => {
                encoder.write_all(b"STANDARD BEN FILE").unwrap();
                XBenEncoder {
                    encoder,
                    previous_sample: IdVec::U8(Vec::new()),
                    count: 0,
                    variant: BenVariant::Standard,
                }
            }
            BenVariant::MkvChain => {
                encoder.write_all(b"MKVCHAIN BEN FILE").unwrap();
                XBenEncoder {
                    encoder,
                    previous_sample: IdVec::U8(Vec::new()),
                    count: 0,
                    variant: BenVariant::MkvChain,
                }
            }
            BenVariant::TwoDelta => {
                panic!("not implemented");
            }
        }
    }

    /// Encode and write a JSON assignment record into the compressed XBEN stream.
    ///
    /// # Arguments
    ///
    /// * `data` - A JSON object containing an `assignment` array.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` after the record has been validated and encoded.
    pub fn write_json_value(&mut self, data: Value) -> Result<()> {
        let encoded = encode_ben32_line(data)?;
        match self.variant {
            BenVariant::Standard => {
                self.encoder.write_all(encoded.as_u8_slice()?)?;
            }
            BenVariant::MkvChain => {
                if encoded == self.previous_sample {
                    self.count += 1;
                } else {
                    if self.count > 0 {
                        self.encoder
                            .write_all(self.previous_sample.as_u8_slice()?)?;
                        self.encoder.write_all(&self.count.to_be_bytes())?;
                    }
                    self.previous_sample = encoded;
                    self.count = 1;
                }
            }
            BenVariant::TwoDelta => {
                panic!("not implemented");
            }
        }
        Ok(())
    }

    /// Read BEN frames from `reader` and write them into this XBEN stream.
    ///
    /// If the source still contains the 17-byte BEN banner, it is consumed and
    /// replaced by the banner already written by this encoder.
    ///
    /// # Arguments
    ///
    /// * `reader` - The BEN input stream, with or without its banner.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` after the BEN stream has been translated into XBEN.
    pub fn write_ben_file(&mut self, mut reader: impl BufRead) -> Result<()> {
        let peek = reader.fill_buf()?;
        let has_banner = peek.len() >= 17
            && (peek.starts_with(b"STANDARD BEN FILE") || peek.starts_with(b"MKVCHAIN BEN FILE"));

        if has_banner {
            reader.consume(17);
        }

        ben_to_ben32_lines(&mut reader, &mut self.encoder, self.variant)
    }
}

impl<W: Write> Drop for XBenEncoder<W> {
    /// Flush any buffered XBEN repetition state during drop.
    fn drop(&mut self) {
        if self.variant == BenVariant::MkvChain && self.count > 0 {
            self.encoder
                .write_all(
                    self.previous_sample
                        .as_u8_slice()
                        .expect("Error writing last line to file"),
                )
                .expect("Error writing last line to file");
            self.encoder
                .write_all(&self.count.to_be_bytes())
                .expect("Error writing last line count to file");
        }
    }
}
