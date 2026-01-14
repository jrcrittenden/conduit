//! SQLite database management

use rusqlite::{params, Connection, Result as SqliteResult};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use thiserror::Error;

/// SQL schema for creating tables
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS repositories (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    base_path TEXT,
    repository_url TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS workspaces (
    id TEXT PRIMARY KEY,
    repository_id TEXT NOT NULL,
    name TEXT NOT NULL,
    branch TEXT NOT NULL,
    path TEXT NOT NULL,
    created_at TEXT NOT NULL,
    last_accessed TEXT NOT NULL,
    is_default INTEGER NOT NULL DEFAULT 0,
    archived_at TEXT,
    archived_commit_sha TEXT,
    FOREIGN KEY (repository_id) REFERENCES repositories(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_workspaces_repository ON workspaces(repository_id);

CREATE TABLE IF NOT EXISTS app_state (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS session_tabs (
    id TEXT PRIMARY KEY,
    tab_index INTEGER NOT NULL,
    workspace_id TEXT,
    agent_type TEXT NOT NULL,
    agent_mode TEXT DEFAULT 'build',
    agent_session_id TEXT,
    model TEXT,
    pr_number INTEGER,
    created_at TEXT NOT NULL,
    pending_user_message TEXT,
    queued_messages TEXT NOT NULL DEFAULT '[]',
    input_history TEXT NOT NULL DEFAULT '[]',
    fork_seed_id TEXT,
    title TEXT,
    FOREIGN KEY (workspace_id) REFERENCES workspaces(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_session_tabs_order ON session_tabs(tab_index);

CREATE TABLE IF NOT EXISTS fork_seeds (
    id TEXT PRIMARY KEY,
    agent_type TEXT NOT NULL,
    parent_session_id TEXT,
    parent_workspace_id TEXT,
    created_at TEXT NOT NULL,
    seed_prompt_hash TEXT NOT NULL,
    seed_prompt_path TEXT,
    token_estimate INTEGER NOT NULL,
    context_window INTEGER NOT NULL,
    seed_ack_filtered INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (parent_workspace_id) REFERENCES workspaces(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_fork_seeds_parent_session ON fork_seeds(parent_session_id);

CREATE TABLE IF NOT EXISTS repository_settings (
    repository_id TEXT PRIMARY KEY,
    coderabbit_mode TEXT NOT NULL DEFAULT 'auto',
    coderabbit_retention TEXT NOT NULL DEFAULT 'keep',
    coderabbit_scope TEXT NOT NULL DEFAULT 'all',
    coderabbit_backoff_seconds TEXT NOT NULL DEFAULT '30,120,300',
    coderabbit_review_loop_enabled INTEGER NOT NULL DEFAULT 0,
    coderabbit_review_loop_scope TEXT NOT NULL DEFAULT 'all',
    coderabbit_review_loop_done_condition TEXT NOT NULL DEFAULT 'actionable-zero',
    coderabbit_review_loop_ask_before_enqueue INTEGER NOT NULL DEFAULT 1,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (repository_id) REFERENCES repositories(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS coderabbit_rounds (
    id TEXT PRIMARY KEY,
    repository_id TEXT NOT NULL,
    workspace_id TEXT,
    pr_number INTEGER NOT NULL,
    head_sha TEXT NOT NULL,
    check_state TEXT NOT NULL,
    check_started_at TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    status TEXT NOT NULL,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    next_fetch_at TEXT,
    actionable_count INTEGER NOT NULL DEFAULT 0,
    total_count INTEGER NOT NULL DEFAULT 0,
    completed_at TEXT,
    notified_at TEXT,
    processed_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (repository_id) REFERENCES repositories(id) ON DELETE CASCADE,
    FOREIGN KEY (workspace_id) REFERENCES workspaces(id) ON DELETE SET NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_coderabbit_rounds_unique
    ON coderabbit_rounds(repository_id, pr_number, head_sha, check_started_at);
CREATE INDEX IF NOT EXISTS idx_coderabbit_rounds_pending
    ON coderabbit_rounds(status, next_fetch_at);

CREATE TABLE IF NOT EXISTS coderabbit_items (
    id TEXT PRIMARY KEY,
    round_id TEXT NOT NULL,
    comment_id INTEGER NOT NULL,
    source TEXT NOT NULL,
    kind TEXT NOT NULL,
    actionable INTEGER NOT NULL DEFAULT 1,
    category TEXT,
    severity TEXT,
    section TEXT,
    file_path TEXT,
    line INTEGER,
    line_start INTEGER,
    line_end INTEGER,
    original_line INTEGER,
    diff_hunk TEXT,
    html_url TEXT NOT NULL,
    body TEXT NOT NULL,
    agent_prompt TEXT,
    item_key TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (round_id) REFERENCES coderabbit_rounds(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_coderabbit_items_unique
    ON coderabbit_items(round_id, item_key);

CREATE TABLE IF NOT EXISTS coderabbit_comments (
    id TEXT PRIMARY KEY,
    round_id TEXT NOT NULL,
    comment_id INTEGER NOT NULL,
    source TEXT NOT NULL,
    html_url TEXT NOT NULL,
    body TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (round_id) REFERENCES coderabbit_rounds(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_coderabbit_comments_unique
    ON coderabbit_comments(round_id, comment_id, source);
"#;

#[derive(Error, Debug)]
pub enum DatabaseError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Failed to determine data directory")]
    NoDataDir,
    #[error("Failed to create data directory: {0}")]
    CreateDir(std::io::Error),
    #[error("Lock poisoned")]
    LockPoisoned,
}

/// Database connection wrapper
#[derive(Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
    /// Path to the database file
    pub path: PathBuf,
}

fn hash_seed_prompt(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

impl Database {
    /// Open or create a database at the specified path
    pub fn open(path: PathBuf) -> Result<Self, DatabaseError> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(DatabaseError::CreateDir)?;
        }

        let conn = Connection::open(&path)?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;

        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
            path,
        };

        db.initialize()?;
        Ok(db)
    }

    /// Open database in the default location (~/.conduit/conduit.db)
    pub fn open_default() -> Result<Self, DatabaseError> {
        Self::open(crate::util::database_path())
    }

    /// Initialize the database schema
    fn initialize(&self) -> Result<(), DatabaseError> {
        let conn = self.conn.lock().map_err(|_| DatabaseError::LockPoisoned)?;
        conn.execute_batch(SCHEMA)?;
        drop(conn);

        // Apply migrations for existing databases
        self.apply_migrations()?;
        Ok(())
    }

    /// Apply database migrations for existing databases
    fn apply_migrations(&self) -> Result<(), DatabaseError> {
        let conn = self.conn.lock().map_err(|_| DatabaseError::LockPoisoned)?;

        // Migration 1: Add archived_at column to workspaces table
        let has_archived_at: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('workspaces') WHERE name='archived_at'",
                [],
                |row| row.get::<_, i64>(0).map(|c| c > 0),
            )
            .unwrap_or(false);

        if !has_archived_at {
            conn.execute("ALTER TABLE workspaces ADD COLUMN archived_at TEXT", [])?;
        }

        // Migration 2: Add pr_number column to session_tabs table
        let has_pr_number: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('session_tabs') WHERE name='pr_number'",
                [],
                |row| row.get::<_, i64>(0).map(|c| c > 0),
            )
            .unwrap_or(false);

        if !has_pr_number {
            conn.execute("ALTER TABLE session_tabs ADD COLUMN pr_number INTEGER", [])?;
        }

        // Migration 3: Add pending_user_message column to session_tabs table
        let has_pending_user_message: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('session_tabs') WHERE name='pending_user_message'",
                [],
                |row| row.get::<_, i64>(0).map(|c| c > 0),
            )
            .unwrap_or(false);

        if !has_pending_user_message {
            conn.execute(
                "ALTER TABLE session_tabs ADD COLUMN pending_user_message TEXT",
                [],
            )?;
        }

        // Migration 4: Add agent_mode column to session_tabs table
        let has_agent_mode: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('session_tabs') WHERE name='agent_mode'",
                [],
                |row| row.get::<_, i64>(0).map(|c| c > 0),
            )
            .unwrap_or(false);

        if !has_agent_mode {
            conn.execute(
                "ALTER TABLE session_tabs ADD COLUMN agent_mode TEXT DEFAULT 'build'",
                [],
            )?;
        }

        // Migration 5: Add archived_commit_sha column to workspaces table
        let has_archived_commit_sha: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('workspaces') WHERE name='archived_commit_sha'",
                [],
                |row| row.get::<_, i64>(0).map(|c| c > 0),
            )
            .unwrap_or(false);

        if !has_archived_commit_sha {
            conn.execute(
                "ALTER TABLE workspaces ADD COLUMN archived_commit_sha TEXT",
                [],
            )?;
        }

        // Migration 6: Add queued_messages column to session_tabs table
        let has_queued_messages: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('session_tabs') WHERE name='queued_messages'",
                [],
                |row| row.get::<_, i64>(0).map(|c| c > 0),
            )
            .unwrap_or(false);

        if !has_queued_messages {
            conn.execute(
                "ALTER TABLE session_tabs ADD COLUMN queued_messages TEXT NOT NULL DEFAULT '[]'",
                [],
            )?;
        }

        conn.execute(
            "UPDATE session_tabs SET queued_messages = '[]' WHERE queued_messages IS NULL",
            [],
        )?;

        // Migration 7: Add fork_seed_id column to session_tabs table
        let has_fork_seed_id: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('session_tabs') WHERE name='fork_seed_id'",
                [],
                |row| row.get::<_, i64>(0).map(|c| c > 0),
            )
            .unwrap_or(false);

        if !has_fork_seed_id {
            conn.execute("ALTER TABLE session_tabs ADD COLUMN fork_seed_id TEXT", [])?;
        }

        // Migration 7: Replace fork_seeds seed_prompt_text with seed_prompt_hash/path
        let fork_seeds_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='fork_seeds'",
                [],
                |row| row.get::<_, i64>(0).map(|c| c > 0),
            )
            .unwrap_or(false);

        if fork_seeds_exists {
            let has_seed_prompt_hash: bool = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('fork_seeds') WHERE name='seed_prompt_hash'",
                    [],
                    |row| row.get::<_, i64>(0).map(|c| c > 0),
                )
                .unwrap_or(false);

            if !has_seed_prompt_hash {
                // Verify old schema has seed_prompt_text column before attempting migration
                let has_seed_prompt_text: bool = conn
                    .query_row(
                        "SELECT COUNT(*) FROM pragma_table_info('fork_seeds') WHERE name='seed_prompt_text'",
                        [],
                        |row| row.get::<_, i64>(0).map(|c| c > 0),
                    )
                    .unwrap_or(false);

                if !has_seed_prompt_text {
                    // Old schema doesn't match expectations, skip migration
                    tracing::warn!(
                        "fork_seeds table exists but lacks seed_prompt_text column; skipping migration"
                    );
                    return Ok(());
                }

                conn.execute_batch(
                    r#"
CREATE TABLE IF NOT EXISTS fork_seeds_new (
    id TEXT PRIMARY KEY,
    agent_type TEXT NOT NULL,
    parent_session_id TEXT,
    parent_workspace_id TEXT,
    created_at TEXT NOT NULL,
    seed_prompt_hash TEXT NOT NULL,
    seed_prompt_path TEXT,
    token_estimate INTEGER NOT NULL,
    context_window INTEGER NOT NULL,
    seed_ack_filtered INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (parent_workspace_id) REFERENCES workspaces(id) ON DELETE SET NULL
);
"#,
                )?;

                let mut stmt = conn.prepare(
                    "SELECT id, agent_type, parent_session_id, parent_workspace_id, created_at, seed_prompt_text, token_estimate, context_window, seed_ack_filtered FROM fork_seeds",
                )?;
                let rows = stmt.query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)?,
                    ))
                })?;

                for row in rows {
                    let (
                        id,
                        agent_type,
                        parent_session_id,
                        parent_workspace_id,
                        created_at,
                        seed_prompt_text,
                        token_estimate,
                        context_window,
                        seed_ack_filtered,
                    ) = row?;

                    let seed_prompt_hash = hash_seed_prompt(&seed_prompt_text);
                    conn.execute(
                        "INSERT INTO fork_seeds_new (id, agent_type, parent_session_id, parent_workspace_id, created_at, seed_prompt_hash, seed_prompt_path, token_estimate, context_window, seed_ack_filtered)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                        params![
                            id,
                            agent_type,
                            parent_session_id,
                            parent_workspace_id,
                            created_at,
                            seed_prompt_hash,
                            Option::<String>::None,
                            token_estimate,
                            context_window,
                            seed_ack_filtered,
                        ],
                    )?;
                }

                conn.execute("DROP TABLE fork_seeds", [])?;
                conn.execute("ALTER TABLE fork_seeds_new RENAME TO fork_seeds", [])?;
                conn.execute(
                    "CREATE INDEX IF NOT EXISTS idx_fork_seeds_parent_session ON fork_seeds(parent_session_id)",
                    [],
                )?;
            }
        }

        // Migration 8: Add title column to session_tabs table
        let has_title: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('session_tabs') WHERE name='title'",
                [],
                |row| row.get::<_, i64>(0).map(|c| c > 0),
            )
            .unwrap_or(false);

        if !has_title {
            conn.execute("ALTER TABLE session_tabs ADD COLUMN title TEXT", [])?;
        }

        // Migration 9: Add input_history column to session_tabs table
        let has_input_history: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('session_tabs') WHERE name='input_history'",
                [],
                |row| row.get::<_, i64>(0).map(|c| c > 0),
            )
            .unwrap_or(false);

        if !has_input_history {
            conn.execute(
                "ALTER TABLE session_tabs ADD COLUMN input_history TEXT NOT NULL DEFAULT '[]'",
                [],
            )?;
        }

        conn.execute(
            "UPDATE session_tabs SET input_history = '[]' WHERE input_history IS NULL",
            [],
        )?;

        // Migration 10: Add repository_settings table
        let repo_settings_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='repository_settings'",
                [],
                |row| row.get::<_, i64>(0).map(|c| c > 0),
            )
            .unwrap_or(false);

        if !repo_settings_exists {
            conn.execute_batch(
                r#"
CREATE TABLE IF NOT EXISTS repository_settings (
    repository_id TEXT PRIMARY KEY,
    coderabbit_mode TEXT NOT NULL DEFAULT 'auto',
    coderabbit_retention TEXT NOT NULL DEFAULT 'keep',
    coderabbit_scope TEXT NOT NULL DEFAULT 'all',
    coderabbit_backoff_seconds TEXT NOT NULL DEFAULT '30,120,300',
    coderabbit_review_loop_enabled INTEGER NOT NULL DEFAULT 0,
    coderabbit_review_loop_scope TEXT NOT NULL DEFAULT 'all',
    coderabbit_review_loop_done_condition TEXT NOT NULL DEFAULT 'actionable-zero',
    coderabbit_review_loop_ask_before_enqueue INTEGER NOT NULL DEFAULT 1,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (repository_id) REFERENCES repositories(id) ON DELETE CASCADE
);
"#,
            )?;
        }

        // Migration 11: Add coderabbit_rounds table
        let coderabbit_rounds_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='coderabbit_rounds'",
                [],
                |row| row.get::<_, i64>(0).map(|c| c > 0),
            )
            .unwrap_or(false);

        if !coderabbit_rounds_exists {
            conn.execute_batch(
                r#"
CREATE TABLE IF NOT EXISTS coderabbit_rounds (
    id TEXT PRIMARY KEY,
    repository_id TEXT NOT NULL,
    workspace_id TEXT,
    pr_number INTEGER NOT NULL,
    head_sha TEXT NOT NULL,
    check_state TEXT NOT NULL,
    check_started_at TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    status TEXT NOT NULL,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    next_fetch_at TEXT,
    actionable_count INTEGER NOT NULL DEFAULT 0,
    total_count INTEGER NOT NULL DEFAULT 0,
    completed_at TEXT,
    notified_at TEXT,
    processed_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (repository_id) REFERENCES repositories(id) ON DELETE CASCADE,
    FOREIGN KEY (workspace_id) REFERENCES workspaces(id) ON DELETE SET NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_coderabbit_rounds_unique
    ON coderabbit_rounds(repository_id, pr_number, head_sha, check_started_at);
CREATE INDEX IF NOT EXISTS idx_coderabbit_rounds_pending
    ON coderabbit_rounds(status, next_fetch_at);
"#,
            )?;
        }

        // Migration 12: Add coderabbit_items table
        let coderabbit_items_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='coderabbit_items'",
                [],
                |row| row.get::<_, i64>(0).map(|c| c > 0),
            )
            .unwrap_or(false);

        if !coderabbit_items_exists {
            conn.execute_batch(
                r#"
CREATE TABLE IF NOT EXISTS coderabbit_items (
    id TEXT PRIMARY KEY,
    round_id TEXT NOT NULL,
    comment_id INTEGER NOT NULL,
    source TEXT NOT NULL,
    kind TEXT NOT NULL,
    actionable INTEGER NOT NULL DEFAULT 1,
    category TEXT,
    severity TEXT,
    section TEXT,
    file_path TEXT,
    line INTEGER,
    line_start INTEGER,
    line_end INTEGER,
    original_line INTEGER,
    diff_hunk TEXT,
    html_url TEXT NOT NULL,
    body TEXT NOT NULL,
    agent_prompt TEXT,
    item_key TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (round_id) REFERENCES coderabbit_rounds(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_coderabbit_items_unique
    ON coderabbit_items(round_id, item_key);

CREATE TABLE IF NOT EXISTS coderabbit_comments (
    id TEXT PRIMARY KEY,
    round_id TEXT NOT NULL,
    comment_id INTEGER NOT NULL,
    source TEXT NOT NULL,
    html_url TEXT NOT NULL,
    body TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (round_id) REFERENCES coderabbit_rounds(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_coderabbit_comments_unique
    ON coderabbit_comments(round_id, comment_id, source);
"#,
            )?;
        }

        // Migration 13: Add coderabbit_scope column to repository_settings
        let has_coderabbit_scope: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('repository_settings') WHERE name='coderabbit_scope'",
                [],
                |row| row.get::<_, i64>(0).map(|c| c > 0),
            )
            .unwrap_or(false);

        if !has_coderabbit_scope {
            conn.execute(
                "ALTER TABLE repository_settings ADD COLUMN coderabbit_scope TEXT NOT NULL DEFAULT 'all'",
                [],
            )?;
        }

        // Migration 14: Add total_count to coderabbit_rounds
        let has_total_count: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('coderabbit_rounds') WHERE name='total_count'",
                [],
                |row| row.get::<_, i64>(0).map(|c| c > 0),
            )
            .unwrap_or(false);

        if !has_total_count {
            conn.execute(
                "ALTER TABLE coderabbit_rounds ADD COLUMN total_count INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }

        // Migration 15: Add coderabbit_comments table
        let coderabbit_comments_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='coderabbit_comments'",
                [],
                |row| row.get::<_, i64>(0).map(|c| c > 0),
            )
            .unwrap_or(false);

        if !coderabbit_comments_exists {
            conn.execute_batch(
                r#"
CREATE TABLE IF NOT EXISTS coderabbit_comments (
    id TEXT PRIMARY KEY,
    round_id TEXT NOT NULL,
    comment_id INTEGER NOT NULL,
    source TEXT NOT NULL,
    html_url TEXT NOT NULL,
    body TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (round_id) REFERENCES coderabbit_rounds(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_coderabbit_comments_unique
    ON coderabbit_comments(round_id, comment_id, source);
"#,
            )?;
        }

        // Migration 16: Expand coderabbit_items schema for non-actionable items
        let has_item_key: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('coderabbit_items') WHERE name='item_key'",
                [],
                |row| row.get::<_, i64>(0).map(|c| c > 0),
            )
            .unwrap_or(false);

        if !has_item_key {
            conn.execute_batch(
                r#"
CREATE TABLE coderabbit_items_new (
    id TEXT PRIMARY KEY,
    round_id TEXT NOT NULL,
    comment_id INTEGER NOT NULL,
    source TEXT NOT NULL,
    kind TEXT NOT NULL,
    actionable INTEGER NOT NULL DEFAULT 1,
    category TEXT,
    severity TEXT,
    section TEXT,
    file_path TEXT,
    line INTEGER,
    line_start INTEGER,
    line_end INTEGER,
    original_line INTEGER,
    diff_hunk TEXT,
    html_url TEXT NOT NULL,
    body TEXT NOT NULL,
    agent_prompt TEXT,
    item_key TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (round_id) REFERENCES coderabbit_rounds(id) ON DELETE CASCADE
);

INSERT INTO coderabbit_items_new (
    id, round_id, comment_id, source, kind, actionable, category, severity, section,
    file_path, line, line_start, line_end, original_line, diff_hunk, html_url, body,
    agent_prompt, item_key, created_at, updated_at
)
SELECT
    id,
    round_id,
    comment_id,
    source,
    'actionable' AS kind,
    1 AS actionable,
    category,
    severity,
    NULL AS section,
    file_path,
    line,
    line AS line_start,
    line AS line_end,
    original_line,
    diff_hunk,
    html_url,
    body,
    agent_prompt,
    source || ':' || comment_id AS item_key,
    created_at,
    updated_at
FROM coderabbit_items;

DROP TABLE coderabbit_items;
ALTER TABLE coderabbit_items_new RENAME TO coderabbit_items;

CREATE UNIQUE INDEX IF NOT EXISTS idx_coderabbit_items_unique
    ON coderabbit_items(round_id, item_key);
"#,
            )?;
        }

        // Migration 17: Add CodeRabbit review loop settings columns
        let has_review_loop_enabled: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('repository_settings') WHERE name='coderabbit_review_loop_enabled'",
                [],
                |row| row.get::<_, i64>(0).map(|c| c > 0),
            )
            .unwrap_or(false);

        if !has_review_loop_enabled {
            conn.execute(
                "ALTER TABLE repository_settings ADD COLUMN coderabbit_review_loop_enabled INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }

        let has_review_loop_scope: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('repository_settings') WHERE name='coderabbit_review_loop_scope'",
                [],
                |row| row.get::<_, i64>(0).map(|c| c > 0),
            )
            .unwrap_or(false);

        if !has_review_loop_scope {
            conn.execute(
                "ALTER TABLE repository_settings ADD COLUMN coderabbit_review_loop_scope TEXT NOT NULL DEFAULT 'all'",
                [],
            )?;
        }

        let has_review_loop_done: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('repository_settings') WHERE name='coderabbit_review_loop_done_condition'",
                [],
                |row| row.get::<_, i64>(0).map(|c| c > 0),
            )
            .unwrap_or(false);

        if !has_review_loop_done {
            conn.execute(
                "ALTER TABLE repository_settings ADD COLUMN coderabbit_review_loop_done_condition TEXT NOT NULL DEFAULT 'actionable-zero'",
                [],
            )?;
        }

        let has_review_loop_ask_before: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('repository_settings') WHERE name='coderabbit_review_loop_ask_before_enqueue'",
                [],
                |row| row.get::<_, i64>(0).map(|c| c > 0),
            )
            .unwrap_or(false);

        if !has_review_loop_ask_before {
            conn.execute(
                "ALTER TABLE repository_settings ADD COLUMN coderabbit_review_loop_ask_before_enqueue INTEGER NOT NULL DEFAULT 1",
                [],
            )?;
        }

        // Migration 18: Add notification tracking to coderabbit_rounds
        let has_notified_at: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('coderabbit_rounds') WHERE name='notified_at'",
                [],
                |row| row.get::<_, i64>(0).map(|c| c > 0),
            )
            .unwrap_or(false);

        if !has_notified_at {
            conn.execute(
                "ALTER TABLE coderabbit_rounds ADD COLUMN notified_at TEXT",
                [],
            )?;
        }

        let has_processed_at: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('coderabbit_rounds') WHERE name='processed_at'",
                [],
                |row| row.get::<_, i64>(0).map(|c| c > 0),
            )
            .unwrap_or(false);

        if !has_processed_at {
            conn.execute(
                "ALTER TABLE coderabbit_rounds ADD COLUMN processed_at TEXT",
                [],
            )?;
        }

        Ok(())
    }

    /// Get a reference to the connection (for DAOs)
    pub fn connection(&self) -> Arc<Mutex<Connection>> {
        self.conn.clone()
    }

    /// Execute a closure with the connection
    pub fn with_connection<F, T>(&self, f: F) -> Result<T, DatabaseError>
    where
        F: FnOnce(&Connection) -> SqliteResult<T>,
    {
        let conn = self.conn.lock().map_err(|_| DatabaseError::LockPoisoned)?;
        f(&conn).map_err(DatabaseError::Sqlite)
    }
}

impl std::fmt::Debug for Database {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Database")
            .field("path", &self.path)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_database_creation() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let _db = Database::open(db_path.clone()).unwrap();
        assert!(db_path.exists());
    }

    #[test]
    fn test_schema_initialization() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db = Database::open(db_path).unwrap();

        // Verify tables exist
        db.with_connection(|conn| {
            let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type='table'")?;
            let tables: Vec<String> = stmt
                .query_map([], |row| row.get(0))?
                .filter_map(|r| r.ok())
                .collect();
            assert!(tables.contains(&"repositories".to_string()));
            assert!(tables.contains(&"workspaces".to_string()));
            Ok(())
        })
        .unwrap();
    }
}
