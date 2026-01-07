//! Utility modules

pub mod names;
pub mod paths;

pub use names::{generate_branch_name, generate_workspace_name, get_git_username};
pub use paths::{
    data_dir, database_path, log_file_path, logs_dir, migrate_worktrees_to_workspaces,
    workspaces_dir,
};
