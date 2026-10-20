// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One way to copy a SQLite database file: `VACUUM INTO` a temporary file
//! beside the destination, then rename it into place.
//!
//! Used by every copy srv makes of a store: the copy-forward of a previous
//! version's databases (`bootstrap/carry_forward.rs`), the pre-migration
//! snapshot (`snapshot.rs`) and the pre-migration backup
//! (`migrations/runner.rs`). A plain file copy of a database in WAL mode can
//! miss committed writes that are still only in its `-wal` file, and other
//! processes may hold the shared stores open while a copy is taken;
//! `VACUUM INTO` reads through SQLite, so the copy holds every committed
//! write. The source is opened read-only, so copying never writes to it.

use std::path::Path;

/// A consistent copy of the SQLite database `src` at `dst`. `dst` is replaced
/// if it exists; a failed copy leaves no partial file behind.
pub fn copy_database(src: &Path, dst: &Path) -> Result<(), String> {
    let tmp = dst.with_extension("db.copying");
    let _ = std::fs::remove_file(&tmp);
    let result = (|| {
        let conn = rusqlite::Connection::open_with_flags(
            src,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| format!("open {}: {e}", src.display()))?;
        conn.busy_timeout(std::time::Duration::from_secs(5)).map_err(|e| e.to_string())?;
        let tmp_str = tmp.to_str().ok_or("the data path isn't UTF-8")?;
        conn.execute("VACUUM INTO ?1", [tmp_str])
            .map_err(|e| format!("copy {}: {e}", src.display()))?;
        drop(conn);
        std::fs::rename(&tmp, dst).map_err(|e| format!("rename into {}: {e}", dst.display()))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn wal_db_with_uncheckpointed_row(path: &Path) -> Connection {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA wal_autocheckpoint=0;
             CREATE TABLE t (v TEXT);
             INSERT INTO t VALUES ('committed');",
        )
        .unwrap();
        conn
    }

    #[test]
    fn the_copy_includes_writes_still_in_the_wal() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.db");
        // Kept open, so the row stays in src.db-wal, not src.db.
        let _writer = wal_db_with_uncheckpointed_row(&src);

        let plain = dir.path().join("plain.db");
        std::fs::copy(&src, &plain).unwrap();
        let plain_rows: Result<i64, _> =
            Connection::open(&plain).unwrap().query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0));
        assert!(plain_rows.is_err() || plain_rows.unwrap() == 0, "a plain copy misses the WAL");

        let dst = dir.path().join("dst.db");
        copy_database(&src, &dst).unwrap();
        let v: String = Connection::open(&dst).unwrap().query_row("SELECT v FROM t", [], |r| r.get(0)).unwrap();
        assert_eq!(v, "committed");
        assert!(!dst.with_extension("db.copying").exists(), "no temporary file is left behind");
    }

    #[test]
    fn a_failed_copy_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let dst = dir.path().join("dst.db");
        assert!(copy_database(&dir.path().join("missing.db"), &dst).is_err());
        assert!(!dst.exists());
        assert!(!dst.with_extension("db.copying").exists());
    }
}
