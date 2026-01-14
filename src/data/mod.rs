//! Data persistence layer for Conduit
//!
//! This module provides SQLite-based storage for repositories and workspaces.

mod app_state;
mod coderabbit;
mod database;
mod fork_seed;
mod models;
mod repository;
mod session_tab;
mod workspace;

pub use app_state::AppStateStore;
pub use coderabbit::{
    CodeRabbitCommentStore, CodeRabbitItemStore, CodeRabbitRoundStore, RepositorySettingsStore,
};
pub use database::Database;
pub use fork_seed::ForkSeedStore;
pub use models::{
    CodeRabbitCategory, CodeRabbitComment, CodeRabbitFeedbackScope, CodeRabbitItem,
    CodeRabbitItemKind, CodeRabbitItemSource, CodeRabbitMode, CodeRabbitRetention,
    CodeRabbitReviewLoopDoneCondition, CodeRabbitRound, CodeRabbitRoundStatus, CodeRabbitSeverity,
    ForkSeed, QueuedImageAttachment, QueuedMessage, QueuedMessageMode, Repository,
    RepositorySettings, SessionTab, Workspace,
};
pub use repository::RepositoryStore;
pub use session_tab::SessionTabStore;
pub use workspace::WorkspaceStore;
