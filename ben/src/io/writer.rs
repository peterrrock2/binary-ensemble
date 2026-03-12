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

    pub fn write_assignment(&mut self, assign_vec: Vec<u16>) -> Result<()> {
        let rle_vec = assign_to_rle(assign_vec);
        self.write_rle(rle_vec)?;
        Ok(())
    }

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

    pub fn write_json_value(&mut self, data: Value) -> Result<()> {
        let encoded = encode_ben32_line(data);
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
