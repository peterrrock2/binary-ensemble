use crate::codec::decode::MAX_FRAME_PAYLOAD_BYTES;
use crate::BenVariant;
use byteorder::{BigEndian, ReadBytesExt};
use std::io::{self, Read};

/// Reject a declared payload length above [`MAX_FRAME_PAYLOAD_BYTES`] **before** allocating the
/// payload buffer, so a corrupt or malicious frame header cannot force a multi-gigabyte
/// reservation. Well-formed frames never approach the cap.
pub(crate) fn check_payload_len(n_bytes: u32) -> io::Result<()> {
    if n_bytes > MAX_FRAME_PAYLOAD_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "BEN frame payload of {n_bytes} bytes exceeds {MAX_FRAME_PAYLOAD_BYTES}; \
                 refusing to allocate"
            ),
        ));
    }
    Ok(())
}

/// Reject a TwoDelta run-length bit width outside `1..=16`. The bit unpackers shift a 32-bit
/// register by `32 - width` and decrement a counter by `width`, so a zero or oversized width
/// is not merely corrupt; it would shift out of range or never terminate.
pub(crate) fn check_twodelta_run_width(max_len_bits: u8) -> io::Result<()> {
    if max_len_bits == 0 || max_len_bits > 16 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid TwoDelta run-length bit width: {max_len_bits}"),
        ));
    }
    Ok(())
}

/// n_bytes consistency for a TwoDelta frame: the encoder writes `n_bytes = ceil(runs * width / 8)`.
/// Any other relationship between the payload length and the recovered run count is a
/// corrupt-frame signal, exactly mirroring the Standard/MkvChain payload check in
/// [`decode_ben_line`](crate::codec::decode::decode_ben_line).
pub(crate) fn check_twodelta_frame_consistency(
    n_bytes: u32,
    run_count: usize,
    max_len_bits: u8,
) -> io::Result<()> {
    let expected_bytes = (run_count as u64 * u64::from(max_len_bits)).div_ceil(8);
    if u64::from(n_bytes) != expected_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "inconsistent TwoDelta frame size: n_bytes={n_bytes} but {run_count} run \
                 length(s) at {max_len_bits} bit(s) each require {expected_bytes} byte(s)"
            ),
        ));
    }
    Ok(())
}

/// Unpack a TwoDelta frame's bit-packed run lengths, rejecting interior zeros.
///
/// The encoder never emits a zero run length, so a zero slot is legal only as a trailing
/// byte-padding artifact (when `max_len_bits` is narrow, the final byte's zero padding can form
/// one or more complete all-zero slots). A zero followed by a real run length is interior
/// corruption: silently dropping it would shift the alternation parity of every subsequent run
/// and decode to a plausible-but-wrong assignment, so it is rejected instead. This is the
/// TwoDelta analog of the interior-zero discipline in
/// [`decode_ben_line`](crate::codec::decode::decode_ben_line).
pub(crate) fn unpack_twodelta_run_lengths(
    payload: &[u8],
    max_len_bits: u8,
) -> io::Result<Vec<u16>> {
    let mut run_lengths = Vec::new();
    let mut buffer: u32 = 0;
    let mut n_bits_in_buff: u16 = 0;
    // Zero slots seen since the last real run length. Accepted as padding if the frame ends,
    // rejected as interior corruption if a real run length follows.
    let mut pending_zero_slots: usize = 0;

    for &byte in payload {
        // Place the incoming byte at the top of the 32-bit shift register, below any bits already
        // buffered; extraction always reads from the register's high end.
        buffer |= ((byte as u32) << 24) >> n_bits_in_buff;
        n_bits_in_buff += 8;

        while n_bits_in_buff >= max_len_bits as u16 {
            let item = (buffer >> (32 - max_len_bits)) as u16;
            buffer <<= max_len_bits;
            n_bits_in_buff -= max_len_bits as u16;
            if item == 0 {
                pending_zero_slots += 1;
            } else {
                if pending_zero_slots > 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "TwoDelta frame contains an interior zero run length; the encoder never \
                         emits zero-length runs",
                    ));
                }
                run_lengths.push(item);
            }
        }
    }

    Ok(run_lengths)
}

/// One sample's encoded bytes at the frame layer, freshly read from a wire stream.
///
/// `Standard` and `MkvChain` carry **opaque** bit-packed payload bytes: the runs are not expanded
/// until a caller asks for them. This is what makes frame-level subsampling cheap: the iterator can
/// pull frames at byte level and only the kept frames pay the bit-unpacking cost.
///
/// `TwoDelta` is the exception: applying a delta to the previous assignment requires the run-length
/// vector, so the decoder unpacks it eagerly at parse time. This is not a regression; the bytes
/// would have been needed immediately on use anyway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BenDecodeFrame {
    /// A `Standard`-variant frame with no trailing repetition count.
    Standard {
        /// The number of bits used to encode the maximum district id.
        max_val_bit_count: u8,
        /// The number of bits used to encode the maximum run length.
        max_len_bit_count: u8,
        /// The number of bytes in the packed payload.
        n_bytes: u32,
        /// The bit-packed payload bytes, opaque until `expand` is called.
        raw_bytes: Vec<u8>,
    },
    /// An `MkvChain`-variant frame carrying its repetition count.
    MkvChain {
        /// The number of bits used to encode the maximum district id.
        max_val_bit_count: u8,
        /// The number of bits used to encode the maximum run length.
        max_len_bit_count: u8,
        /// The number of bytes in the packed payload.
        n_bytes: u32,
        /// The bit-packed payload bytes, opaque until `expand` is called.
        raw_bytes: Vec<u8>,
        /// The number of times this frame repeats.
        count: u16,
    },
    /// A `TwoDelta`-variant delta frame. Run lengths are eagerly decoded at parse time because
    /// applying the delta needs them.
    TwoDelta {
        /// The pair of district ids encoded in this frame.
        pair: (u16, u16),
        /// The unpacked alternating run lengths over the positions occupied by the pair.
        run_lengths: Vec<u16>,
        /// The number of times this delta repeats.
        count: u16,
    },
}

impl BenDecodeFrame {
    /// Read the next frame in the wire format dictated by `variant`.
    ///
    /// Returns `Ok(None)` on a clean EOF at a frame boundary, `Ok(Some(frame))` on success, and
    /// `Err` on any I/O or format error.
    ///
    /// Note: in a `TwoDelta` *stream* the body layout is chosen per frame: snapshot frames are
    /// `MkvChain`-formatted and delta frames are `TwoDelta`-formatted. That choice is carried by a
    /// 1-byte tag the stream reader (e.g. [`BenStreamReader`]) consumes before calling this; it
    /// resolves the tag to a [`BenVariant`] and passes it here. This function reads the body for
    /// whatever variant it is given and is unaware of the tag.
    ///
    /// [`BenStreamReader`]: crate::io::reader::BenStreamReader
    pub fn from_reader(reader: &mut impl Read, variant: BenVariant) -> io::Result<Option<Self>> {
        match variant {
            BenVariant::Standard => Self::read_standard(reader),
            BenVariant::MkvChain => Self::read_mkv_chain(reader),
            BenVariant::TwoDelta => Self::read_twodelta(reader),
        }
    }

    fn read_standard(reader: &mut impl Read) -> io::Result<Option<Self>> {
        let max_val_bit_count = match reader.read_u8() {
            Ok(v) => v,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        };

        let max_len_bit_count = reader.read_u8()?;
        let n_bytes = reader.read_u32::<BigEndian>()?;
        check_payload_len(n_bytes)?;

        let mut raw_bytes = vec![0u8; n_bytes as usize];
        reader.read_exact(&mut raw_bytes)?;

        Ok(Some(Self::Standard {
            max_val_bit_count,
            max_len_bit_count,
            n_bytes,
            raw_bytes,
        }))
    }

    fn read_mkv_chain(reader: &mut impl Read) -> io::Result<Option<Self>> {
        let max_val_bit_count = match reader.read_u8() {
            Ok(v) => v,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        };

        let max_len_bit_count = reader.read_u8()?;
        let n_bytes = reader.read_u32::<BigEndian>()?;
        check_payload_len(n_bytes)?;

        let mut raw_bytes = vec![0u8; n_bytes as usize];
        reader.read_exact(&mut raw_bytes)?;

        let count = reader.read_u16::<BigEndian>()?;

        Ok(Some(Self::MkvChain {
            max_val_bit_count,
            max_len_bit_count,
            n_bytes,
            raw_bytes,
            count,
        }))
    }

    fn read_twodelta(reader: &mut impl Read) -> io::Result<Option<Self>> {
        let pair_a = match reader.read_u16::<BigEndian>() {
            Ok(v) => v,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        };

        let pair_b = reader.read_u16::<BigEndian>()?;
        let max_len_bits = reader.read_u8()?;
        check_twodelta_run_width(max_len_bits)?;
        let n_bytes = reader.read_u32::<BigEndian>()?;
        check_payload_len(n_bytes)?;

        let mut payload = vec![0u8; n_bytes as usize];
        reader.read_exact(&mut payload)?;

        let count = reader.read_u16::<BigEndian>()?;

        let pair = (pair_a, pair_b);
        let run_lengths = unpack_twodelta_run_lengths(&payload, max_len_bits)?;
        check_twodelta_frame_consistency(n_bytes, run_lengths.len(), max_len_bits)?;

        Ok(Some(Self::TwoDelta {
            pair,
            run_lengths,
            count,
        }))
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

    /// Borrow the bit-packed payload bytes for `Standard`/`MkvChain` arms. Returns `None` for
    /// `TwoDelta` (which doesn't keep raw bytes after parsing).
    pub fn raw_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Standard { raw_bytes, .. } | Self::MkvChain { raw_bytes, .. } => Some(raw_bytes),
            Self::TwoDelta { .. } => None,
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

    /// The bit width of the largest run length, or `None` for `TwoDelta` (whose width sat in the
    /// wire format but is not retained on decode).
    pub fn max_len_bit_count(&self) -> Option<u8> {
        match self {
            Self::Standard {
                max_len_bit_count, ..
            }
            | Self::MkvChain {
                max_len_bit_count, ..
            } => Some(*max_len_bit_count),
            Self::TwoDelta { .. } => None,
        }
    }

    /// The number of payload bytes for `Standard`/`MkvChain`, or `None` for `TwoDelta`.
    pub fn n_bytes(&self) -> Option<u32> {
        match self {
            Self::Standard { n_bytes, .. } | Self::MkvChain { n_bytes, .. } => Some(*n_bytes),
            Self::TwoDelta { .. } => None,
        }
    }

    /// The pair of district ids encoded by a `TwoDelta` frame, or `None` for the snapshot arms.
    pub fn pair(&self) -> Option<(u16, u16)> {
        match self {
            Self::TwoDelta { pair, .. } => Some(*pair),
            _ => None,
        }
    }

    /// Borrow the alternating run-length vector for a `TwoDelta` frame, or `None` for the snapshot
    /// arms.
    pub fn run_lengths(&self) -> Option<&[u16]> {
        match self {
            Self::TwoDelta { run_lengths, .. } => Some(run_lengths),
            _ => None,
        }
    }

    /// Materialize the frame as a full assignment vector.
    ///
    /// `Standard` and `MkvChain` ignore `prev` (any owned vector is dropped). `TwoDelta` consumes
    /// `prev` in place to apply the delta and returns an error if `prev` is `None`.
    pub fn expand(&self, prev: Option<Vec<u16>>) -> io::Result<Vec<u16>> {
        use crate::codec::decode::{
            apply_twodelta_runs_to_assignment, decode_ben_line, DecodeError,
        };
        use crate::util::rle::rle_to_vec;
        use std::io::Cursor;

        match self {
            Self::Standard {
                max_val_bit_count,
                max_len_bit_count,
                n_bytes,
                raw_bytes,
            }
            | Self::MkvChain {
                max_val_bit_count,
                max_len_bit_count,
                n_bytes,
                raw_bytes,
                ..
            } => decode_ben_line(
                Cursor::new(raw_bytes),
                *max_val_bit_count,
                *max_len_bit_count,
                *n_bytes,
            )
            .map(rle_to_vec),
            Self::TwoDelta {
                pair, run_lengths, ..
            } => {
                let prev =
                    prev.ok_or_else(|| io::Error::from(DecodeError::TwoDeltaNoAnchorFrame))?;
                apply_twodelta_runs_to_assignment(prev, *pair, run_lengths, None)
            }
        }
    }
}
