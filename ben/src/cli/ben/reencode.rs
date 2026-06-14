//! `ben reencode` handler: change the encoding without relabeling.
//!
//! At least one of `--output-variant`, `--collapse-runs`, or `--n-items` must be set. A re-encode
//! that would change nothing is rejected rather than emitting an identical copy.

use super::args::{Globals, ReencodeArgs};
use super::relabel_helpers::{ben_stem, ben_variant_name, write_or_in_place};
use crate::ops::relabel::{RelabelOptions, RunPolicy};
use std::fs::File;
use std::io::BufReader;

pub(super) fn run(args: ReencodeArgs, g: &Globals) -> Result<(), String> {
    if args.output_variant.is_none() && !args.collapse_runs && args.n_items.is_none() {
        return Err(
            "nothing to do: pass --output-variant, --collapse-runs, or --n-items.".to_string(),
        );
    }
    tracing::info!("Re-encoding BEN file.");

    let input_file = File::open(&args.input_file)
        .map_err(|e| format!("Could not open input file {:?}: {e}", args.input_file))?;
    let reader = BufReader::new(input_file);

    let output_variant = args.output_variant.map(|v| v.to_ben_variant());

    // Verbatim labels; `--collapse-runs` merges adjacent equal assignments, otherwise frame
    // boundaries are preserved. The optional target variant and sample limit ride along.
    let mut options = RelabelOptions::verbatim();
    if args.collapse_runs {
        options = options.with_run_policy(RunPolicy::CollapseAdjacentEqualAssignments);
    }
    if let Some(variant) = output_variant {
        options = options.with_target_variant(variant);
    }
    options = options.with_max_samples_opt(args.n_items);

    write_or_in_place(
        reader,
        &args.input_file,
        g.output_file.clone(),
        args.add_suffix,
        g.overwrite,
        || {
            let stem = ben_stem(&args.input_file);
            match output_variant {
                Some(variant) => format!("{stem}_{}.ben", ben_variant_name(variant)),
                None => format!("{stem}_reencode.ben"),
            }
        },
        options,
    )
}
