//! In-place progress spinners for streaming encode/decode/relabel loops.
//!
//! Streaming operations have no upfront totals (BEN/JSONL inputs are read
//! frame-by-frame), so a percentage bar is not possible — this module
//! provides a running-counter spinner instead. The spinner writes directly
//! to stderr via [`indicatif`], bypassing `tracing` (whose fmt subscriber
//! appends `\n` and would defeat carriage-return redraws).
//!
//! Visibility is gated by two checks performed at construction time:
//! 1. `cli::common::is_quiet()` — the `--quiet` CLI flag.
//! 2. `std::io::stderr().is_terminal()` — auto-disable when stderr is
//!    redirected, so logs and pipelines stay clean.
//!
//! Both checks happen once in [`Spinner::new`]; the resulting [`Spinner`]
//! is either a live indicatif bar or a no-op stub.

use std::io::IsTerminal;
use std::time::Duration;

use indicatif::{ProgressBar, ProgressStyle};

/// A scope-bound progress spinner backed by [`indicatif::ProgressBar`].
///
/// The spinner animates on a steady tick and exposes a single counter via
/// [`Spinner::set_count`]. On drop, the spinner clears its line so that
/// subsequent stderr writes start fresh.
pub struct Spinner {
    bar: Option<ProgressBar>,
}

impl Spinner {
    /// Build a spinner for a streaming operation.
    ///
    /// Returns a no-op spinner when `--quiet` is set or when stderr is not
    /// a TTY.
    ///
    /// # Arguments
    ///
    /// * `prefix` - The label shown before the running counter, e.g.
    ///   `"Encoding line"`.
    ///
    /// # Returns
    ///
    /// A [`Spinner`] that may or may not have an active indicatif bar.
    pub fn new(prefix: &'static str) -> Self {
        if crate::cli::common::is_quiet() || !std::io::stderr().is_terminal() {
            return Self { bar: None };
        }

        let template = format!("{{spinner}} {prefix}: {{pos}}");
        let style = ProgressStyle::with_template(&template)
            .unwrap_or_else(|_| ProgressStyle::default_spinner());

        let bar = ProgressBar::new_spinner().with_style(style);
        bar.enable_steady_tick(Duration::from_millis(80));

        Self { bar: Some(bar) }
    }

    /// Update the running counter. No-op when the spinner is disabled.
    ///
    /// # Arguments
    ///
    /// * `n` - The new counter value to display.
    ///
    /// # Returns
    ///
    /// This function does not return a value.
    pub fn set_count(&self, n: u64) {
        if let Some(bar) = &self.bar {
            bar.set_position(n);
        }
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        if let Some(bar) = self.bar.take() {
            bar.finish_and_clear();
        }
    }
}
