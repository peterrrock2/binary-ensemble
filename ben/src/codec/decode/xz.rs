use crate::codec::translate::ben32_to_ben_lines;
use crate::format::banners::{banner_for_variant, variant_from_banner, BANNER_LEN};
use crate::format::FormatError;
use crate::io::reader::BenStreamReader;
use crate::io::writer::BenStreamWriter;
use crate::progress::Spinner;
use crate::BenVariant;
use std::io::{self, BufRead, BufReader, Read, Write};
use xz2::read::XzDecoder;

/// Decode an XBEN stream into an equivalent BEN stream.
///
/// The output begins with the normal BEN banner followed by uncompressed BEN
/// frames.
///
/// # Arguments
///
/// * `reader` - The compressed XBEN input stream.
/// * `writer` - The destination for the uncompressed BEN stream.
///
/// # Returns
///
/// Returns `Ok(())` after the full XBEN stream has been decoded into BEN.
pub fn decode_xben_to_ben<R: BufRead, W: Write>(reader: R, mut writer: W) -> io::Result<()> {
    let mut decoder = XzDecoder::new(reader);

    let mut first_buffer = [0u8; BANNER_LEN];

    if let Err(e) = decoder.read_exact(&mut first_buffer) {
        return Err(e);
    }

    let variant = match variant_from_banner(&first_buffer) {
        Some(BenVariant::Standard) => {
            writer.write_all(banner_for_variant(BenVariant::Standard))?;
            BenVariant::Standard
        }
        Some(BenVariant::MkvChain) => {
            writer.write_all(banner_for_variant(BenVariant::MkvChain))?;
            BenVariant::MkvChain
        }
        Some(BenVariant::TwoDelta) => {
            let mut xben = BenStreamReader::from_xben_decompressed(
                BufReader::new(decoder),
                BenVariant::TwoDelta,
            );
            let mut ben = BenStreamWriter::for_ben(writer, BenVariant::TwoDelta)?;
            for record in &mut xben {
                let (assignment, count) = record?;
                ben.write_assignment(assignment.clone())?;
                for _ in 1..count {
                    ben.write_assignment(assignment.clone())?;
                }
            }
            ben.finish()?;
            return Ok(());
        }
        None => {
            return Err(io::Error::from(FormatError::UnknownBanner {
                actual: first_buffer.to_vec(),
            }));
        }
    };

    let mut buffer = [0u8; 1048576];
    let mut overflow: Vec<u8> = Vec::new();

    let mut line_count: usize = 0;
    let spinner = Spinner::new("Decoding sample");
    loop {
        let count = decoder.read(&mut buffer)?;
        if count == 0 {
            break;
        }

        overflow.extend(&buffer[..count]);

        let mut last_valid_assignment = 0;

        // TwoDelta was dispatched before this loop and returned early.
        if variant == BenVariant::Standard {
            for i in (3..overflow.len()).step_by(4) {
                if overflow[i - 3..=i] == [0, 0, 0, 0] {
                    last_valid_assignment = i + 1;
                    line_count += 1;
                    spinner.set_count(line_count as u64);
                }
            }
        } else {
            for i in (3..overflow.len() - 2).step_by(2) {
                if overflow[i - 3..=i] == [0, 0, 0, 0] {
                    last_valid_assignment = i + 3;
                    let lines = &overflow[i + 1..i + 3];
                    let n_lines = u16::from_be_bytes([lines[0], lines[1]]);
                    line_count += n_lines as usize;
                    spinner.set_count(line_count as u64);
                }
            }
        }

        if last_valid_assignment == 0 {
            continue;
        }

        ben32_to_ben_lines(&overflow[0..last_valid_assignment], &mut writer, variant)?;
        overflow = overflow[last_valid_assignment..].to_vec();
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
