// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The copy a run takes before its first write (§4 Phase 2, "Back up"):
//! every database it will touch through SQLite's `VACUUM INTO`
//! (`db_copy::copy_database`, never a raw copy of a file in WAL mode), and
//! every other file it will change or delete as it is, each with its
//! SHA-256 in the journal. Also the digests restore mode compares.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::home::{open_ro, owner_only};
use super::journal::{BackupEntry, Fingerprint};

/// What to back up: databases and plain files, by original path.
#[derive(Debug, Default)]
pub struct BackupList {
    pub databases: Vec<PathBuf>,
    pub files: Vec<PathBuf>,
}

/// Copy every listed path into `dir` and describe each copy. A database that
/// can't be copied fails the run before anything is written; a file that is
/// gone by now is skipped.
pub fn take(dir: &Path, list: &BackupList) -> Result<Vec<BackupEntry>, String> {
    std::fs::create_dir_all(dir.join("db")).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::create_dir_all(dir.join("files")).map_err(|e| format!("{}: {e}", dir.display()))?;
    for d in [dir.to_path_buf(), dir.join("db"), dir.join("files")] {
        owner_only(&d)?;
    }
    let mut out = Vec::new();
    for (i, db) in list.databases.iter().enumerate() {
        let name = db.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let copy = format!("db/{:04}-{name}", i + 1);
        crate::backend::storage::db_copy::copy_database(db, &dir.join(&copy))?;
        owner_only(&dir.join(&copy))?;
        out.push(BackupEntry { original: db.display().to_string(), sha256: file_sha256(&dir.join(&copy))?, copy, database: true });
    }
    for (i, file) in list.files.iter().enumerate() {
        let Ok(bytes) = std::fs::read(file) else { continue };
        let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let copy = format!("files/{:04}-{name}", i + 1);
        super::home::write_file(&dir.join(&copy), &bytes)?;
        out.push(BackupEntry { original: file.display().to_string(), sha256: hex::encode(Sha256::digest(&bytes)), copy, database: false });
    }
    Ok(out)
}

/// The copy of `original` among `entries`, checked against its recorded
/// checksum.
pub fn copy_of(run_dir: &Path, entries: &[BackupEntry], original: &Path) -> Result<PathBuf, String> {
    let want = original.display().to_string();
    let entry = entries.iter().find(|e| e.original == want).ok_or_else(|| format!("no backup of {want}"))?;
    let copy = run_dir.join(&entry.copy);
    let sum = file_sha256(&copy)?;
    if sum != entry.sha256 {
        return Err(format!("the backup of {want} doesn't match its checksum"));
    }
    Ok(copy)
}

pub fn file_sha256(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(hex::encode(Sha256::digest(&bytes)))
}

/// A digest of a database's rows, every table, independent of row order and
/// of how its pages sit in the file and its WAL: what changed and what
/// didn't, where a file checksum would differ after any checkpoint.
pub fn db_digest(path: &Path) -> Result<String, String> {
    let conn = open_ro(path)?;
    let mut tables: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())?
    };
    tables.sort();
    let mut all = Sha256::new();
    for table in tables {
        let mut stmt = conn.prepare(&format!("SELECT * FROM \"{table}\"")).map_err(|e| e.to_string())?;
        let width = stmt.column_count();
        let mut rows: Vec<String> = stmt
            .query_map([], |r| {
                let mut h = Sha256::new();
                for i in 0..width {
                    h.update(cell(r.get_ref(i)?).as_bytes());
                    h.update(b"\x1f");
                }
                Ok(hex::encode(h.finalize()))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        rows.sort();
        all.update(table.as_bytes());
        for row in rows {
            all.update(row.as_bytes());
        }
    }
    Ok(hex::encode(all.finalize()))
}

/// One cell, as the digest sees it. A JSON object or array (a block's
/// `data`) counts by its content, not its bytes: a row rewritten and put back
/// holds the same JSON, whatever order its keys were serialized in.
fn cell(v: rusqlite::types::ValueRef<'_>) -> String {
    use rusqlite::types::ValueRef;
    let json = match v {
        ValueRef::Text(b) | ValueRef::Blob(b) if matches!(b.first(), Some(b'{') | Some(b'[')) => serde_json::from_slice::<serde_json::Value>(b).ok(),
        _ => None,
    };
    match json {
        Some(j) => format!("json:{}", canonical(&j)),
        None => format!("{v:?}"),
    }
}

/// `v` with every object's keys sorted.
fn canonical(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let inner: Vec<String> = keys.iter().map(|k| format!("{:?}:{}", k, canonical(&map[*k]))).collect();
            format!("{{{}}}", inner.join(","))
        }
        serde_json::Value::Array(items) => format!("[{}]", items.iter().map(canonical).collect::<Vec<_>>().join(",")),
        other => other.to_string(),
    }
}

/// What `path` is now, for restore mode.
pub fn fingerprint(path: &Path, database: bool) -> Result<Fingerprint, String> {
    if database {
        return Ok(Fingerprint { path: path.display().to_string(), digest: db_digest(path)?, mtime_ms: 0 });
    }
    let mtime_ms = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let digest = if path.exists() { file_sha256(path)? } else { String::new() };
    Ok(Fingerprint { path: path.display().to_string(), digest, mtime_ms })
}
