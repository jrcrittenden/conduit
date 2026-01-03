use anyhow::Result;
use conduit::{util, App, Config};
use std::fs::{self, OpenOptions};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging to file (~/.conduit/logs/conduit.log)
    fs::create_dir_all(util::logs_dir())?;

    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(util::log_file_path())?;

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::WARN.into()),
        )
        .with_writer(log_file)
        .with_ansi(false) // Disable ANSI colors in log file
        .init();

    // Create config
    let config = Config::default();

    // Create and run app
    let mut app = App::new(config);
    app.run().await
}
