//! Raw-frame iterator surface over the unified stream reader.

use std::io::{self, Read};

use super::ben::pop_frame_from_reader;
use super::xben::pop_frame_from_overflow;
use super::{zero_count_frame_error, BenStreamInner, BenStreamReader, XBenInner};
use crate::codec::encode::encode_ben32_assignments;
use crate::codec::{BenDecodeFrame, BenEncodeFrame};
use crate::io::reader::errors::DecoderInitError;
use crate::io::reader::subsample::DecodeFrame;
use crate::BenVariant;

/// Iterator over raw frames from a [`BenStreamReader`].
///
/// In the BEN arm: `Standard` and `MkvChain` frames are yielded as read off the wire; `TwoDelta`
/// frames are materialized as assignments and re-encoded as `Standard` decode frames so downstream
/// subsample consumers always see self-contained frames.
///
/// In the XBEN arm: `Standard` and `MkvChain` frames are yielded as raw ben32 byte slices with
/// their repetition count; `TwoDelta` chunks are materialized to assignments and re-encoded as
/// ben32 frames.
pub struct BenStreamFrameReader<R: Read> {
    inner: BenStreamReader<R>,
}

impl<R: Read> BenStreamFrameReader<R> {
    /// Create a raw frame iterator from a plain BEN stream.
    pub fn from_ben(reader: R) -> Result<Self, DecoderInitError> {
        Ok(Self {
            inner: BenStreamReader::from_ben(reader)?,
        })
    }

    /// Create a raw frame iterator from an XBEN stream.
    pub fn from_xben(reader: R) -> Result<Self, DecoderInitError> {
        Ok(Self {
            inner: BenStreamReader::from_xben(reader)?,
        })
    }

    pub(super) fn from_stream(inner: BenStreamReader<R>) -> Self {
        Self { inner }
    }

    /// Return the BEN variant detected from the stream banner.
    pub fn variant(&self) -> BenVariant {
        self.inner.variant()
    }

    /// Return the wire format of the underlying stream.
    pub fn wire_format(&self) -> super::BenWireFormat {
        self.inner.wire_format()
    }
}

impl<R: Read> Iterator for BenStreamFrameReader<R> {
    type Item = io::Result<(DecodeFrame, u16)>;

    fn next(&mut self) -> Option<Self::Item> {
        let variant = self.inner.variant();
        let silent = self.inner.is_silent();
        match self.inner.inner_mut() {
            BenStreamInner::Ben {
                reader,
                previous_assignment,
                twodelta_masks,
                sample_count,
                spinner,
            } => match variant {
                BenVariant::Standard | BenVariant::MkvChain => {
                    match pop_frame_from_reader(reader, variant) {
                        Some(Ok(frame)) => {
                            let count = frame.count();
                            if count == 0 {
                                return Some(Err(zero_count_frame_error("BEN")));
                            }
                            Some(Ok((DecodeFrame::Ben(frame), count)))
                        }
                        Some(Err(e)) => Some(Err(e)),
                        None => None,
                    }
                }
                BenVariant::TwoDelta => {
                    match super::ben::next_record_ben(
                        reader,
                        variant,
                        previous_assignment,
                        twodelta_masks,
                        sample_count,
                        spinner,
                        silent,
                    ) {
                        Some(Ok((assignment, count))) => {
                            let encoded = match BenEncodeFrame::from_assignment(
                                &assignment,
                                BenVariant::Standard,
                                None,
                            ) {
                                Ok(encoded) => encoded,
                                Err(e) => return Some(Err(e)),
                            };
                            let (max_val_bit_count, max_len_bit_count, n_bytes, raw_bytes) =
                                match encoded {
                                    BenEncodeFrame::Standard {
                                        max_val_bit_count,
                                        max_len_bit_count,
                                        n_bytes,
                                        raw_bytes,
                                        ..
                                    } => {
                                        (max_val_bit_count, max_len_bit_count, n_bytes, raw_bytes)
                                    }
                                    _ => unreachable!(
                                        "BenEncodeFrame::from_assignment(Standard) always returns Standard"
                                    ),
                                };
                            // Strip the 6-byte frame header so the emitted decode-side frame's
                            // raw_bytes matches the historical payload-only shape that
                            // BenDecodeFrame::Standard carries.
                            let payload_only = raw_bytes[6..].to_vec();
                            Some(Ok((
                                DecodeFrame::Ben(BenDecodeFrame::Standard {
                                    max_val_bit_count,
                                    max_len_bit_count,
                                    n_bytes,
                                    raw_bytes: payload_only,
                                }),
                                count,
                            )))
                        }
                        Some(Err(err)) => Some(Err(err)),
                        None => None,
                    }
                }
            },
            BenStreamInner::XBen(inner) => next_frame_xben(inner, variant)
                .map(|res| res.map(|(bytes, cnt)| (DecodeFrame::XBen(bytes, variant), cnt))),
        }
    }
}

/// Pull the next raw ben32 frame from an XBEN inner state.
///
/// For TwoDelta streams the underlying chunk is materialized via the record iterator and re-encoded
/// as a self-contained ben32 frame.
pub(super) fn next_frame_xben<R: Read>(
    inner: &mut XBenInner<R>,
    variant: BenVariant,
) -> Option<io::Result<(Vec<u8>, u16)>> {
    if variant == BenVariant::TwoDelta {
        return super::xben::next_record_xben(inner, variant).map(|res| {
            res.and_then(|(assignment, count)| Ok((encode_ben32_assignments(&assignment)?, count)))
        });
    }

    use crate::codec::decode::DecodeError;
    loop {
        if let Some((frame, consumed, count)) = pop_frame_from_overflow(variant, &inner.overflow) {
            if count == 0 {
                inner.overflow.drain(..consumed);
                return Some(Err(zero_count_frame_error("XBEN")));
            }
            let out = frame.to_vec();
            inner.overflow.drain(..consumed);
            return Some(Ok((out, count)));
        }

        let read = match inner.xz.read(&mut inner.buf) {
            Ok(0) => {
                if inner.overflow.is_empty() {
                    return None;
                } else {
                    return Some(Err(io::Error::from(DecodeError::XBenTruncated)));
                }
            }
            Ok(n) => n,
            Err(e) => return Some(Err(e)),
        };
        inner.overflow.extend_from_slice(&inner.buf[..read]);
    }
}
