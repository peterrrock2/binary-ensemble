//! `ben canonicalize` handler: relabel districts in first-seen order, 0-based.

use super::args::{CanonicalizeArgs, Globals};
use super::relabel_helpers::{ben_stem, write_or_in_place};
use crate::ops::relabel::RelabelOptions;
use std::fs::File;
use std::io::BufReader;

pub(super) fn run(args: CanonicalizeArgs, g: &Globals) -> Result<(), String> {
    tracing::info!("Canonicalizing assignment vectors in ben file.");

    let input_file = File::open(&args.input_file)
        .map_err(|e| format!("Could not open input file {:?}: {e}", args.input_file))?;
    let reader = BufReader::new(input_file);

    let output_variant = args.output_variant.map(|v| v.to_ben_variant());
    let options = match output_variant {
        Some(variant) => RelabelOptions::first_seen().with_target_variant(variant),
        None => RelabelOptions::first_seen(),
    }
    .with_max_samples_opt(args.n_items);

    write_or_in_place(
        reader,
        &args.input_file,
        g.output_file.clone(),
        args.add_suffix,
        g.overwrite,
        || format!("{}_first_seen_relabeled.ben", ben_stem(&args.input_file)),
        options,
    )
}
