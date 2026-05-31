use super::args::ExtractArgs;
use crate::cli::common::check_overwrite;
use crate::io::bundle::BendlReader;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Write};

pub(super) fn run_extract(args: ExtractArgs) -> Result<(), String> {
    if !args.stream && args.asset.is_none() {
        return Err("extract requires either --stream or --asset <name>".to_string());
    }
    check_overwrite(
        args.output.to_str().ok_or("non-utf8 output path")?,
        args.overwrite,
    )
    .map_err(|e| format!("{e}"))?;

    let file =
        File::open(&args.input).map_err(|e| format!("failed to open {:?}: {e}", args.input))?;
    let mut reader = BendlReader::open(BufReader::new(file))
        .map_err(|e| format!("failed to parse bundle header: {e}"))?;

    if args.stream {
        let mut stream = if args.allow_unfinalized && !reader.is_finalized() {
            reader
                .assignment_stream_reader_unverified()
                .map_err(|e| format!("failed to open stream region: {e}"))?
        } else {
            reader
                .assignment_stream_reader()
                .map_err(|e| format!("failed to open stream region: {e}"))?
        };
        let mut out = BufWriter::new(
            File::create(&args.output)
                .map_err(|e| format!("failed to create {:?}: {e}", args.output))?,
        );
        io::copy(&mut stream, &mut out).map_err(|e| format!("failed to copy stream bytes: {e}"))?;
        out.flush().map_err(|e| format!("flush failed: {e}"))?;
    } else {
        // asset is Some — validated by the early return above.
        let name = args.asset.unwrap();
        let entry = reader
            .find_asset_by_name(&name)
            .cloned()
            .ok_or_else(|| format!("no asset named {name:?} in bundle"))?;
        let mut asset = reader
            .asset_reader(&entry)
            .map_err(|e| format!("failed to open asset {name:?}: {e}"))?;
        let mut out = BufWriter::new(
            File::create(&args.output)
                .map_err(|e| format!("failed to create {:?}: {e}", args.output))?,
        );
        io::copy(&mut asset, &mut out)
            .map_err(|e| format!("failed to copy asset {name:?} bytes: {e}"))?;
        out.flush().map_err(|e| format!("flush failed: {e}"))?;
    }

    Ok(())
}
