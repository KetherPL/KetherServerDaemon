// SPDX-License-Identifier: GPL-3.0-only
use tracing_subscriber::{fmt, EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

/// Initialize tracing subscriber with configuration
pub fn setup_logging(log_level: &str) -> anyhow::Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(log_level))
        .unwrap_or_else(|_| EnvFilter::new("info"));
    
    tracing_subscriber::registry()
        .with(filter)
        .with(
            fmt::layer()
                .with_ansi(true)
                .with_target(true)
                .with_file(true)
                .with_line_number(true)
        )
        .try_init()?;
    
    Ok(())
}

