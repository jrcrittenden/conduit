//! App state data access object (key-value store)

use chrono::Utc;
use rusqlite::{params, Connection, Result as SqliteResult};
use std::sync::{Arc, Mutex};

/// Data access object for app state (key-value store)
#[derive(Clone)]
pub struct AppStateStore {
    conn: Arc<Mutex<Connection>>,
}

impl AppStateStore {
    /// Create a new AppStateDao
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    /// Set a value (insert or update)
    pub fn set(&self, key: &str, value: &str) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO app_state (key, value, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = ?2, updated_at = ?3",
            params![key, value, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    /// Get a value by key
    pub fn get(&self, key: &str) -> SqliteResult<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT value FROM app_state WHERE key = ?1")?;
        let mut rows = stmt.query(params![key])?;

        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    /// Delete a key
    pub fn delete(&self, key: &str) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM app_state WHERE key = ?1", params![key])?;
        Ok(())
    }

    /// Clear all state
    pub fn clear_all(&self) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM app_state", [])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Database;
    use tempfile::tempdir;

    fn setup_db() -> (tempfile::TempDir, Database, AppStateStore) {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path().join("test.db")).unwrap();
        let dao = AppStateStore::new(db.connection());
        (dir, db, dao)
    }

    #[test]
    fn test_set_and_get() {
        let (_dir, _db, dao) = setup_db();

        dao.set("active_tab_index", "2").unwrap();
        let value = dao.get("active_tab_index").unwrap();
        assert_eq!(value, Some("2".to_string()));
    }

    #[test]
    fn test_update() {
        let (_dir, _db, dao) = setup_db();

        dao.set("sidebar_visible", "true").unwrap();
        dao.set("sidebar_visible", "false").unwrap();

        let value = dao.get("sidebar_visible").unwrap();
        assert_eq!(value, Some("false".to_string()));
    }

    #[test]
    fn test_get_nonexistent() {
        let (_dir, _db, dao) = setup_db();

        let value = dao.get("nonexistent").unwrap();
        assert_eq!(value, None);
    }

    #[test]
    fn test_delete() {
        let (_dir, _db, dao) = setup_db();

        dao.set("to_delete", "value").unwrap();
        dao.delete("to_delete").unwrap();

        let value = dao.get("to_delete").unwrap();
        assert_eq!(value, None);
    }

    #[test]
    fn test_clear_all() {
        let (_dir, _db, dao) = setup_db();

        dao.set("key1", "value1").unwrap();
        dao.set("key2", "value2").unwrap();
        dao.clear_all().unwrap();

        assert_eq!(dao.get("key1").unwrap(), None);
        assert_eq!(dao.get("key2").unwrap(), None);
    }
}
