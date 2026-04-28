use std::path::Path;
use tracing_appender;
use tracing_subscriber::{
    EnvFilter,
    fmt::time::UtcTime,
    layer::{Layer, SubscriberExt},
    util::SubscriberInitExt,
};

pub fn init_logging(log_dir: &str) -> tracing_appender::non_blocking::WorkerGuard {
    let log_path = Path::new(log_dir);
    std::fs::create_dir_all(log_path).expect("Failed to create log directory");

    // File appender with daily rotation
    let file_appender = tracing_appender::rolling::daily(log_path, "trading-journal.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    // Console: pretty output, respects RUST_LOG, defaults to INFO
    let console_layer = tracing_subscriber::fmt::layer()
        .pretty()
        .with_timer(UtcTime::rfc_3339())
        .with_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")));

    // File (JSON): always logs DEBUG+ for your complete trading journal
    let file_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_writer(non_blocking)
        .with_timer(UtcTime::rfc_3339())
        .with_target(false)
        .with_current_span(false)
        .with_filter(EnvFilter::new("debug"));

    tracing_subscriber::registry()
        .with(file_layer) // full debug → journal file
        .with(console_layer) // clean console (info by default)
        .init();

    guard
}
