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

/// Get the workspaces directory (~/.conduit/workspaces)
pub fn workspaces_dir() -> PathBuf {
    data_dir().join("workspaces")
}

/// Migrate old worktrees folder to workspaces folder if needed
///
/// This is a one-time migration for users upgrading from older versions.
/// If ~/.conduit/worktrees exists and ~/.conduit/workspaces doesn't,
/// we rename the folder.
pub fn migrate_worktrees_to_workspaces() {
    let old_path = data_dir().join("worktrees");
    let new_path = data_dir().join("workspaces");

    if old_path.exists() && !new_path.exists() {
        match std::fs::rename(&old_path, &new_path) {
            Ok(()) => {
                tracing::info!(
                    old = %old_path.display(),
                    new = %new_path.display(),
                    "Migrated worktrees folder to workspaces"
                );
            }
            Err(e) => {
                tracing::warn!(
                    old = %old_path.display(),
                    new = %new_path.display(),
                    error = %e,
                    "Failed to migrate worktrees folder to workspaces"
                );
            }
        }
    }
}

/// Get the config file path (~/.conduit/config.toml)
pub fn config_path() -> PathBuf {
    data_dir().join("config.toml")
}
