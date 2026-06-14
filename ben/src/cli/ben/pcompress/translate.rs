//! BEN <-> PCOMPRESS assignment translation helpers.
//!
//! Both formats use zero-based district ids, so the bridge is a straight transcode with no id
//! shifting: it only reshapes between BEN frames and PCOMPRESS's one-JSON-array-per-line text.

use crate::io::reader::BenStreamReader;
use crate::io::writer::BenStreamWriter;
use crate::BenVariant;
use serde_json::json;
use std::io::{self, BufRead, Read, Write};
use xz2::write::XzEncoder;

/// Decode BEN and emit one assignment vector per line for PCOMPRESS.
pub(super) fn assignment_decode_ben<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
) -> io::Result<()> {
    let ben_reader = BenStreamReader::from_ben(&mut reader)?;
    let mut line = String::new();

    for result in ben_reader {
        let (assignment, count) = result?;
        render_pcompress_line(&assignment, &mut line);
        for _ in 0..count {
            writeln!(writer, "{line}")?;
        }
    }

    Ok(())
}

/// Render a BEN assignment vector as a PCOMPRESS JSON array. Ids are zero-based in both formats, so
/// they are emitted unchanged.
fn render_pcompress_line(assignment: &[u16], output: &mut String) {
    output.clear();
    output.push('[');
    for (idx, &value) in assignment.iter().enumerate() {
        if idx > 0 {
            output.push(',');
        }
        output.push_str(&value.to_string());
    }
    output.push(']');
}

/// Parse one PCOMPRESS line of district ids into a BEN assignment. Malformed JSON is surfaced as an
/// `InvalidData` error rather than a panic.
fn parse_ben_assignment(line: io::Result<String>) -> io::Result<Vec<u16>> {
    let line = line?;
    serde_json::from_str(&line).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("malformed PCOMPRESS assignment line: {e}"),
        )
    })
}

/// Read PCOMPRESS assignment vectors and encode them as BEN.
pub(super) fn assignment_encode_ben<R: Read + BufRead, W: Write>(
    reader: R,
    writer: W,
) -> io::Result<()> {
    let mut ben_writer = BenStreamWriter::for_ben(writer, BenVariant::MkvChain)?;

    for line in reader.lines() {
        ben_writer.write_assignment(parse_ben_assignment(line)?)?;
    }
    ben_writer.finish()?;
    Ok(())
}

/// Read PCOMPRESS assignment vectors and encode them as XBEN.
pub(super) fn assignment_encode_xben<R: Read + BufRead, W: Write>(
    reader: R,
    writer: W,
) -> io::Result<()> {
    let encoder = XzEncoder::new(writer, 9);
    let mut xben_writer =
        BenStreamWriter::for_xben_with_encoder(encoder, BenVariant::MkvChain, None)?;

    for line in reader.lines() {
        let assignment = parse_ben_assignment(line)?;
        xben_writer.write_json_value(json!({ "assignment": assignment }))?;
    }
    xben_writer.finish()?;

    Ok(())
}
