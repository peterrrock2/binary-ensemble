//! BEN ↔ PCOMPRESS assignment translation helpers.
//!
//! PCOMPRESS uses zero-based district ids; BEN uses one-based. These helpers bridge the two
//! conventions so the per-mode handlers can be kept short.

use crate::io::reader::BenStreamReader;
use crate::io::writer::BenStreamWriter;
use crate::BenVariant;
use serde_json::json;
use std::io::{self, BufRead, Read, Write};
use xz2::write::XzEncoder;

/// Decode BEN and emit one zero-based assignment vector per line for PCOMPRESS.
pub(super) fn assignment_decode_ben<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
) -> io::Result<()> {
    let ben_reader = BenStreamReader::from_ben(&mut reader)?;
    let mut line = String::new();

    for result in ben_reader {
        match result {
            Ok((assignment, count)) => {
                render_zero_based_assignment_line(&assignment, &mut line)?;
                for _ in 0..count {
                    writeln!(writer, "{line}")?;
                }
            }
            Err(e) => return Err(e),
        }
    }

    Ok(())
}

/// Render a BEN assignment vector as a zero-based JSON array for PCOMPRESS.
///
/// BEN district ids are one-based in the PCOMPRESS convention; id `0` has no zero-based
/// counterpart, so it is rejected rather than silently aliased onto id `1`.
fn render_zero_based_assignment_line(assignment: &[u16], output: &mut String) -> io::Result<()> {
    output.clear();
    output.push('[');
    for (idx, &value) in assignment.iter().enumerate() {
        let Some(zero_based) = value.checked_sub(1) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "district id 0 cannot be converted to PCOMPRESS's zero-based ids; \
                 relabel the BEN stream to one-based ids first",
            ));
        };
        if idx > 0 {
            output.push(',');
        }
        output.push_str(&zero_based.to_string());
    }
    output.push(']');
    Ok(())
}

/// Parse one PCOMPRESS line of zero-based district ids into a one-based BEN assignment.
///
/// Malformed JSON and the unconvertible id `65535` (whose one-based form overflows `u16`) are
/// surfaced as `InvalidData` errors rather than panics or silent wraparound.
fn parse_one_based_assignment(line: io::Result<String>) -> io::Result<Vec<u16>> {
    let line = line?;
    let zero_based: Vec<u16> = serde_json::from_str(&line).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("malformed PCOMPRESS assignment line: {e}"),
        )
    })?;
    zero_based
        .into_iter()
        .map(|x| {
            x.checked_add(1).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "district id 65535 cannot be converted to BEN's one-based ids",
                )
            })
        })
        .collect()
}

/// Read zero-based assignment vectors and encode them as BEN.
pub(super) fn assignment_encode_ben<R: Read + BufRead, W: Write>(
    reader: R,
    writer: W,
) -> io::Result<()> {
    let mut ben_writer = BenStreamWriter::for_ben(writer, BenVariant::MkvChain)?;

    for line in reader.lines() {
        ben_writer.write_assignment(parse_one_based_assignment(line)?)?;
    }
    ben_writer.finish()?;
    Ok(())
}

/// Read zero-based assignment vectors and encode them as XBEN.
pub(super) fn assignment_encode_xben<R: Read + BufRead, W: Write>(
    reader: R,
    writer: W,
) -> io::Result<()> {
    let encoder = XzEncoder::new(writer, 9);
    let mut xben_writer =
        BenStreamWriter::for_xben_with_encoder(encoder, BenVariant::MkvChain, None)?;

    for line in reader.lines() {
        let assignment = parse_one_based_assignment(line)?;
        xben_writer.write_json_value(json!({ "assignment": assignment }))?;
    }
    xben_writer.finish()?;

    Ok(())
}
