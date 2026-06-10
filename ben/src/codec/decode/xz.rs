use crate::codec::translate::ben32_to_ben_line;
use crate::format::banners::banner_for_variant;
use crate::io::reader::{BenStreamReader, DecodeFrame};
use crate::io::writer::BenStreamWriter;
use crate::progress::Spinner;
use crate::{BenVariant, XBenVariant};
use std::io::{self, BufRead, Read, Write};
use xz2::read::XzDecoder;

/// Decode an XBEN stream into an equivalent BEN stream.
///
/// The output begins with the normal BEN banner followed by uncompressed BEN frames.
///
/// `Standard` and `MkvChain` streams are translated frame-by-frame at the ben32 layer, which
/// preserves each frame's original run boundaries and never materializes assignment vectors.
/// `TwoDelta` streams use a different compressed layout, so their assignments are materialized and
/// re-encoded through the TwoDelta stream writer.
///
/// # Arguments
///
/// * `reader` - The compressed XBEN input stream.
/// * `writer` - The destination for the uncompressed BEN stream.
///
/// # Returns
///
/// Returns `Ok(())` after the full XBEN stream has been decoded into BEN.
///
/// # Errors
///
/// Surfaces an error (rather than a truncated result) if the decompressed stream ends partway
/// through a frame, declares a zero repetition count, or carries an unknown banner.
pub fn decode_xben_to_ben<R: BufRead, W: Write>(reader: R, mut writer: W) -> io::Result<()> {
    let xben = BenStreamReader::from_xben(reader).map_err(io::Error::from)?;
    let variant = xben.variant();

    if variant == BenVariant::TwoDelta {
        let mut ben = BenStreamWriter::for_ben(writer, BenVariant::TwoDelta)?;
        for record in xben {
            let (assignment, count) = record?;
            for _ in 0..count {
                ben.write_assignment(assignment.clone())?;
            }
        }
        ben.finish()?;
        return Ok(());
    }

    let xben_variant = match variant {
        BenVariant::Standard => XBenVariant::Standard,
        BenVariant::MkvChain => XBenVariant::MkvChain,
        BenVariant::TwoDelta => unreachable!("TwoDelta was dispatched above"),
    };
    writer.write_all(banner_for_variant(variant))?;

    let spinner = Spinner::new("Decoding sample");
    let mut sample_count = 0usize;
    for item in xben.into_frames() {
        let (frame, count) = item?;
        let mut frame_bytes = match frame {
            DecodeFrame::XBen(bytes, _) => bytes,
            DecodeFrame::Ben(_) => {
                unreachable!("an XBEN stream's frame iterator always yields ben32 frames")
            }
        };
        // A MkvChain ben32 frame carries its repetition count after the zero sentinel; the
        // translator takes the count as a separate argument, so drop it from the frame bytes.
        if xben_variant == XBenVariant::MkvChain {
            frame_bytes.truncate(frame_bytes.len() - 2);
        }
        writer.write_all(&ben32_to_ben_line(frame_bytes, xben_variant, count)?)?;
        sample_count += count as usize;
        spinner.set_count(sample_count as u64);
    }
    Ok(())
}

/// Decompress a general XZ byte stream without applying any BEN-specific logic.
///
/// # Arguments
///
/// * `reader` - The compressed XZ stream.
/// * `writer` - The destination for the decompressed bytes.
///
/// # Returns
///
/// Returns `Ok(())` once the compressed stream has been fully expanded.
pub fn xz_decompress<R: BufRead, W: Write>(reader: R, mut writer: W) -> io::Result<()> {
    let mut decoder = XzDecoder::new(reader);
    let mut buffer = [0u8; 4096];

    loop {
        let count = decoder.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        writer.write_all(&buffer[..count])?;
    }

    Ok(())
}
