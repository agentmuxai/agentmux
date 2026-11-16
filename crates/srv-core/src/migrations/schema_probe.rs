// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The one "does this table exist" probe. Every migration used to carry its own copy of the
//! `sqlite_master` query, with its own idea of what to do with an error.

use rusqlite::Connection;

/// Whether `table` exists in `conn`'s main schema. A failed probe is an error, never "absent": a
/// migration that treats a database it could not read as empty would skip work it owes.
pub(crate) fn table_exists(conn: &Connection, table: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [table],
        |row| row.get(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_table_is_found_once_it_exists_and_a_view_or_index_is_not_a_table() {
        let conn = Connection::open_in_memory().unwrap();
        assert!(!table_exists(&conn, "t").unwrap());
        conn.execute_batch("CREATE TABLE t (id INTEGER); CREATE INDEX i ON t(id); CREATE VIEW v AS SELECT id FROM t;").unwrap();
        assert!(table_exists(&conn, "t").unwrap());
        assert!(!table_exists(&conn, "i").unwrap());
        assert!(!table_exists(&conn, "v").unwrap());
    }

    #[test]
    fn a_connection_that_cannot_be_read_is_an_error_not_an_absent_table() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not-a-database.db");
        std::fs::write(&path, b"this is not an sqlite file at all, just text padding it past the header size....").unwrap();
        let conn = Connection::open(&path).unwrap();
        assert!(table_exists(&conn, "t").is_err());
    }
}
