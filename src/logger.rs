use anyhow::Result;
use std::path::Path;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

pub fn setup_logger(log_file: &str) -> Result<()> {
    let log_path = Path::new(log_file);
    let log_dir = log_path.parent().unwrap_or(Path::new("logs"));
    let log_name = log_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("proxy.log");

    std::fs::create_dir_all(log_dir)?;

    let file_appender = RollingFileAppender::new(Rotation::DAILY, log_dir, log_name);
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    Box::leak(Box::new(guard));

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(env_filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .compact(),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(non_blocking)
                .with_ansi(false)
                .with_target(false)
                .compact(),
        )
        .init();

    Ok(())
}
