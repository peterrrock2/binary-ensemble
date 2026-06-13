use super::compress_rle_to_ben_bytes;
use crate::codec::encode::errors::EncodeError;
use crate::util::rle::assign_to_rle;
use crate::BenVariant;
use std::io;

/// Serialize a TwoDelta frame's wire bytes from its parsed parts:
/// `[pair.0 u16][pair.1 u16][width u8][n_bytes u32][payload][count u16]`, all big-endian.
fn assemble_twodelta_raw_bytes(
    pair: (u16, u16),
    max_len_bit_count: u8,
    payload: &[u8],
    count: u16,
) -> Vec<u8> {
    let mut raw_bytes = Vec::with_capacity(9 + payload.len() + 2);
    raw_bytes.extend_from_slice(&pair.0.to_be_bytes());
    raw_bytes.extend_from_slice(&pair.1.to_be_bytes());
    raw_bytes.push(max_len_bit_count);
    raw_bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    raw_bytes.extend_from_slice(payload);
    raw_bytes.extend_from_slice(&count.to_be_bytes());
    raw_bytes
}

/// One sample's encoded bytes at the frame layer.
///
/// Variants mirror [`BenVariant`]: a stream's variant tag dictates which arm each frame in the
/// stream uses. Encode-side arms carry the source RLE runs (or run-length vector for `TwoDelta`)
/// alongside the serialized `raw_bytes`, because frames on this side are built *from* runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BenEncodeFrame {
    /// A `Standard`-variant frame. No trailing repetition count on the wire.
    Standard {
        /// The RLE runs that were encoded into this frame.
        runs: Vec<(u16, u16)>,
        /// The number of bits used to encode the maximum district id.
        max_val_bit_count: u8,
        /// The number of bits used to encode the maximum run length.
        max_len_bit_count: u8,
        /// The number of bytes in the packed payload.
        n_bytes: u32,
        /// The full serialized frame bytes (frame header + payload).
        raw_bytes: Vec<u8>,
    },
    /// An `MkvChain`-variant frame. Carries a trailing `u16` repetition count.
    MkvChain {
        /// The RLE runs that were encoded into this frame.
        runs: Vec<(u16, u16)>,
        /// The number of bits used to encode the maximum district id.
        max_val_bit_count: u8,
        /// The number of bits used to encode the maximum run length.
        max_len_bit_count: u8,
        /// The number of bytes in the packed payload.
        n_bytes: u32,
        /// The full serialized frame bytes (frame header + payload + count).
        raw_bytes: Vec<u8>,
        /// The number of times this frame repeats.
        count: u16,
    },
    /// A `TwoDelta`-variant frame: a delta over `pair` with alternating run lengths. Carries a
    /// trailing `u16` repetition count.
    TwoDelta {
        /// The pair of district ids encoded in this frame. `pair.0` corresponds to the first run.
        pair: (u16, u16),
        /// The number of bits used to encode the maximum run length.
        max_len_bit_count: u8,
        /// The number of bytes in the packed payload.
        n_bytes: u32,
        /// The alternating run-length vector over the positions occupied by the pair.
        run_length_vector: Vec<u16>,
        /// The full serialized TwoDelta frame bytes (header + payload + count).
        raw_bytes: Vec<u8>,
        /// The number of times this frame repeats.
        count: u16,
    },
}

impl BenEncodeFrame {
    /// Build a `Standard` or `MkvChain` frame from RLE runs.
    ///
    /// `count` is ignored for `Standard` and defaults to `1` for `MkvChain`.
    ///
    /// # Errors
    ///
    /// Returns `InvalidInput` if `variant` is [`BenVariant::TwoDelta`] (use
    /// [`BenEncodeFrame::from_run_lengths`] for that) or if the packed payload would overflow
    /// the frame header's `u32` byte length, a bound only corrupt input can reach.
    pub fn from_rle(
        runs: Vec<(u16, u16)>,
        variant: BenVariant,
        count: Option<u16>,
    ) -> io::Result<Self> {
        let (max_val, max_len) = runs
            .iter()
            .fold((0u16, 0u16), |(max_val, max_len), &(val, len)| {
                (max_val.max(val), max_len.max(len))
            });
        let max_val_bit_count = (16 - max_val.leading_zeros() as u8).max(1);
        let max_len_bit_count = (16 - max_len.leading_zeros() as u8).max(1);
        let assign_bits = (max_val_bit_count + max_len_bit_count) as u64;
        let payload_bits = assign_bits * runs.len() as u64;
        let n_bytes = u32::try_from(payload_bits.div_ceil(8)).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                EncodeError::FramePayloadOverflow {
                    runs: runs.len(),
                    bits_per_run: assign_bits,
                },
            )
        })?;
        let mut raw_bytes =
            compress_rle_to_ben_bytes(max_val_bit_count, max_len_bit_count, n_bytes, &runs);

        match variant {
            BenVariant::Standard => Ok(Self::Standard {
                runs,
                max_val_bit_count,
                max_len_bit_count,
                n_bytes,
                raw_bytes,
            }),
            BenVariant::MkvChain => {
                let count = count.unwrap_or(1);
                raw_bytes.extend(count.to_be_bytes());
                Ok(Self::MkvChain {
                    runs,
                    max_val_bit_count,
                    max_len_bit_count,
                    n_bytes,
                    raw_bytes,
                    count,
                })
            }
            BenVariant::TwoDelta => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "BenEncodeFrame::from_rle does not support TwoDelta; \
                 use BenEncodeFrame::from_run_lengths instead",
            )),
        }
    }

    /// Build a `Standard` or `MkvChain` frame from an assignment vector.
    ///
    /// # Errors
    ///
    /// Returns `InvalidInput` if `variant` is [`BenVariant::TwoDelta`] (TwoDelta frames cannot be
    /// derived from a single assignment vector) or if the packed payload would overflow the
    /// frame header's `u32` byte length; see [`BenEncodeFrame::from_rle`].
    pub fn from_assignment(
        assignment: impl AsRef<[u16]>,
        variant: BenVariant,
        count: Option<u16>,
    ) -> io::Result<Self> {
        Self::from_rle(assign_to_rle(assignment), variant, count)
    }

    /// Build a `TwoDelta` frame from a pair and pre-computed run lengths.
    ///
    /// `count` defaults to `1` if `None`.
    ///
    /// # Errors
    ///
    /// Returns `InvalidInput` if the packed payload would overflow the frame header's `u32` byte
    /// length, a bound only corrupt input can reach.
    pub fn from_run_lengths(
        pair: (u16, u16),
        run_length_vector: Vec<u16>,
        count: Option<u16>,
    ) -> io::Result<Self> {
        let count = count.unwrap_or(1);

        let max_len = run_length_vector.iter().copied().max().unwrap_or(0);
        let max_len_bit_count = (16 - max_len.leading_zeros() as u8).max(1);

        let payload_bits = max_len_bit_count as u64 * run_length_vector.len() as u64;
        let n_bytes = u32::try_from(payload_bits.div_ceil(8)).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                EncodeError::FramePayloadOverflow {
                    runs: run_length_vector.len(),
                    bits_per_run: u64::from(max_len_bit_count),
                },
            )
        })?;

        // pair_bytes (4) + max_len_bit_count (1) + n_bytes (4) + payload (n_bytes) + count (2)
        let mut raw_bytes = Vec::with_capacity((n_bytes + 11) as usize);
        raw_bytes.extend_from_slice(&pair.0.to_be_bytes());
        raw_bytes.extend_from_slice(&pair.1.to_be_bytes());
        raw_bytes.push(max_len_bit_count);
        raw_bytes.extend_from_slice(&n_bytes.to_be_bytes());

        let mut remainder: u32 = 0;
        let mut remainder_bits: u8 = 0;

        for &item in &run_length_vector {
            let mut packed = (remainder << max_len_bit_count) | item as u32;
            let mut bits_left = remainder_bits + max_len_bit_count;

            while bits_left >= 8 {
                bits_left -= 8;
                raw_bytes.push((packed >> bits_left) as u8);
                packed &= !((u32::MAX) << bits_left);
            }

            remainder = packed;
            remainder_bits = bits_left;
        }

        if remainder_bits > 0 {
            raw_bytes.push((remainder << (8 - remainder_bits)) as u8);
        }

        raw_bytes.extend(count.to_be_bytes());

        Ok(Self::TwoDelta {
            pair,
            max_len_bit_count,
            n_bytes,
            run_length_vector,
            raw_bytes,
            count,
        })
    }

    /// Reconstruct a `TwoDelta` frame from already-parsed header fields and a raw payload,
    /// validating the payload as it is unpacked.
    ///
    /// This is the inverse of [`BenEncodeFrame::from_run_lengths`]: it re-assembles the serialized
    /// bytes and decodes the bit-packed payload back into the run-length vector so that both
    /// representations are available on the resulting frame.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::InvalidData`] when:
    ///
    /// - `max_len_bit_count` is outside `1..=16`;
    /// - the payload contains an interior zero run length; the encoder never emits zero-length
    ///   runs, so only the final byte's zero padding may form zero slots, and silently dropping
    ///   one would shift the alternation parity of every later run;
    /// - the payload length is not `ceil(runs * width / 8)` for the recovered run count.
    pub fn try_from_parts(
        pair: (u16, u16),
        max_len_bit_count: u8,
        payload: Vec<u8>,
        count: u16,
    ) -> io::Result<Self> {
        use super::decode::{
            check_twodelta_frame_consistency, check_twodelta_run_width, unpack_twodelta_run_lengths,
        };

        check_twodelta_run_width(max_len_bit_count)?;
        let n_bytes = payload.len() as u32;
        let run_length_vector = unpack_twodelta_run_lengths(&payload, max_len_bit_count)?;
        check_twodelta_frame_consistency(n_bytes, run_length_vector.len(), max_len_bit_count)?;

        let raw_bytes = assemble_twodelta_raw_bytes(pair, max_len_bit_count, &payload, count);
        Ok(Self::TwoDelta {
            pair,
            max_len_bit_count,
            n_bytes,
            run_length_vector,
            raw_bytes,
            count,
        })
    }

    /// Borrow the serialized frame bytes.
    pub fn as_slice(&self) -> &[u8] {
        match self {
            Self::Standard { raw_bytes, .. } => raw_bytes,
            Self::MkvChain { raw_bytes, .. } => raw_bytes,
            Self::TwoDelta { raw_bytes, .. } => raw_bytes,
        }
    }

    /// Clone out the serialized frame bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.as_slice().to_vec()
    }

    /// Consume the frame and return the serialized frame bytes without cloning.
    pub fn into_bytes(self) -> Vec<u8> {
        match self {
            Self::Standard { raw_bytes, .. } => raw_bytes,
            Self::MkvChain { raw_bytes, .. } => raw_bytes,
            Self::TwoDelta { raw_bytes, .. } => raw_bytes,
        }
    }

    /// Borrow just the packed payload bytes (the variant-specific region between the frame header
    /// and any trailing count).
    ///
    /// Returns the payload slice for any well-formed frame.
    pub fn payload(&self) -> &[u8] {
        match self {
            Self::Standard {
                n_bytes, raw_bytes, ..
            }
            | Self::MkvChain {
                n_bytes, raw_bytes, ..
            } => &raw_bytes[6..6 + *n_bytes as usize],
            Self::TwoDelta {
                n_bytes, raw_bytes, ..
            } => &raw_bytes[9..9 + *n_bytes as usize],
        }
    }

    /// The frame's repetition count (`1` for `Standard`).
    pub fn count(&self) -> u16 {
        match self {
            Self::Standard { .. } => 1,
            Self::MkvChain { count, .. } | Self::TwoDelta { count, .. } => *count,
        }
    }

    /// The variant tag corresponding to this frame's arm.
    pub fn variant(&self) -> BenVariant {
        match self {
            Self::Standard { .. } => BenVariant::Standard,
            Self::MkvChain { .. } => BenVariant::MkvChain,
            Self::TwoDelta { .. } => BenVariant::TwoDelta,
        }
    }

    /// The bit width of the largest district id in this frame, or `None` for `TwoDelta`
    /// (which doesn't carry one).
    pub fn max_val_bit_count(&self) -> Option<u8> {
        match self {
            Self::Standard {
                max_val_bit_count, ..
            }
            | Self::MkvChain {
                max_val_bit_count, ..
            } => Some(*max_val_bit_count),
            Self::TwoDelta { .. } => None,
        }
    }

    /// The bit width of the largest run length in this frame.
    pub fn max_len_bit_count(&self) -> u8 {
        match self {
            Self::Standard {
                max_len_bit_count, ..
            }
            | Self::MkvChain {
                max_len_bit_count, ..
            }
            | Self::TwoDelta {
                max_len_bit_count, ..
            } => *max_len_bit_count,
        }
    }

    /// The number of bytes in the packed payload region.
    pub fn n_bytes(&self) -> u32 {
        match self {
            Self::Standard { n_bytes, .. }
            | Self::MkvChain { n_bytes, .. }
            | Self::TwoDelta { n_bytes, .. } => *n_bytes,
        }
    }

    /// The pair of district ids encoded by a `TwoDelta` frame, or `None` for the snapshot arms.
    pub fn pair(&self) -> Option<(u16, u16)> {
        match self {
            Self::TwoDelta { pair, .. } => Some(*pair),
            _ => None,
        }
    }

    /// Borrow the source RLE runs for `Standard` and `MkvChain`, or `None` for `TwoDelta`
    /// (which carries `run_length_vector` instead).
    pub fn runs(&self) -> Option<&[(u16, u16)]> {
        match self {
            Self::Standard { runs, .. } | Self::MkvChain { runs, .. } => Some(runs),
            Self::TwoDelta { .. } => None,
        }
    }

    /// Borrow the alternating run-length vector for a `TwoDelta` frame, or `None` for the snapshot
    /// arms.
    pub fn run_length_vector(&self) -> Option<&[u16]> {
        match self {
            Self::TwoDelta {
                run_length_vector, ..
            } => Some(run_length_vector),
            _ => None,
        }
    }
}

impl AsRef<[u8]> for BenEncodeFrame {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl std::ops::Deref for BenEncodeFrame {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl PartialEq<Vec<u8>> for BenEncodeFrame {
    fn eq(&self, other: &Vec<u8>) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl PartialEq<BenEncodeFrame> for Vec<u8> {
    fn eq(&self, other: &BenEncodeFrame) -> bool {
        self.as_slice() == other.as_slice()
    }
}
