use super::args::InspectArgs;
use crate::io::bundle::format::{
    AssignmentFormat, ASSET_FLAG_CHECKSUM, ASSET_FLAG_JSON, ASSET_FLAG_XZ,
};
use crate::io::bundle::BendlReader;
use std::fs::File;
use std::io::BufReader;

pub(super) fn run_inspect(args: InspectArgs) -> Result<(), String> {
    let file =
        File::open(&args.input).map_err(|e| format!("failed to open {:?}: {e}", args.input))?;
    let reader = BendlReader::open(BufReader::new(file))
        .map_err(|e| format!("failed to parse bundle header: {e}"))?;

    let header = reader.header();
    println!("file:              {}", args.input.display());
    println!(
        "version:           {}.{}",
        header.major_version, header.minor_version
    );
    println!("finalized:         {}", reader.is_finalized());
    println!(
        "assignment_format: {}",
        match reader.assignment_format() {
            Some(AssignmentFormat::Ben) => "ben",
            Some(AssignmentFormat::Xben) => "xben",
            None => "unknown",
        }
    );
    println!(
        "sample_count:      {}",
        match reader.sample_count() {
            Some(n) => n.to_string(),
            None => "<unknown>".to_string(),
        }
    );
    println!(
        "stream:            offset={} len={}",
        header.stream_offset, header.stream_len
    );
    println!(
        "directory:         offset={} len={}",
        header.directory_offset, header.directory_len
    );

    let entries = reader.assets();
    println!("assets:            {} entries", entries.len());
    for entry in entries {
        let mut flag_parts: Vec<&str> = Vec::new();
        if entry.asset_flags & ASSET_FLAG_JSON != 0 {
            flag_parts.push("json");
        }
        if entry.asset_flags & ASSET_FLAG_XZ != 0 {
            flag_parts.push("xz");
        }
        if entry.asset_flags & ASSET_FLAG_CHECKSUM != 0 {
            flag_parts.push("checksum");
        }
        let flag_str = if flag_parts.is_empty() {
            "-".to_string()
        } else {
            flag_parts.join(",")
        };
        println!(
            "  type={:<4} name={:<24} offset={:<10} len={:<10} flags={}",
            entry.asset_type, entry.name, entry.payload_offset, entry.payload_len, flag_str
        );
    }

    Ok(())
}
