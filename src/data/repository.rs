//! Repository data access object

use super::models::Repository;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, Result as SqliteResult};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// Data access object for Repository operations
#[derive(Clone)]
pub struct RepositoryStore {
    conn: Arc<Mutex<Connection>>,
}

impl RepositoryStore {
    /// Create a new RepositoryDao
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    /// Insert a new repository
    pub fn create(&self, repo: &Repository) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO repositories (id, name, base_path, repository_url, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                repo.id.to_string(),
                repo.name,
                repo.base_path
                    .as_ref()
                    .map(|p| p.to_string_lossy().to_string()),
                repo.repository_url,
                repo.created_at.to_rfc3339(),
                repo.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    /// Get a repository by ID
    pub fn get_by_id(&self, id: Uuid) -> SqliteResult<Option<Repository>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, base_path, repository_url, created_at, updated_at
             FROM repositories WHERE id = ?1",
        )?;

        let mut rows = stmt.query(params![id.to_string()])?;
        if let Some(row) = rows.next()? {
            Ok(Some(Self::row_to_repository(row)?))
        } else {
            Ok(None)
        }
    }

    /// Get all repositories
    pub fn get_all(&self) -> SqliteResult<Vec<Repository>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, base_path, repository_url, created_at, updated_at
             FROM repositories ORDER BY name",
        )?;

        let repos = stmt
            .query_map([], |row| Self::row_to_repository(row))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(repos)
    }

    /// Update a repository
    pub fn update(&self, repo: &Repository) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE repositories SET name = ?2, base_path = ?3, repository_url = ?4, updated_at = ?5
             WHERE id = ?1",
            params![
                repo.id.to_string(),
                repo.name,
                repo.base_path.as_ref().map(|p| p.to_string_lossy().to_string()),
                repo.repository_url,
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    /// Delete a repository (cascades to workspaces)
    pub fn delete(&self, id: Uuid) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM repositories WHERE id = ?1",
            params![id.to_string()],
        )?;
        Ok(())
    }

    /// Check if a repository exists by base path
    pub fn exists_by_path(&self, path: &PathBuf) -> SqliteResult<bool> {
        let conn = self.conn.lock().unwrap();
        let path_str = path.to_string_lossy().to_string();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM repositories WHERE base_path = ?1",
            params![path_str],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Get a repository by base path
    pub fn get_by_path(&self, path: &PathBuf) -> SqliteResult<Option<Repository>> {
        let conn = self.conn.lock().unwrap();
        let path_str = path.to_string_lossy().to_string();
        let mut stmt = conn.prepare(
            "SELECT id, name, base_path, repository_url, created_at, updated_at
             FROM repositories WHERE base_path = ?1",
        )?;

        let mut rows = stmt.query(params![path_str])?;
        if let Some(row) = rows.next()? {
            Ok(Some(Self::row_to_repository(row)?))
        } else {
            Ok(None)
        }
    }

    /// Convert a database row to a Repository
    fn row_to_repository(row: &rusqlite::Row) -> SqliteResult<Repository> {
        let id_str: String = row.get(0)?;
        let base_path: Option<String> = row.get(2)?;
        let created_at_str: String = row.get(4)?;
        let updated_at_str: String = row.get(5)?;

        Ok(Repository {
            id: Uuid::parse_str(&id_str).unwrap_or_else(|_| Uuid::new_v4()),
            name: row.get(1)?,
            base_path: base_path.map(PathBuf::from),
            repository_url: row.get(3)?,
            created_at: DateTime::parse_from_rfc3339(&created_at_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            updated_at: DateTime::parse_from_rfc3339(&updated_at_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Database;
    use tempfile::tempdir;

    fn setup_db() -> (tempfile::TempDir, Database, RepositoryStore) {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path().join("test.db")).unwrap();
        let dao = RepositoryStore::new(db.connection());
        (dir, db, dao)
    }

    #[test]
    fn test_create_and_get() {
        let (_dir, _db, dao) = setup_db();
        let repo = Repository::from_local_path("test-repo", PathBuf::from("/tmp/test"));

        dao.create(&repo).unwrap();
        let retrieved = dao.get_by_id(repo.id).unwrap().unwrap();

        assert_eq!(retrieved.name, "test-repo");
        assert_eq!(retrieved.base_path, Some(PathBuf::from("/tmp/test")));
    }

    #[test]
    fn test_get_all() {
        let (_dir, _db, dao) = setup_db();

        let repo1 = Repository::from_local_path("repo-a", PathBuf::from("/tmp/a"));
        let repo2 = Repository::from_local_path("repo-b", PathBuf::from("/tmp/b"));

        dao.create(&repo1).unwrap();
        dao.create(&repo2).unwrap();

        let all = dao.get_all().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].name, "repo-a"); // Sorted by name
        assert_eq!(all[1].name, "repo-b");
    }

    #[test]
    fn test_delete() {
        let (_dir, _db, dao) = setup_db();
        let repo = Repository::from_local_path("to-delete", PathBuf::from("/tmp/delete"));

        dao.create(&repo).unwrap();
        assert!(dao.get_by_id(repo.id).unwrap().is_some());

        dao.delete(repo.id).unwrap();
        assert!(dao.get_by_id(repo.id).unwrap().is_none());
    }
}
