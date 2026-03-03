// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! WaveStore: generic OID-based CRUD for WaveObj types.
//! Port of Go's pkg/wstore/wstore_dbops.go + wstore_dbsetup.go.
//!
//! Uses `Mutex<Connection>` matching Go's `MaxOpenConns(1)`.
//! SQLite WAL mode + 5s busy timeout (same as Go).

#![allow(dead_code)]

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection};

use crate::backend::waveobj::{wave_obj_from_json, wave_obj_to_json, WaveObj};

use super::error::StoreError;
use super::migrations::run_wstore_migrations;

/// SQLite-backed object store for WaveObj types.
pub struct WaveStore {
    conn: Mutex<Connection>,
}

impl WaveStore {
    /// Open a WaveStore backed by a file on disk.
    /// Configures WAL mode and 5s busy timeout (matching Go).
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        Self::configure_and_migrate(conn)
    }

    /// Open an in-memory WaveStore for testing.
    pub fn open_in_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory()?;
        Self::configure_and_migrate(conn)
    }

    fn configure_and_migrate(conn: Connection) -> Result<Self, StoreError> {
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA busy_timeout=5000;
             PRAGMA synchronous=NORMAL;
             PRAGMA cache_size=-8000;
             PRAGMA mmap_size=268435456;
             PRAGMA temp_store=MEMORY;",
        )?;
        run_wstore_migrations(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Table name for a WaveObj type: `db_<otype>`.
    fn table_name<T: WaveObj>() -> String {
        format!("db_{}", T::get_otype())
    }

    /// Get a single object by OID. Returns `None` if not found.
    pub fn get<T: WaveObj>(&self, oid: &str) -> Result<Option<T>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let table = Self::table_name::<T>();
        let mut stmt =
            conn.prepare(&format!("SELECT version, data FROM {table} WHERE oid = ?1"))?;

        let result = stmt.query_row(params![oid], |row| {
            let version: i64 = row.get(0)?;
            let data: Vec<u8> = row.get(1)?;
            Ok((version, data))
        });

        match result {
            Ok((version, data)) => {
                let mut obj: T = wave_obj_from_json(&data)?;
                obj.set_version(version);
                Ok(Some(obj))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StoreError::Sqlite(e)),
        }
    }

    /// Get a single object, returning `StoreError::NotFound` if missing.
    pub fn must_get<T: WaveObj>(&self, oid: &str) -> Result<T, StoreError> {
        self.get::<T>(oid)?.ok_or(StoreError::NotFound)
    }

    /// Get a single object as raw JSON Value by otype and OID.
    /// Used by GetObject/GetObjects to return data without strict struct deserialization.
    pub fn get_raw(&self, otype: &str, oid: &str) -> Result<Option<serde_json::Value>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let table = format!("db_{}", otype);
        let mut stmt =
            conn.prepare(&format!("SELECT version, data FROM {table} WHERE oid = ?1"))?;

        let result = stmt.query_row(params![oid], |row| {
            let version: i64 = row.get(0)?;
            let data: Vec<u8> = row.get(1)?;
            Ok((version, data))
        });

        match result {
            Ok((version, data)) => {
                let mut val: serde_json::Value = serde_json::from_slice(&data)
                    .map_err(|e| StoreError::Json(e))?;
                if let Some(obj) = val.as_object_mut() {
                    obj.insert("version".to_string(), serde_json::json!(version));
                    obj.insert("otype".to_string(), serde_json::json!(otype));
                }
                Ok(Some(val))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StoreError::Sqlite(e)),
        }
    }

    /// Check if an object exists (by otype and OID).
    pub fn exists_raw(&self, otype: &str, oid: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let table = format!("db_{}", otype);
        let count: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM {table} WHERE oid = ?1"),
            params![oid],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Insert a new object. Sets version to 1.
    pub fn insert<T: WaveObj>(&self, obj: &mut T) -> Result<(), StoreError> {
        let oid = obj.get_oid().to_string();
        if oid.is_empty() {
            return Err(StoreError::EmptyOID);
        }

        obj.set_version(1);
        let data = wave_obj_to_json(obj)?;

        let conn = self.conn.lock().unwrap();
        let table = Self::table_name::<T>();
        conn.execute(
            &format!("INSERT INTO {table} (oid, version, data) VALUES (?1, 1, ?2)"),
            params![oid, data],
        )?;

        Ok(())
    }

    /// Update an existing object. Increments version atomically.
    /// Returns the new version number.
    pub fn update<T: WaveObj>(&self, obj: &mut T) -> Result<i64, StoreError> {
        let oid = obj.get_oid().to_string();
        if oid.is_empty() {
            return Err(StoreError::EmptyOID);
        }

        let data = wave_obj_to_json(obj)?;

        let conn = self.conn.lock().unwrap();
        let table = Self::table_name::<T>();

        // Optimistic locking: increment version and return new value.
        // Matches Go: `UPDATE ... SET version = version+1 ... RETURNING version`
        let new_version: i64 = conn.query_row(
            &format!(
                "UPDATE {table} SET data = ?1, version = version + 1 WHERE oid = ?2 RETURNING version"
            ),
            params![data, oid],
            |row| row.get(0),
        )?;

        obj.set_version(new_version);
        Ok(new_version)
    }

    /// Update an object using raw JSON (bypasses struct deserialization).
    /// Used by UpdateObject where the frontend sends the full replacement object.
    /// This matches Go's generic map-based UpdateObject behavior.
    pub fn update_raw(&self, otype: &str, oid: &str, value: &serde_json::Value) -> Result<i64, StoreError> {
        if oid.is_empty() {
            return Err(StoreError::EmptyOID);
        }
        let data = serde_json::to_vec(value)?;
        let conn = self.conn.lock().unwrap();
        let table = format!("db_{}", otype);
        let new_version: i64 = conn.query_row(
            &format!(
                "UPDATE {table} SET data = ?1, version = version + 1 WHERE oid = ?2 RETURNING version"
            ),
            params![data, oid],
            |row| row.get(0),
        )?;
        Ok(new_version)
    }

    /// Delete an object by OID.
    pub fn delete<T: WaveObj>(&self, oid: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        let table = Self::table_name::<T>();
        conn.execute(
            &format!("DELETE FROM {table} WHERE oid = ?1"),
            params![oid],
        )?;
        Ok(())
    }

    /// Delete by otype string and OID (for dynamic dispatch).
    /// Validates `otype` against `VALID_OTYPES` to prevent SQL injection.
    pub fn delete_by_otype(&self, otype: &str, oid: &str) -> Result<(), StoreError> {
        if !crate::backend::waveobj::VALID_OTYPES.contains(&otype) {
            return Err(StoreError::Other(format!("unknown otype: {otype:?}")));
        }
        let conn = self.conn.lock().unwrap();
        let table = format!("db_{otype}");
        conn.execute(
            &format!("DELETE FROM {table} WHERE oid = ?1"),
            params![oid],
        )?;
        Ok(())
    }

    /// Get all objects of a given type.
    pub fn get_all<T: WaveObj>(&self) -> Result<Vec<T>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let table = Self::table_name::<T>();
        let mut stmt = conn.prepare(&format!("SELECT oid, version, data FROM {table}"))?;
        let rows = stmt.query_map([], |row| {
            let version: i64 = row.get(1)?;
            let data: Vec<u8> = row.get(2)?;
            Ok((version, data))
        })?;

        let mut result = Vec::new();
        for row in rows {
            let (version, data) = row?;
            let mut obj: T = wave_obj_from_json(&data)?;
            obj.set_version(version);
            result.push(obj);
        }
        Ok(result)
    }

    /// Count objects of a given type.
    pub fn count<T: WaveObj>(&self) -> Result<i64, StoreError> {
        let conn = self.conn.lock().unwrap();
        let table = Self::table_name::<T>();
        let count: i64 =
            conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })?;
        Ok(count)
    }

    /// Execute multiple operations in a single SQLite transaction.
    /// Acquires the Mutex once, wraps all operations in BEGIN/COMMIT.
    /// On error, rolls back and returns the error.
    ///
    /// This is the key performance primitive — reduces N lock acquisitions
    /// and N fsyncs to 1 each.
    pub fn with_tx<F, R>(&self, f: F) -> Result<R, StoreError>
    where
        F: FnOnce(&StoreTx) -> Result<R, StoreError>,
    {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch("BEGIN")?;
        let tx = StoreTx { conn: &conn };
        match f(&tx) {
            Ok(result) => {
                conn.execute_batch("COMMIT")?;
                Ok(result)
            }
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }
}

/// A borrowed connection handle for use inside [`WaveStore::with_tx`].
/// Provides the same CRUD methods as `WaveStore` but operates on the
/// already-locked connection without additional Mutex acquisition.
pub struct StoreTx<'a> {
    conn: &'a Connection,
}

impl<'a> StoreTx<'a> {
    fn table_name<T: WaveObj>() -> String {
        format!("db_{}", T::get_otype())
    }

    pub fn get<T: WaveObj>(&self, oid: &str) -> Result<Option<T>, StoreError> {
        let table = Self::table_name::<T>();
        let mut stmt =
            self.conn.prepare(&format!("SELECT version, data FROM {table} WHERE oid = ?1"))?;

        let result = stmt.query_row(params![oid], |row| {
            let version: i64 = row.get(0)?;
            let data: Vec<u8> = row.get(1)?;
            Ok((version, data))
        });

        match result {
            Ok((version, data)) => {
                let mut obj: T = wave_obj_from_json(&data)?;
                obj.set_version(version);
                Ok(Some(obj))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StoreError::Sqlite(e)),
        }
    }

    pub fn must_get<T: WaveObj>(&self, oid: &str) -> Result<T, StoreError> {
        self.get::<T>(oid)?.ok_or(StoreError::NotFound)
    }

    pub fn insert<T: WaveObj>(&self, obj: &mut T) -> Result<(), StoreError> {
        let oid = obj.get_oid().to_string();
        if oid.is_empty() {
            return Err(StoreError::EmptyOID);
        }

        obj.set_version(1);
        let data = wave_obj_to_json(obj)?;

        let table = Self::table_name::<T>();
        self.conn.execute(
            &format!("INSERT INTO {table} (oid, version, data) VALUES (?1, 1, ?2)"),
            params![oid, data],
        )?;

        Ok(())
    }

    pub fn update<T: WaveObj>(&self, obj: &mut T) -> Result<i64, StoreError> {
        let oid = obj.get_oid().to_string();
        if oid.is_empty() {
            return Err(StoreError::EmptyOID);
        }

        let data = wave_obj_to_json(obj)?;

        let table = Self::table_name::<T>();
        let new_version: i64 = self.conn.query_row(
            &format!(
                "UPDATE {table} SET data = ?1, version = version + 1 WHERE oid = ?2 RETURNING version"
            ),
            params![data, oid],
            |row| row.get(0),
        )?;

        obj.set_version(new_version);
        Ok(new_version)
    }

    pub fn get_all<T: WaveObj>(&self) -> Result<Vec<T>, StoreError> {
        let table = Self::table_name::<T>();
        let mut stmt = self.conn.prepare(&format!("SELECT oid, version, data FROM {table}"))?;
        let rows = stmt.query_map([], |row| {
            let version: i64 = row.get(1)?;
            let data: Vec<u8> = row.get(2)?;
            Ok((version, data))
        })?;

        let mut result = Vec::new();
        for row in rows {
            let (version, data) = row?;
            let mut obj: T = wave_obj_from_json(&data)?;
            obj.set_version(version);
            result.push(obj);
        }
        Ok(result)
    }

    pub fn delete<T: WaveObj>(&self, oid: &str) -> Result<(), StoreError> {
        let table = Self::table_name::<T>();
        self.conn.execute(
            &format!("DELETE FROM {table} WHERE oid = ?1"),
            params![oid],
        )?;
        Ok(())
    }
}

// ====================================================================
// Tests
// ====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::waveobj::*;

    fn make_store() -> WaveStore {
        WaveStore::open_in_memory().unwrap()
    }

    #[test]
    fn test_insert_and_get_client() {
        let store = make_store();
        let mut client = Client {
            oid: "test-client-oid".to_string(),
            version: 0,
            windowids: vec!["w1".to_string()],
            meta: MetaMapType::new(),
            tosagreed: 1700000000000,
            ..Default::default()
        };
        store.insert(&mut client).unwrap();
        assert_eq!(client.get_version(), 1);

        let loaded = store.must_get::<Client>("test-client-oid").unwrap();
        assert_eq!(loaded.oid, "test-client-oid");
        assert_eq!(loaded.version, 1);
        assert_eq!(loaded.windowids, vec!["w1"]);
        assert_eq!(loaded.tosagreed, 1700000000000);
    }

    #[test]
    fn test_insert_and_get_window() {
        let store = make_store();
        let mut win = Window {
            oid: "win-1".to_string(),
            workspaceid: "ws-1".to_string(),
            pos: Point { x: 10, y: 20 },
            winsize: WinSize {
                width: 800,
                height: 600,
            },
            meta: MetaMapType::new(),
            ..Default::default()
        };
        store.insert(&mut win).unwrap();

        let loaded = store.must_get::<Window>("win-1").unwrap();
        assert_eq!(loaded.workspaceid, "ws-1");
        assert_eq!(loaded.pos.x, 10);
        assert_eq!(loaded.winsize.width, 800);
    }

    #[test]
    fn test_insert_and_get_workspace() {
        let store = make_store();
        let mut ws = Workspace {
            oid: "ws-1".to_string(),
            name: "Test WS".to_string(),
            tabids: vec!["t1".to_string()],
            activetabid: "t1".to_string(),
            meta: MetaMapType::new(),
            ..Default::default()
        };
        store.insert(&mut ws).unwrap();

        let loaded = store.must_get::<Workspace>("ws-1").unwrap();
        assert_eq!(loaded.name, "Test WS");
        assert_eq!(loaded.tabids, vec!["t1"]);
    }

    #[test]
    fn test_insert_and_get_tab() {
        let store = make_store();
        let mut tab = Tab {
            oid: "tab-1".to_string(),
            name: "Shell".to_string(),
            layoutstate: "ls-1".to_string(),
            blockids: vec!["b1".to_string()],
            meta: MetaMapType::new(),
            ..Default::default()
        };
        store.insert(&mut tab).unwrap();

        let loaded = store.must_get::<Tab>("tab-1").unwrap();
        assert_eq!(loaded.name, "Shell");
    }

    #[test]
    fn test_insert_and_get_block() {
        let store = make_store();
        let mut block = Block {
            oid: "blk-1".to_string(),
            parentoref: "tab:tab-1".to_string(),
            meta: {
                let mut m = MetaMapType::new();
                m.insert("view".into(), serde_json::json!("term"));
                m
            },
            ..Default::default()
        };
        store.insert(&mut block).unwrap();

        let loaded = store.must_get::<Block>("blk-1").unwrap();
        assert_eq!(loaded.parentoref, "tab:tab-1");
        assert_eq!(loaded.meta.get("view").unwrap(), "term");
    }

    #[test]
    fn test_insert_and_get_layout_state() {
        let store = make_store();
        let mut ls = LayoutState {
            oid: "ls-1".to_string(),
            rootnode: Some(serde_json::json!({"type": "split"})),
            magnifiednodeid: "n1".to_string(),
            ..Default::default()
        };
        store.insert(&mut ls).unwrap();

        let loaded = store.must_get::<LayoutState>("ls-1").unwrap();
        assert_eq!(loaded.magnifiednodeid, "n1");
        assert!(loaded.rootnode.is_some());
    }

    #[test]
    fn test_get_nonexistent_returns_none() {
        let store = make_store();
        let result = store.get::<Client>("nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_must_get_nonexistent_returns_error() {
        let store = make_store();
        let result = store.must_get::<Client>("nonexistent");
        assert!(matches!(result, Err(StoreError::NotFound)));
    }

    #[test]
    fn test_update_increments_version() {
        let store = make_store();
        let mut client = Client {
            oid: "c1".to_string(),
            meta: MetaMapType::new(),
            ..Default::default()
        };
        store.insert(&mut client).unwrap();
        assert_eq!(client.version, 1);

        client.windowids = vec!["w-new".to_string()];
        let v2 = store.update(&mut client).unwrap();
        assert_eq!(v2, 2);
        assert_eq!(client.version, 2);

        let v3 = store.update(&mut client).unwrap();
        assert_eq!(v3, 3);
    }

    #[test]
    fn test_delete() {
        let store = make_store();
        let mut client = Client {
            oid: "del-me".to_string(),
            meta: MetaMapType::new(),
            ..Default::default()
        };
        store.insert(&mut client).unwrap();
        assert!(store.get::<Client>("del-me").unwrap().is_some());

        store.delete::<Client>("del-me").unwrap();
        assert!(store.get::<Client>("del-me").unwrap().is_none());
    }

    #[test]
    fn test_get_all() {
        let store = make_store();
        for i in 0..3 {
            let mut tab = Tab {
                oid: format!("tab-{i}"),
                name: format!("Tab {i}"),
                meta: MetaMapType::new(),
                ..Default::default()
            };
            store.insert(&mut tab).unwrap();
        }

        let all = store.get_all::<Tab>().unwrap();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn test_count() {
        let store = make_store();
        assert_eq!(store.count::<Client>().unwrap(), 0);

        let mut c = Client {
            oid: "c1".to_string(),
            meta: MetaMapType::new(),
            ..Default::default()
        };
        store.insert(&mut c).unwrap();
        assert_eq!(store.count::<Client>().unwrap(), 1);
    }

    #[test]
    fn test_insert_empty_oid_fails() {
        let store = make_store();
        let mut client = Client {
            oid: String::new(),
            meta: MetaMapType::new(),
            ..Default::default()
        };
        let result = store.insert(&mut client);
        assert!(matches!(result, Err(StoreError::EmptyOID)));
    }

    #[test]
    fn test_with_tx_commits_on_success() {
        let store = make_store();
        store
            .with_tx(|tx| {
                let mut ws = Workspace {
                    oid: "ws-tx".to_string(),
                    name: "TX Workspace".to_string(),
                    meta: MetaMapType::new(),
                    ..Default::default()
                };
                tx.insert(&mut ws)?;

                let mut tab = Tab {
                    oid: "tab-tx".to_string(),
                    name: "T1".to_string(),
                    layoutstate: "ls-tx".to_string(),
                    meta: MetaMapType::new(),
                    ..Default::default()
                };
                tx.insert(&mut tab)?;

                // Update workspace to reference tab
                ws.tabids.push("tab-tx".to_string());
                tx.update(&mut ws)?;

                Ok(())
            })
            .unwrap();

        // Verify everything committed
        let ws = store.must_get::<Workspace>("ws-tx").unwrap();
        assert_eq!(ws.name, "TX Workspace");
        assert_eq!(ws.tabids, vec!["tab-tx"]);
        assert_eq!(ws.version, 2); // insert=v1, update=v2

        let tab = store.must_get::<Tab>("tab-tx").unwrap();
        assert_eq!(tab.name, "T1");
    }

    #[test]
    fn test_with_tx_rollbacks_on_error() {
        let store = make_store();
        let result: Result<(), StoreError> = store.with_tx(|tx| {
            let mut ws = Workspace {
                oid: "ws-rollback".to_string(),
                name: "Should Not Exist".to_string(),
                meta: MetaMapType::new(),
                ..Default::default()
            };
            tx.insert(&mut ws)?;

            // Force an error
            Err(StoreError::Other("intentional failure".to_string()))
        });
        assert!(result.is_err());

        // Verify the insert was rolled back
        let ws = store.get::<Workspace>("ws-rollback").unwrap();
        assert!(ws.is_none());
    }
}
