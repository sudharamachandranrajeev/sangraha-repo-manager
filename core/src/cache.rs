//! The last GitHub snapshot and the rename map, in one small SQLite file, so the board opens at
//! once and works offline.

use std::collections::HashMap;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use crate::Result;
use crate::github::Snapshot;

pub struct Cache {
    db: Connection,
}

impl Cache {
    pub fn open_default() -> Result<Cache> {
        let dir = crate::config::data_dir();
        std::fs::create_dir_all(&dir)?;
        Self::open(&dir.join("cache.sqlite"))
    }

    pub fn open(path: &Path) -> Result<Cache> {
        let db = Connection::open(path)?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS kv (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )?;
        Ok(Cache { db })
    }

    fn get<T: serde::de::DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let raw: Option<String> = self
            .db
            .query_row("SELECT value FROM kv WHERE key = ?1", [key], |r| r.get(0))
            .optional()?;
        // A cache written by another version that no longer parses is treated as empty.
        Ok(raw.and_then(|s| serde_json::from_str(&s).ok()))
    }

    fn put<T: serde::Serialize>(&self, key: &str, value: &T) -> Result<()> {
        self.db.execute(
            "INSERT INTO kv (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, serde_json::to_string(value)?],
        )?;
        Ok(())
    }

    pub fn snapshot(&self) -> Result<Option<Snapshot>> {
        self.get("snapshot")
    }

    pub fn set_snapshot(&self, s: &Snapshot) -> Result<()> {
        self.put("snapshot", s)
    }

    pub fn apps(&self) -> Result<Option<crate::apps::AppsSnapshot>> {
        self.get("apps")
    }

    pub fn set_apps(&self, s: &crate::apps::AppsSnapshot) -> Result<()> {
        self.put("apps", s)
    }

    pub fn security(&self) -> Result<Option<crate::security::SecuritySnapshot>> {
        self.get("security")
    }

    pub fn set_security(&self, s: &crate::security::SecuritySnapshot) -> Result<()> {
        self.put("security", s)
    }

    /// Old `owner/name` (lower case) → current `owner/name`; an empty value means "checked, gone".
    pub fn renames(&self) -> Result<HashMap<String, String>> {
        Ok(self.get("renames")?.unwrap_or_default())
    }

    pub fn set_renames(&self, m: &HashMap<String, String>) -> Result<()> {
        self.put("renames", m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let c = Cache::open(&dir.path().join("c.sqlite")).unwrap();
        assert!(c.snapshot().unwrap().is_none());
        let s = Snapshot {
            login: "me".into(),
            fetched_at: 7,
            ..Default::default()
        };
        c.set_snapshot(&s).unwrap();
        c.set_snapshot(&s).unwrap();
        assert_eq!(c.snapshot().unwrap(), Some(s));
        let m = HashMap::from([("me/a".to_owned(), "me/b".to_owned())]);
        c.set_renames(&m).unwrap();
        assert_eq!(c.renames().unwrap(), m);
    }
}
