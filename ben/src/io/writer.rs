use crate::codec::encode::{
    build_twodelta_runs_with_hint, encode_ben32_assignments, encode_ben_vec_from_assign,
    encode_twodelta_vec_with_hint, BenFrame, TwoDeltaFrame,
};
use crate::codec::translate::ben_to_ben32_lines;
use crate::format::banners::{banner_for_variant, has_known_banner_prefix, BANNER_LEN};
use crate::io::reader::BenDecoder;
use crate::util::rle::assign_to_rle;
use crate::BenVariant;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{self, BufRead, Read, Result, Write};
use xz2::write::XzEncoder;

const XBEN_TWODELTA_FULL_TAG: u8 = 0;
const XBEN_TWODELTA_DELTA_TAG: u8 = 1;

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

fn parse_json_assignment(data: Value) -> Result<Vec<u16>> {
    let assign_vec = data["assignment"].as_array().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "'assignment' field either missing or is not an array of integers",
        )
    })?;

    assign_vec
        .iter()
        .map(|x| {
            let u = x.as_u64().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "The value '{}' could not be unwrapped as an unsigned 64 bit integer.",
                        x
                    ),
                )
            })?;

            u16::try_from(u).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("The value '{}' is too large to fit in a u16.", u),
                )
            })
        })
        .collect()
}

fn encode_xben_twodelta_full_frame(assignments: &[u16]) -> Vec<u8> {
    let runs = assign_to_rle(assignments);
    let mut bytes = Vec::with_capacity(1 + 4 + runs.len() * 4);
    bytes.push(XBEN_TWODELTA_FULL_TAG);
    bytes.extend_from_slice(&(runs.len() as u32).to_be_bytes());
    for (value, len) in runs {
        bytes.extend_from_slice(&value.to_be_bytes());
        bytes.extend_from_slice(&len.to_be_bytes());
    }
    bytes
}

fn encode_xben_twodelta_delta_frame(
    previous_assignment: &[u16],
    new_assignment: &[u16],
    delta_pair: Option<(u16, u16)>,
    masks: Option<&HashMap<u16, Vec<usize>>>,
) -> io::Result<Vec<u8>> {
    let (ordered_pair, run_lengths) =
        build_twodelta_runs_with_hint(previous_assignment, new_assignment, delta_pair, masks)?;
    let mut bytes = Vec::with_capacity(1 + 2 + 2 + 4 + run_lengths.len() * 2);
    bytes.push(XBEN_TWODELTA_DELTA_TAG);
    bytes.extend_from_slice(&ordered_pair.0.to_be_bytes());
    bytes.extend_from_slice(&ordered_pair.1.to_be_bytes());
    bytes.extend_from_slice(&(run_lengths.len() as u32).to_be_bytes());
    for run_length in run_lengths {
        bytes.extend_from_slice(&run_length.to_be_bytes());
    }
    Ok(bytes)
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
        writer.write_all(banner_for_variant(variant)).unwrap();

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

                let encoded = encode_ben_vec_from_assign(&assign_vec);
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
            analyze_twodelta_transition(&self.previous_sample, &assign_vec)
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
        self.write_assignment(parse_json_assignment(data)?)
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
    previous_assignment: Vec<u16>,
    previous_masks: HashMap<u16, Vec<usize>>,
    previous_frame: Vec<u8>,
    count: u16,
    variant: BenVariant,
}

impl<W: Write> XBenEncoder<W> {
    fn rebuild_previous_masks(&mut self) {
        self.previous_masks.clear();
        for (idx, &assignment) in self.previous_assignment.iter().enumerate() {
            self.previous_masks.entry(assignment).or_default().push(idx);
        }
    }

    fn set_previous_assignment(&mut self, assignment: Vec<u16>, frame: Vec<u8>, count: u16) {
        self.previous_assignment = assignment;
        self.rebuild_previous_masks();
        self.previous_frame = frame;
        self.count = count;
    }

    fn flush_pending_frame(&mut self) -> Result<()> {
        if self.count == 0 {
            return Ok(());
        }

        self.encoder.write_all(&self.previous_frame)?;
        if matches!(self.variant, BenVariant::MkvChain | BenVariant::TwoDelta) {
            self.encoder.write_all(&self.count.to_be_bytes())?;
        }
        Ok(())
    }

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
        encoder.write_all(banner_for_variant(variant)).unwrap();
        match variant {
            BenVariant::Standard => XBenEncoder {
                encoder,
                previous_assignment: Vec::new(),
                previous_masks: HashMap::new(),
                previous_frame: Vec::new(),
                count: 0,
                variant: BenVariant::Standard,
            },
            BenVariant::MkvChain => XBenEncoder {
                encoder,
                previous_assignment: Vec::new(),
                previous_masks: HashMap::new(),
                previous_frame: Vec::new(),
                count: 0,
                variant: BenVariant::MkvChain,
            },
            BenVariant::TwoDelta => XBenEncoder {
                encoder,
                previous_assignment: Vec::new(),
                previous_masks: HashMap::new(),
                previous_frame: Vec::new(),
                count: 0,
                variant: BenVariant::TwoDelta,
            },
        }
    }

    /// Encode and write a full assignment vector into the compressed XBEN stream.
    ///
    /// # Arguments
    ///
    /// * `assign_vec` - The full assignment vector to encode.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` after the assignment has been queued or written.
    pub fn write_assignment(&mut self, assign_vec: Vec<u16>) -> Result<()> {
        match self.variant {
            BenVariant::Standard => {
                let encoded = encode_ben32_assignments(&assign_vec)?.into_u8_vec()?;
                self.encoder.write_all(&encoded)?;
                self.previous_assignment = assign_vec;
                self.previous_frame = encoded;
                Ok(())
            }
            BenVariant::MkvChain => {
                if is_repeated_assignment(&self.previous_assignment, &assign_vec) {
                    self.count += 1;
                    return Ok(());
                }

                self.flush_pending_frame()?;
                let encoded = encode_ben32_assignments(&assign_vec)?.into_u8_vec()?;
                self.set_previous_assignment(assign_vec, encoded, 1);
                Ok(())
            }
            BenVariant::TwoDelta => {
                if self.previous_assignment.is_empty() {
                    let encoded = encode_xben_twodelta_full_frame(&assign_vec);
                    self.set_previous_assignment(assign_vec, encoded, 1);
                    return Ok(());
                }

                let hints = analyze_twodelta_transition(&self.previous_assignment, &assign_vec);
                if hints.is_repeated {
                    self.count += 1;
                    return Ok(());
                }

                let encoded = encode_xben_twodelta_delta_frame(
                    &self.previous_assignment,
                    &assign_vec,
                    hints.delta_pair,
                    Some(&self.previous_masks),
                )?;
                self.flush_pending_frame()?;
                self.set_previous_assignment(assign_vec, encoded, 1);
                Ok(())
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
        self.write_assignment(parse_json_assignment(data)?)
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
        let has_banner = peek.len() >= BANNER_LEN && has_known_banner_prefix(peek);

        if has_banner {
            if self.variant == BenVariant::TwoDelta {
                let mut banner = [0u8; BANNER_LEN];
                banner.copy_from_slice(&peek[..BANNER_LEN]);
                reader.consume(BANNER_LEN);

                let decoder = BenDecoder::new(io::Cursor::new(banner).chain(reader))?;
                for record in decoder {
                    let (assignment, count) = record?;
                    self.write_assignment(assignment.clone())?;
                    if matches!(self.variant, BenVariant::MkvChain | BenVariant::TwoDelta)
                        && count > 1
                    {
                        self.count += count - 1;
                    } else if self.variant == BenVariant::Standard {
                        for _ in 1..count {
                            self.write_assignment(assignment.clone())?;
                        }
                    }
                }
                return Ok(());
            }
            reader.consume(BANNER_LEN);
        }

        if self.variant == BenVariant::TwoDelta {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "TwoDelta XBEN translation requires a BEN stream with its banner",
            ));
        }

        ben_to_ben32_lines(&mut reader, &mut self.encoder, self.variant)
    }
}

impl<W: Write> Drop for XBenEncoder<W> {
    /// Flush any buffered XBEN repetition state during drop.
    fn drop(&mut self) {
        if matches!(self.variant, BenVariant::MkvChain | BenVariant::TwoDelta) && self.count > 0 {
            self.flush_pending_frame()
                .expect("Error writing last XBEN frame to file");
        }
    }
}
