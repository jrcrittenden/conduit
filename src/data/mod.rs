//! Data persistence layer for Conduit
//!
//! This module provides SQLite-based storage for repositories and workspaces.

mod app_state_dao;
mod database;
mod models;
mod repository_dao;
mod session_tab_dao;
mod workspace_dao;

pub use app_state_dao::AppStateDao;
pub use database::Database;
pub use models::{Repository, SessionTab, Workspace};
pub use repository_dao::RepositoryDao;
pub use session_tab_dao::SessionTabDao;
pub use workspace_dao::WorkspaceDao;
