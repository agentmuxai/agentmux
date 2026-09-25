// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Read-modify-write of small files in one zone, as one transaction across
//! processes (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.1.1).
//!
//! The global transcript store is one database file that every srv instance
//! on the machine opens. A caller that reads a file, decides, and writes it
//! back (the memory record's `heads.json` next to its `log.jsonl`) must do
//! all of that inside one `BEGIN IMMEDIATE` transaction, or two processes
//! both read the same state and one's write is lost. Reads here are sized
//! from the database row, never this process's cache, which another
//! process's write leaves stale; the touched files are dropped from the
//! cache after the commit.

use rusqlite::{params, OptionalExtension, Transaction};

use super::core::FileStore;
use super::counter::{new_gen, read_bytes, read_row, AppendMode};
use crate::backend::storage::error::StoreError;

/// One zone, inside one write transaction. See [`FileStore::zone_txn`].
pub struct ZoneTxn<'t> {
    tx: &'t Transaction<'t>,
    zone_id: String,
    now: i64,
    touched: Vec<String>,
}

impl ZoneTxn<'_> {
    /// A file's whole content as committed, or `None` if it doesn't exist.
    pub fn read(&self, name: &str) -> Result<Option<Vec<u8>>, StoreError> {
        read_in(self.tx, &self.zone_id, name)
    }

    /// Replace a file's content, creating the file if it doesn't exist.
    pub fn put(&mut self, name: &str, data: &[u8]) -> Result<(), StoreError> {
        self.ensure(name)?;
        FileStore::replace_content_in(self.tx, &self.zone_id, name, data, self.now)?;
        self.touch(name);
        Ok(())
    }

    /// Create a file with `data` unless it already exists; `true` if it was
    /// created. For content-addressed files, whose content never changes.
    pub fn put_if_absent(&mut self, name: &str, data: &[u8]) -> Result<bool, StoreError> {
        if read_row(self.tx, &self.zone_id, name)?.is_some() {
            return Ok(false);
        }
        self.put(name, data)?;
        Ok(true)
    }

    /// Append complete `\n`-terminated lines, creating the file if needed.
    pub fn append_lines(&mut self, name: &str, data: &[u8]) -> Result<(), StoreError> {
        self.ensure(name)?;
        FileStore::append_in_tx(self.tx, &self.zone_id, name, data, AppendMode::Lines, self.now)?;
        self.touch(name);
        Ok(())
    }

    fn ensure(&self, name: &str) -> Result<(), StoreError> {
        let exists = self
            .tx
            .query_row(
                "SELECT 1 FROM db_wave_file WHERE zoneid = ?1 AND name = ?2",
                params![self.zone_id, name],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !exists {
            self.tx.execute(
                "INSERT INTO db_wave_file (zoneid, name, size, createdts, modts, opts, meta,
                     gen, lines, lines_size, lines_tail, lines_modts, lines_rev)
                 VALUES (?1, ?2, 0, ?3, ?3, '{}', '{}', ?4, 0, 0, 0, ?3, 0)",
                params![self.zone_id, name, self.now, new_gen()],
            )?;
        }
        Ok(())
    }

    fn touch(&mut self, name: &str) {
        if !self.touched.iter().any(|n| n == name) {
            self.touched.push(name.to_string());
        }
    }
}

/// A file's whole content, sized from its row in `tx`.
fn read_in(tx: &Transaction<'_>, zone_id: &str, name: &str) -> Result<Option<Vec<u8>>, StoreError> {
    let Some(row) = read_row(tx, zone_id, name)? else { return Ok(None) };
    if row.size == 0 {
        return Ok(Some(Vec::new()));
    }
    Ok(Some(read_bytes(tx, zone_id, name, 0, row.size)?))
}

impl FileStore {
    /// Run `f` over `zone_id` as one `BEGIN IMMEDIATE` transaction: its reads
    /// see the committed state and its writes commit together, or not at all
    /// when `f` returns `Err`. Serialized against every process sharing the
    /// database.
    ///
    /// `f` runs with this store's connection locked: it must not call any
    /// other `FileStore` method (the lock isn't reentrant — that deadlocks).
    /// Use the [`ZoneTxn`] it is given.
    pub fn zone_txn<T>(
        &self,
        zone_id: &str,
        f: impl FnOnce(&mut ZoneTxn<'_>) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let now = Self::now_ms();
        let mut touched = Vec::new();
        let out = self.write_txn(|tx| {
            let mut z = ZoneTxn { tx, zone_id: zone_id.to_string(), now, touched: Vec::new() };
            let out = f(&mut z)?;
            touched = z.touched;
            Ok(out)
        })?;
        let names: Vec<&str> = touched.iter().map(String::as_str).collect();
        self.forget_cached(zone_id, &names);
        Ok(out)
    }

    /// Read several files of one zone as one consistent snapshot, sized from
    /// the database — never from this process's cache, which another
    /// process's write leaves stale.
    pub fn read_files_consistent(&self, zone_id: &str, names: &[&str]) -> Result<Vec<Option<Vec<u8>>>, StoreError> {
        self.read_txn(|tx| names.iter().map(|name| read_in(tx, zone_id, name)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ZONE: &str = "agent-uid:u1:memory";

    #[test]
    fn writes_commit_together_and_reads_see_them() {
        let fs = FileStore::open_in_memory().unwrap();
        fs.zone_txn(ZONE, |z| {
            z.put("heads.json", b"{\"a\":1}")?;
            z.append_lines("log.jsonl", b"{\"e\":1}\n")?;
            z.append_lines("log.jsonl", b"{\"e\":2}\n")
        })
        .unwrap();
        let got = fs.read_files_consistent(ZONE, &["heads.json", "log.jsonl", "missing"]).unwrap();
        assert_eq!(got[0].as_deref(), Some(&b"{\"a\":1}"[..]));
        assert_eq!(got[1].as_deref(), Some(&b"{\"e\":1}\n{\"e\":2}\n"[..]));
        assert_eq!(got[2], None);
    }

    #[test]
    fn an_error_rolls_back_every_write() {
        let fs = FileStore::open_in_memory().unwrap();
        fs.zone_txn(ZONE, |z| z.put("heads.json", b"old")).unwrap();
        let r: Result<(), _> = fs.zone_txn(ZONE, |z| {
            z.put("heads.json", b"new")?;
            z.append_lines("log.jsonl", b"x\n")?;
            Err(StoreError::Other("stale parent".into()))
        });
        assert!(r.is_err());
        let got = fs.read_files_consistent(ZONE, &["heads.json", "log.jsonl"]).unwrap();
        assert_eq!(got[0].as_deref(), Some(&b"old"[..]));
        assert_eq!(got[1], None, "the log append rolled back with it");
    }

    #[test]
    fn put_if_absent_never_changes_existing_content() {
        let fs = FileStore::open_in_memory().unwrap();
        let created = fs.zone_txn(ZONE, |z| z.put_if_absent("blob/abc", b"one")).unwrap();
        let again = fs.zone_txn(ZONE, |z| z.put_if_absent("blob/abc", b"two")).unwrap();
        assert!(created && !again);
        assert_eq!(fs.read_files_consistent(ZONE, &["blob/abc"]).unwrap()[0].as_deref(), Some(&b"one"[..]));
    }

    /// Two handles on one database file stand in for two srv processes: a
    /// read through one sees the other's rewrite at its real size, even with
    /// the old size cached.
    #[test]
    fn a_second_process_sees_a_rewrite_at_its_real_size() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("filestore.db");
        let a = FileStore::open(&path).unwrap();
        let b = FileStore::open(&path).unwrap();
        // b creates the file, so b's cache holds its row (size 0) — the
        // stale state a size-from-cache read would trust.
        b.make_file(ZONE, "heads.json", Default::default(), Default::default()).unwrap();
        a.zone_txn(ZONE, |z| z.put("heads.json", b"a much longer body")).unwrap();
        assert_eq!(b.read_file(ZONE, "heads.json").unwrap().as_deref(), Some(&b""[..]), "the cached read is stale");
        let got = b.read_files_consistent(ZONE, &["heads.json"]).unwrap();
        assert_eq!(got[0].as_deref(), Some(&b"a much longer body"[..]));
    }

    /// A read-decide-write in one process can't interleave with another's:
    /// both increments land.
    #[test]
    fn concurrent_read_modify_writes_both_land() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("filestore.db");
        let a = std::sync::Arc::new(FileStore::open(&path).unwrap());
        let b = std::sync::Arc::new(FileStore::open(&path).unwrap());
        a.zone_txn(ZONE, |z| z.put("n", b"0")).unwrap();
        let bump = |fs: std::sync::Arc<FileStore>| {
            std::thread::spawn(move || {
                for _ in 0..25 {
                    fs.zone_txn(ZONE, |z| {
                        let n: u32 = String::from_utf8(z.read("n")?.unwrap()).unwrap().parse().unwrap();
                        z.put("n", (n + 1).to_string().as_bytes())
                    })
                    .unwrap();
                }
            })
        };
        let (ta, tb) = (bump(a.clone()), bump(b.clone()));
        ta.join().unwrap();
        tb.join().unwrap();
        assert_eq!(a.read_files_consistent(ZONE, &["n"]).unwrap()[0].as_deref(), Some(&b"50"[..]));
    }
}
