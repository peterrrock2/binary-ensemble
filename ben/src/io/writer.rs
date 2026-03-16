use crate::codec::encode::encode_ben32_line;
use crate::codec::encode::encode_ben_vec_from_rle;
use crate::codec::translate::ben_to_ben32_lines;
use crate::util::rle::assign_to_rle;
use crate::BenVariant;
use serde_json::Value;
use std::io::{self, BufRead, Result, Write};
use xz2::write::XzEncoder;

/// A struct to make the writing of BEN files easier and more ergonomic.
pub struct BenEncoder<W: Write> {
    writer: W,
    previous_sample: Vec<u8>,
    count: u16,
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
            BenVariant::Standard => {
                writer.write_all(b"STANDARD BEN FILE").unwrap();
            }
            BenVariant::MkvChain => {
                writer.write_all(b"MKVCHAIN BEN FILE").unwrap();
            }
        }
        BenEncoder {
            writer,
            previous_sample: Vec::new(),
            count: 0,
            complete: false,
            variant,
        }
    }

    /// Encode and write a run-length encoded assignment vector as one BEN frame.
    ///
    /// # Arguments
    ///
    /// * `rle_vec` - The assignment vector in `(value, count)` form.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` after the frame has been queued or written.
    pub fn write_rle(&mut self, rle_vec: Vec<(u16, u16)>) -> Result<()> {
        match self.variant {
            BenVariant::Standard => {
                let encoded = encode_ben_vec_from_rle(rle_vec);
                self.writer.write_all(&encoded)?;
                Ok(())
            }
            BenVariant::MkvChain => {
                let encoded = encode_ben_vec_from_rle(rle_vec);
                if encoded == self.previous_sample {
                    self.count += 1;
                } else {
                    if self.count > 0 {
                        self.writer.write_all(&self.previous_sample)?;
                        self.writer.write_all(&self.count.to_be_bytes())?;
                    }
                    self.previous_sample = encoded;
                    self.count = 1;
                }
                Ok(())
            }
        }
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
        let rle_vec = assign_to_rle(assign_vec);
        self.write_rle(rle_vec)?;
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
        let assign_vec = data["assignment"].as_array().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "'assignment' field either missing or is not an array of integers",
            )
        })?;
        let converted_vec = assign_vec
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
            .collect::<Result<Vec<u16>>>()?;

        let rle_vec = assign_to_rle(converted_vec);
        self.write_rle(rle_vec)?;
        Ok(())
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
        if self.variant == BenVariant::MkvChain && self.count > 0 {
            self.writer
                .write_all(&self.previous_sample)
                .expect("Error while writing last line to file");
            self.writer
                .write_all(&self.count.to_be_bytes())
                .expect("Error while writing last count to file");
        }
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
    previous_sample: Vec<u8>,
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
                    previous_sample: Vec::new(),
                    count: 0,
                    variant: BenVariant::Standard,
                }
            }
            BenVariant::MkvChain => {
                encoder.write_all(b"MKVCHAIN BEN FILE").unwrap();
                XBenEncoder {
                    encoder,
                    previous_sample: Vec::new(),
                    count: 0,
                    variant: BenVariant::MkvChain,
                }
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
                self.encoder.write_all(&encoded)?;
            }
            BenVariant::MkvChain => {
                if encoded == self.previous_sample {
                    self.count += 1;
                } else {
                    if self.count > 0 {
                        self.encoder.write_all(&self.previous_sample)?;
                        self.encoder.write_all(&self.count.to_be_bytes())?;
                    }
                    self.previous_sample = encoded;
                    self.count = 1;
                }
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
                .write_all(&self.previous_sample)
                .expect("Error writing last line to file");
            self.encoder
                .write_all(&self.count.to_be_bytes())
                .expect("Error writing last line count to file");
        }
    }
}
