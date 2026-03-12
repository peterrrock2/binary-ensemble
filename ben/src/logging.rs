use tracing::Level;
use tracing_subscriber::EnvFilter;
use std::sync::Once;

static INIT_LOGGER: Once = Once::new();

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

pub fn trace_progress(args: std::fmt::Arguments<'_>) {
    if tracing::enabled!(Level::TRACE) {
        eprint!("{args}");
    }
}
