use std::sync::Once;
use tracing_subscriber::EnvFilter;

static INIT_LOGGER: Once = Once::new();

/// Initialize the global `tracing` subscriber used by the BEN CLIs.
///
/// The subscriber reads `RUST_LOG` when present and otherwise defaults to
/// logging being disabled. Initialization is guarded so it is safe to call
/// multiple times.
///
/// # Returns
///
/// This function does not return a value. Repeated calls after the first are
/// no-ops.
pub fn init_logging() {
    INIT_LOGGER.call_once(|| {
        let filter = EnvFilter::try_from_default_env()
            .or_else(|_| EnvFilter::try_new("off"))
            .expect("valid fallback log filter");

        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .without_time()
            .with_target(false)
            .with_level(false)
            .with_ansi(false)
            .event_format(tracing_subscriber::fmt::format().compact())
            .finish();

        let _ = tracing::subscriber::set_global_default(subscriber);
    });
}
