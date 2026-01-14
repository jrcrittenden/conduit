//! CodeRabbit persistence helpers.

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, Result as SqliteResult};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use super::models::{
    CodeRabbitCategory, CodeRabbitItem, CodeRabbitItemSource, CodeRabbitMode, CodeRabbitRetention,
    CodeRabbitRound, CodeRabbitRoundStatus, CodeRabbitSeverity, RepositorySettings,
};

const DEFAULT_BACKOFF_SECONDS: &[i64] = &[30, 120, 300];

fn parse_backoff_seconds(raw: &str) -> Vec<i64> {
    let mut values = Vec::new();
    for part in raw.split(',') {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(value) = trimmed.parse::<i64>() {
            if value > 0 {
                values.push(value);
            }
        }
    }
    if values.is_empty() {
        DEFAULT_BACKOFF_SECONDS.to_vec()
    } else {
        values
    }
}

fn backoff_to_string(values: &[i64]) -> String {
    if values.is_empty() {
        return DEFAULT_BACKOFF_SECONDS
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(",");
    }
    values
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

#[derive(Clone)]
pub struct RepositorySettingsStore {
    conn: Arc<Mutex<Connection>>,
}

impl RepositorySettingsStore {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    pub fn get_by_repository(
        &self,
        repository_id: Uuid,
    ) -> SqliteResult<Option<RepositorySettings>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT repository_id, coderabbit_mode, coderabbit_retention, coderabbit_backoff_seconds, updated_at
             FROM repository_settings WHERE repository_id = ?1",
        )?;
        let mut rows = stmt.query(params![repository_id.to_string()])?;
        if let Some(row) = rows.next()? {
            Ok(Some(Self::row_to_settings(row)?))
        } else {
            Ok(None)
        }
    }

    pub fn get_or_default(&self, repository_id: Uuid) -> SqliteResult<RepositorySettings> {
        if let Some(settings) = self.get_by_repository(repository_id)? {
            Ok(settings)
        } else {
            Ok(Self::default_settings(repository_id))
        }
    }

    pub fn upsert(&self, settings: &RepositorySettings) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap();
        let updated_at = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO repository_settings (repository_id, coderabbit_mode, coderabbit_retention, coderabbit_backoff_seconds, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(repository_id) DO UPDATE SET
                 coderabbit_mode = excluded.coderabbit_mode,
                 coderabbit_retention = excluded.coderabbit_retention,
                 coderabbit_backoff_seconds = excluded.coderabbit_backoff_seconds,
                 updated_at = excluded.updated_at",
            params![
                settings.repository_id.to_string(),
                settings.coderabbit_mode.as_str(),
                settings.coderabbit_retention.as_str(),
                backoff_to_string(&settings.coderabbit_backoff_seconds),
                updated_at,
            ],
        )?;
        Ok(())
    }

    fn default_settings(repository_id: Uuid) -> RepositorySettings {
        RepositorySettings {
            repository_id,
            coderabbit_mode: CodeRabbitMode::Auto,
            coderabbit_retention: CodeRabbitRetention::Keep,
            coderabbit_backoff_seconds: DEFAULT_BACKOFF_SECONDS.to_vec(),
            updated_at: Utc::now(),
        }
    }

    fn row_to_settings(row: &rusqlite::Row) -> SqliteResult<RepositorySettings> {
        let repo_id: String = row.get(0)?;
        let mode: String = row.get(1)?;
        let retention: String = row.get(2)?;
        let backoff_raw: String = row.get(3)?;
        let updated_at_str: String = row.get(4)?;

        Ok(RepositorySettings {
            repository_id: Uuid::parse_str(&repo_id).unwrap_or_else(|_| Uuid::new_v4()),
            coderabbit_mode: CodeRabbitMode::parse(&mode),
            coderabbit_retention: CodeRabbitRetention::parse(&retention),
            coderabbit_backoff_seconds: parse_backoff_seconds(&backoff_raw),
            updated_at: DateTime::parse_from_rfc3339(&updated_at_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        })
    }
}

#[derive(Clone)]
pub struct CodeRabbitRoundStore {
    conn: Arc<Mutex<Connection>>,
}

impl CodeRabbitRoundStore {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    pub fn create(&self, round: &CodeRabbitRound) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO coderabbit_rounds (
                id, repository_id, workspace_id, pr_number, head_sha, check_state, check_started_at,
                observed_at, status, attempt_count, next_fetch_at, actionable_count, completed_at,
                created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                round.id.to_string(),
                round.repository_id.to_string(),
                round.workspace_id.map(|id| id.to_string()),
                round.pr_number,
                round.head_sha,
                round.check_state,
                round.check_started_at.to_rfc3339(),
                round.observed_at.to_rfc3339(),
                round.status.as_str(),
                round.attempt_count,
                round.next_fetch_at.map(|dt| dt.to_rfc3339()),
                round.actionable_count,
                round.completed_at.map(|dt| dt.to_rfc3339()),
                round.created_at.to_rfc3339(),
                round.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn get_by_key(
        &self,
        repository_id: Uuid,
        pr_number: i64,
        head_sha: &str,
        check_started_at: &str,
    ) -> SqliteResult<Option<CodeRabbitRound>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, repository_id, workspace_id, pr_number, head_sha, check_state, check_started_at,
                    observed_at, status, attempt_count, next_fetch_at, actionable_count, completed_at,
                    created_at, updated_at
             FROM coderabbit_rounds
             WHERE repository_id = ?1 AND pr_number = ?2 AND head_sha = ?3 AND check_started_at = ?4",
        )?;
        let mut rows = stmt.query(params![
            repository_id.to_string(),
            pr_number,
            head_sha,
            check_started_at
        ])?;
        if let Some(row) = rows.next()? {
            Ok(Some(Self::row_to_round(row)?))
        } else {
            Ok(None)
        }
    }

    pub fn get_latest_for_head(
        &self,
        repository_id: Uuid,
        pr_number: i64,
        head_sha: &str,
    ) -> SqliteResult<Option<CodeRabbitRound>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, repository_id, workspace_id, pr_number, head_sha, check_state, check_started_at,
                    observed_at, status, attempt_count, next_fetch_at, actionable_count, completed_at,
                    created_at, updated_at
             FROM coderabbit_rounds
             WHERE repository_id = ?1 AND pr_number = ?2 AND head_sha = ?3
             ORDER BY check_started_at DESC
             LIMIT 1",
        )?;
        let mut rows = stmt.query(params![repository_id.to_string(), pr_number, head_sha])?;
        if let Some(row) = rows.next()? {
            Ok(Some(Self::row_to_round(row)?))
        } else {
            Ok(None)
        }
    }

    pub fn list_pending_due(&self, now: DateTime<Utc>) -> SqliteResult<Vec<CodeRabbitRound>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, repository_id, workspace_id, pr_number, head_sha, check_state, check_started_at,
                    observed_at, status, attempt_count, next_fetch_at, actionable_count, completed_at,
                    created_at, updated_at
             FROM coderabbit_rounds
             WHERE status = 'pending'
               AND next_fetch_at IS NOT NULL
               AND next_fetch_at <= ?1
             ORDER BY next_fetch_at ASC",
        )?;
        let rounds = stmt
            .query_map(params![now.to_rfc3339()], Self::row_to_round)?
            .filter_map(|row| row.ok())
            .collect();
        Ok(rounds)
    }

    pub fn update(&self, round: &CodeRabbitRound) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE coderabbit_rounds SET
                workspace_id = ?2,
                status = ?3,
                attempt_count = ?4,
                next_fetch_at = ?5,
                actionable_count = ?6,
                completed_at = ?7,
                updated_at = ?8
             WHERE id = ?1",
            params![
                round.id.to_string(),
                round.workspace_id.map(|id| id.to_string()),
                round.status.as_str(),
                round.attempt_count,
                round.next_fetch_at.map(|dt| dt.to_rfc3339()),
                round.actionable_count,
                round.completed_at.map(|dt| dt.to_rfc3339()),
                round.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn delete_by_pr(&self, repository_id: Uuid, pr_number: i64) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM coderabbit_rounds WHERE repository_id = ?1 AND pr_number = ?2",
            params![repository_id.to_string(), pr_number],
        )?;
        Ok(())
    }

    fn row_to_round(row: &rusqlite::Row) -> SqliteResult<CodeRabbitRound> {
        let id_str: String = row.get(0)?;
        let repo_id_str: String = row.get(1)?;
        let workspace_id_str: Option<String> = row.get(2)?;
        let check_started_at_str: String = row.get(6)?;
        let observed_at_str: String = row.get(7)?;
        let status_str: String = row.get(8)?;
        let next_fetch_at_str: Option<String> = row.get(10)?;
        let completed_at_str: Option<String> = row.get(12)?;
        let created_at_str: String = row.get(13)?;
        let updated_at_str: String = row.get(14)?;

        Ok(CodeRabbitRound {
            id: Uuid::parse_str(&id_str).unwrap_or_else(|_| Uuid::new_v4()),
            repository_id: Uuid::parse_str(&repo_id_str).unwrap_or_else(|_| Uuid::new_v4()),
            workspace_id: workspace_id_str.and_then(|value| Uuid::parse_str(&value).ok()),
            pr_number: row.get(3)?,
            head_sha: row.get(4)?,
            check_state: row.get(5)?,
            check_started_at: DateTime::parse_from_rfc3339(&check_started_at_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            observed_at: DateTime::parse_from_rfc3339(&observed_at_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            status: CodeRabbitRoundStatus::parse(&status_str),
            attempt_count: row.get(9)?,
            next_fetch_at: next_fetch_at_str
                .and_then(|value| DateTime::parse_from_rfc3339(&value).ok())
                .map(|dt| dt.with_timezone(&Utc)),
            actionable_count: row.get(11)?,
            completed_at: completed_at_str
                .and_then(|value| DateTime::parse_from_rfc3339(&value).ok())
                .map(|dt| dt.with_timezone(&Utc)),
            created_at: DateTime::parse_from_rfc3339(&created_at_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            updated_at: DateTime::parse_from_rfc3339(&updated_at_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        })
    }
}

#[derive(Clone)]
pub struct CodeRabbitItemStore {
    conn: Arc<Mutex<Connection>>,
}

impl CodeRabbitItemStore {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    pub fn insert_items(&self, items: &[CodeRabbitItem]) -> SqliteResult<usize> {
        let conn = self.conn.lock().unwrap();
        let mut inserted = 0usize;
        for item in items {
            let rows = conn.execute(
                "INSERT OR IGNORE INTO coderabbit_items (
                    id, round_id, comment_id, source, category, severity, file_path, line,
                    original_line, diff_hunk, html_url, body, agent_prompt, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
                    item.id.to_string(),
                    item.round_id.to_string(),
                    item.comment_id,
                    item.source.as_str(),
                    item.category.as_str(),
                    item.severity.map(|severity| severity.as_str()),
                    item.file_path,
                    item.line,
                    item.original_line,
                    item.diff_hunk,
                    item.html_url,
                    item.body,
                    item.agent_prompt,
                    item.created_at.to_rfc3339(),
                    item.updated_at.to_rfc3339(),
                ],
            )?;
            if rows > 0 {
                inserted += 1;
            }
        }
        Ok(inserted)
    }

    pub fn count_for_round(&self, round_id: Uuid) -> SqliteResult<i64> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM coderabbit_items WHERE round_id = ?1",
            params![round_id.to_string()],
            |row| row.get(0),
        )?;
        Ok(count)
    }
}

#[allow(dead_code)]
fn _assert_enum_strs() {
    let _ = CodeRabbitCategory::PotentialIssue.as_str();
    let _ = CodeRabbitItemSource::ReviewComment.as_str();
    let _ = CodeRabbitRoundStatus::Pending.as_str();
    let _ = CodeRabbitSeverity::Critical.as_str();
}
