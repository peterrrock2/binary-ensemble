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
                render_zero_based_assignment_line(&assignment, &mut line);
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
fn render_zero_based_assignment_line(assignment: &[u16], output: &mut String) {
    output.clear();
    output.push('[');
    for (idx, value) in assignment.iter().enumerate() {
        if idx > 0 {
            output.push(',');
        }
        output.push_str(&value.saturating_sub(1).to_string());
    }
    output.push(']');
}

/// Read zero-based assignment vectors and encode them as BEN.
pub(super) fn assignment_encode_ben<R: Read + BufRead, W: Write>(
    reader: R,
    writer: W,
) -> io::Result<()> {
    let mut ben_writer = BenStreamWriter::for_ben(writer, BenVariant::MkvChain)?;

    for line in reader.lines() {
        let assignment: Vec<u16> = serde_json::from_str::<Vec<u16>>(&line.unwrap())
            .unwrap()
            .into_iter()
            .map(|x| x as u16 + 1)
            .collect();
        ben_writer.write_assignment(assignment)?;
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
        let assignment: Vec<u16> = serde_json::from_str::<Vec<u16>>(&line.unwrap())
            .unwrap()
            .into_iter()
            .map(|x| x as u16 + 1)
            .collect();
        xben_writer.write_json_value(json!({ "assignment": assignment }))?;
    }
    xben_writer.finish()?;

    Ok(())
}
