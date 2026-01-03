//! Path utilities for Conduit data directories

use std::path::PathBuf;

/// Get the base Conduit data directory (~/.conduit)
pub fn data_dir() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".conduit"))
        .unwrap_or_else(|| PathBuf::from(".conduit"))
}

/// Get the database file path (~/.conduit/conduit.db)
pub fn database_path() -> PathBuf {
    data_dir().join("conduit.db")
}

/// Get the logs directory (~/.conduit/logs)
pub fn logs_dir() -> PathBuf {
    data_dir().join("logs")
}

/// Get the default log file path (~/.conduit/logs/conduit.log)
pub fn log_file_path() -> PathBuf {
    logs_dir().join("conduit.log")
}

/// Get the worktrees directory (~/.conduit/worktrees)
pub fn worktrees_dir() -> PathBuf {
    data_dir().join("worktrees")
}

/// Get the config file path (~/.conduit/config.toml)
pub fn config_path() -> PathBuf {
    data_dir().join("config.toml")
}
