// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Carry what channels kept to themselves into the shared store.
//!
//! Auth is shared by every channel now (`SPEC_SHARED_AUTH_ACROSS_CHANNELS_2026_10_03.md`),
//! but until 2026-10-03 every non-`stable` channel isolated it, so real data sits
//! in `channels/<ch>/identity-store.db` files the shared store has never seen. On
//! the first shared boot this pass copies it across, read-only on the source:
//!
//! | What | From | Why |
//! |---|---|---|
//! | Global Memory (global, non-system bundles) and their versions | every channel store, newest wins | each channel kept its own; the same entry written twice is one entry |
//! | Native memory and its versions | every channel store, newest wins | the shared store has none; the entries are scattered over older channels |
//! | Accounts | this channel's own isolated store and the newest same-branch predecessor | a login belongs to the build that made it; adopting every channel's would flood the Armory with test accounts |
//!
//! Everything else is left alone. Non-global bundles in particular are mostly
//! per-agent bundles each channel made for itself (4,740 distinct ones across the
//! owner's 46 stores).
//!
//! **Idempotent, and deletions stick.** A ledger (`adoption-ledger.json` in the
//! shared dir; a file, because a schema change to the shared store would stop an
//! older build opening it) records the keys already considered and each source's
//! stamp. An item deleted afterwards is not put back, and a source that has not
//! changed is not reopened. An account row keeps its stored `OAuthConfigDir`
//! pointing at the source channel's folder: the spawn path uses the stored dir,
//! so the login works at once, with no copy of the credentials.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::types::Value;
use rusqlite::{params_from_iter, Connection, OpenFlags};
use serde::{Deserialize, Serialize};

use crate::backend::storage::store::Store;

const LEDGER_FILE: &str = "adoption-ledger.json";
static TMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Longest a boot waits for the pass; what is left is picked up next boot.
pub(crate) const BOOT_BUDGET: Duration = Duration::from_secs(8);

/// What a source file looked like when it was last fully processed: the database
/// and its write-ahead log (a WAL-mode database's newest writes live there).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Stamp {
    modified_ms: u64,
    size: u64,
    wal_modified_ms: u64,
    wal_size: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Ledger {
    sources: BTreeMap<String, Stamp>,
    keys: BTreeSet<String>,
}

/// Counts for the boot log and the tests.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Report {
    pub sources_read: usize,
    pub sources_unchanged: usize,
    pub sources_failed: usize,
    pub global_memory_added: usize,
    pub global_memory_updated: usize,
    pub native_memory_added: usize,
    pub native_memory_updated: usize,
    pub accounts_added: usize,
    /// Rows the target refused (a name already taken by another id, a missing
    /// required column): skipped, not fatal.
    pub rows_refused: usize,
    /// Rows left out because they were adopted once and deleted since.
    pub deleted_stay_deleted: usize,
    pub budget_exhausted: bool,
}

impl Report {
    pub(crate) fn changed_anything(&self) -> bool {
        self.global_memory_added
            + self.global_memory_updated
            + self.native_memory_added
            + self.native_memory_updated
            + self.accounts_added
            > 0
    }
}

pub(crate) struct Inputs<'a> {
    /// The true global root (`~/.agentmux`).
    pub home: &'a Path,
    /// The directory holding `adoption-ledger.json` (`<home>/shared`).
    pub shared_dir: &'a Path,
    /// This channel's id (`AGENTMUX_CHANNEL`).
    pub channel: &'a str,
    /// This channel's own directory (`AGENTMUX_INSTANCE_DIR`), if known.
    pub instance_dir: Option<&'a Path>,
    /// The shared store's file: never a source.
    pub target_path: &'a Path,
    pub budget: Duration,
}

/// The sources for one pass. The predecessor is the newest other build of this
/// branch (same `local-<branch>-<hash>-` prefix) whose store holds an account.
#[derive(Debug, Default)]
pub(crate) struct Sources {
    /// Every channel's identity store, newest first.
    pub all: Vec<PathBuf>,
    /// This channel's own store and its predecessor's, for accounts.
    pub accounts: Vec<PathBuf>,
}

fn modified(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// `local-<branch>-<hash6>-<8 hex>` → its prefix up to and including the last `-`.
/// A per-build channel's name ends in an 8-hex build suffix; the part before it
/// names the branch, so another build of the same branch shares the prefix.
fn build_prefix(channel: &str) -> Option<&str> {
    let (prefix, suffix) = channel.rsplit_once('-')?;
    let is_build = suffix.len() == 8 && suffix.bytes().all(|b| b.is_ascii_hexdigit());
    (channel.starts_with("local-") && is_build).then(|| &channel[..prefix.len() + 1])
}

pub(crate) fn discover_sources(inp: &Inputs) -> Sources {
    let mut found: Vec<(SystemTime, PathBuf)> = Vec::new();
    let mut push = |p: PathBuf| {
        if p.is_file() && !same_file(&p, inp.target_path) && !found.iter().any(|(_, q)| same_file(q, &p)) {
            if let Some(t) = modified(&p) {
                found.push((t, p));
            }
        }
    };
    if let Ok(rd) = std::fs::read_dir(inp.home.join("channels")) {
        for e in rd.flatten() {
            push(e.path().join("identity-store.db"));
        }
    }
    // `task dev` instance folders (`dev/<branch>/<clone>/`) are deliberately NOT
    // scanned: every agent's dev builds make one, hundreds on a busy host, and
    // their stores are throwaway test data. A dev channel's OWN store is still
    // a source, through `instance_dir` below.
    if let Some(dir) = inp.instance_dir {
        push(dir.join("identity-store.db"));
    }
    // Newest first, so the first copy of an entry seen is the one that wins and
    // an older copy is only a fallback for what is still missing.
    found.sort_by_key(|(t, _)| std::cmp::Reverse(*t));

    let mut accounts = Vec::new();
    if let Some(dir) = inp.instance_dir {
        let own = dir.join("identity-store.db");
        if own.is_file() && !same_file(&own, inp.target_path) {
            accounts.push(own);
        }
    }
    if let Some(prefix) = build_prefix(inp.channel) {
        let own_name = inp.channel;
        let predecessor = found
            .iter()
            .filter(|(_, p)| {
                p.parent()
                    .and_then(|d| d.file_name())
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n != own_name && n.starts_with(prefix) && build_prefix(n).is_some())
            })
            // The build you upgraded from is the newest one that holds a login:
            // a build that was only opened and closed has nothing to carry.
            .filter(|(_, p)| has_accounts(p))
            .max_by_key(|(t, _)| *t)
            .map(|(_, p)| p.clone());
        if let Some(p) = predecessor {
            if !accounts.iter().any(|q| same_file(q, &p)) {
                accounts.push(p);
            }
        }
    }
    Sources { all: found.into_iter().map(|(_, p)| p).collect(), accounts }
}

/// Whether the store at `path` holds at least one account.
fn has_accounts(path: &Path) -> bool {
    open_source(path)
        .ok()
        .and_then(|c| c.query_row("SELECT COUNT(*) FROM db_accounts", [], |r| r.get::<_, i64>(0)).ok())
        .is_some_and(|n| n > 0)
}

fn ms(t: Option<SystemTime>) -> u64 {
    t.and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as u64)
}

fn stamp_of(path: &Path) -> Stamp {
    let wal = PathBuf::from(format!("{}-wal", path.display()));
    Stamp {
        modified_ms: ms(modified(path)),
        size: std::fs::metadata(path).map_or(0, |m| m.len()),
        wal_modified_ms: ms(modified(&wal)),
        wal_size: std::fs::metadata(&wal).map_or(0, |m| m.len()),
    }
}

fn load_ledger(shared_dir: &Path) -> Ledger {
    match std::fs::read_to_string(shared_dir.join(LEDGER_FILE)) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
            // Keep the unreadable file for inspection instead of overwriting it.
            let aside = shared_dir.join(format!("{LEDGER_FILE}.corrupt"));
            let _ = std::fs::rename(shared_dir.join(LEDGER_FILE), &aside);
            tracing::warn!(error = %e, kept = %aside.display(), "adoption: ledger unreadable, starting a new one");
            Ledger::default()
        }),
        Err(_) => Ledger::default(),
    }
}

/// Write the ledger. Several channels share one machine now, so two boots can
/// run at once: the file on disk is read again and merged first (keys only ever
/// get added, so a union loses nothing another boot recorded), and the temp
/// file is unique so two writers never interleave in one file.
fn save_ledger(shared_dir: &Path, ledger: &Ledger) {
    let mut merged = std::fs::read_to_string(shared_dir.join(LEDGER_FILE))
        .ok()
        .and_then(|s| serde_json::from_str::<Ledger>(&s).ok())
        .unwrap_or_default();
    merged.keys.extend(ledger.keys.iter().cloned());
    for (k, v) in &ledger.sources {
        merged.sources.insert(k.clone(), v.clone());
    }
    let n = TMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = shared_dir.join(format!("{LEDGER_FILE}.{}.{n}.tmp", std::process::id()));
    let ledger = &merged;
    let result = serde_json::to_vec(ledger)
        .map_err(|e| e.to_string())
        .and_then(|b| std::fs::write(&tmp, b).map_err(|e| e.to_string()))
        .and_then(|_| std::fs::rename(&tmp, shared_dir.join(LEDGER_FILE)).map_err(|e| e.to_string()));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    if let Err(e) = result {
        tracing::warn!(error = %e, "adoption: could not save the ledger; the next boot repeats the pass");
    }
}

fn open_source(path: &Path) -> rusqlite::Result<Connection> {
    let c = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    c.busy_timeout(Duration::from_millis(500))?;
    Ok(c)
}

fn table_cols(c: &Connection, table: &str) -> Vec<String> {
    let Ok(mut stmt) = c.prepare(&format!("PRAGMA table_info({table})")) else { return Vec::new() };
    stmt.query_map([], |r| r.get::<_, String>(1)).map(|rows| rows.flatten().collect()).unwrap_or_default()
}

fn quote_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

fn quote_lit(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn value_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Integer(i) => i.to_string(),
        Value::Real(f) => f.to_string(),
        Value::Text(t) => t.clone(),
        Value::Blob(b) => format!("blob{}", b.len()),
    }
}

/// How one table is copied.
struct Spec<'a> {
    table: &'static str,
    /// Ledger key prefix.
    cat: &'static str,
    key: &'a [&'static str],
    /// An INTEGER column; a source row with a larger value replaces the target's.
    newer: Option<&'static str>,
    /// A WHERE clause over the source table.
    filter: String,
}

#[derive(Default)]
struct Copied {
    added: Vec<Vec<String>>,
    updated: Vec<Vec<String>>,
    refused: usize,
    deleted_stay_deleted: usize,
}

/// Copy `spec`'s rows from `src` into `dst`. Only columns both tables have are
/// copied, so a source from an older or newer schema works.
fn copy_table(src: &Connection, dst: &Connection, spec: &Spec, ledger: &mut Ledger) -> Copied {
    let mut out = Copied::default();
    let (sc, dc) = (table_cols(src, spec.table), table_cols(dst, spec.table));
    if sc.is_empty() || dc.is_empty() || spec.key.iter().any(|k| !sc.contains(&k.to_string()) || !dc.contains(&k.to_string())) {
        return out;
    }
    let cols: Vec<&String> = dc.iter().filter(|c| sc.contains(c)).collect();
    let col_list = cols.iter().map(|c| quote_ident(c)).collect::<Vec<_>>().join(", ");
    let sql = format!("SELECT {col_list} FROM {} WHERE {}", spec.table, spec.filter);
    let rows: Vec<Vec<Value>> = match src.prepare(&sql).and_then(|mut s| {
        let n = cols.len();
        s.query_map([], |r| (0..n).map(|i| r.get::<_, Value>(i)).collect::<rusqlite::Result<Vec<_>>>())
            .map(|it| it.flatten().collect())
    }) {
        Ok(r) => r,
        Err(_) => return out,
    };
    let idx = |name: &str| cols.iter().position(|c| c.as_str() == name);
    let key_idx: Vec<usize> = spec.key.iter().filter_map(|k| idx(k)).collect();
    let where_key = spec.key.iter().enumerate().map(|(i, k)| format!("{} = ?{}", quote_ident(k), i + 1)).collect::<Vec<_>>().join(" AND ");
    let placeholders = (1..=cols.len()).map(|i| format!("?{i}")).collect::<Vec<_>>().join(", ");
    let insert_sql = format!("INSERT INTO {} ({col_list}) VALUES ({placeholders})", spec.table);
    for row in rows {
        let key_vals: Vec<Value> = key_idx.iter().map(|&i| row[i].clone()).collect();
        let key_text: Vec<String> = key_vals.iter().map(value_text).collect();
        let ledger_key = format!("{}:{}", spec.cat, key_text.join("|"));
        let existing: Option<Value> = match spec.newer {
            Some(n) => dst
                .query_row(&format!("SELECT {} FROM {} WHERE {where_key}", quote_ident(n), spec.table), params_from_iter(key_vals.iter()), |r| r.get::<_, Value>(0))
                .ok(),
            None => dst
                .query_row(&format!("SELECT 1 FROM {} WHERE {where_key}", spec.table), params_from_iter(key_vals.iter()), |r| r.get::<_, Value>(0))
                .ok(),
        };
        match existing {
            None => {
                if ledger.keys.contains(&ledger_key) {
                    out.deleted_stay_deleted += 1;
                    continue;
                }
                match dst.execute(&insert_sql, params_from_iter(row.iter())) {
                    Ok(_) => {
                        ledger.keys.insert(ledger_key);
                        out.added.push(key_text);
                    }
                    Err(_) => out.refused += 1,
                }
            }
            Some(have) => {
                ledger.keys.insert(ledger_key);
                let (Some(n), Value::Integer(have_n)) = (spec.newer, &have) else { continue };
                let Some(ni) = idx(n) else { continue };
                let Value::Integer(src_n) = &row[ni] else { continue };
                if src_n > have_n {
                    let set_cols: Vec<usize> = (0..cols.len()).filter(|i| !key_idx.contains(i)).collect();
                    if set_cols.is_empty() {
                        continue;
                    }
                    let set = set_cols.iter().enumerate().map(|(j, &i)| format!("{} = ?{}", quote_ident(cols[i]), spec.key.len() + j + 1)).collect::<Vec<_>>().join(", ");
                    let sql = format!("UPDATE {} SET {set} WHERE {where_key}", spec.table);
                    let params: Vec<Value> = key_vals.iter().cloned().chain(set_cols.iter().map(|&i| row[i].clone())).collect();
                    match dst.execute(&sql, params_from_iter(params.iter())) {
                        Ok(_) => out.updated.push(key_text),
                        Err(_) => out.refused += 1,
                    }
                }
            }
        }
    }
    out
}

fn in_list(values: impl Iterator<Item = String>) -> Option<String> {
    let v: Vec<String> = values.map(|s| quote_lit(&s)).collect();
    (!v.is_empty()).then(|| v.join(", "))
}

/// Global Memory and its versions, from one source.
fn adopt_global_memory(src: &Connection, dst: &Connection, ledger: &mut Ledger, r: &mut Report) {
    let has_system = table_cols(src, "db_bundles").iter().any(|c| c == "is_system");
    let filter = if has_system { "is_global = 1 AND COALESCE(is_system, 0) = 0" } else { "is_global = 1" };
    let c = copy_table(src, dst, &Spec { table: "db_bundles", cat: "gm", key: &["id"], newer: Some("updated_at"), filter: filter.into() }, ledger);
    r.global_memory_added += c.added.len();
    r.global_memory_updated += c.updated.len();
    r.rows_refused += c.refused;
    r.deleted_stay_deleted += c.deleted_stay_deleted;
    let ids = c.added.iter().chain(c.updated.iter()).map(|k| k[0].clone());
    if let Some(list) = in_list(ids) {
        let v = copy_table(src, dst, &Spec { table: "db_bundle_versions", cat: "gmv", key: &["id"], newer: None, filter: format!("bundle_id IN ({list})") }, ledger);
        r.rows_refused += v.refused;
    }
}

/// Native memory and its versions, from one source.
fn adopt_native_memory(src: &Connection, dst: &Connection, ledger: &mut Ledger, r: &mut Report) {
    let c = copy_table(src, dst, &Spec { table: "db_agent_native_memory", cat: "nm", key: &["agent_id", "filename"], newer: Some("updated_at"), filter: "1 = 1".into() }, ledger);
    r.native_memory_added += c.added.len();
    r.native_memory_updated += c.updated.len();
    r.rows_refused += c.refused;
    r.deleted_stay_deleted += c.deleted_stay_deleted;
    let pairs = c.added.iter().chain(c.updated.iter()).map(|k| format!("{}|{}", k[0], k[1]));
    if let Some(list) = in_list(pairs) {
        let v = copy_table(src, dst, &Spec { table: "db_agent_native_memory_versions", cat: "nmv", key: &["id"], newer: None, filter: format!("(agent_id || '|' || filename) IN ({list})") }, ledger);
        r.rows_refused += v.refused;
    }
}

/// Accounts, from one source. Per-agent cloud credentials are not adopted:
/// they belong to the MuxBus account the source channel signed in as, which
/// need not be this channel's, and they are provisioned again on first use.
fn adopt_accounts(src: &Connection, dst: &Connection, ledger: &mut Ledger, r: &mut Report) {
    let a = copy_table(src, dst, &Spec { table: "db_accounts", cat: "acct", key: &["id"], newer: None, filter: "1 = 1".into() }, ledger);
    r.accounts_added += a.added.len();
    r.rows_refused += a.refused;
    r.deleted_stay_deleted += a.deleted_stay_deleted;
}

/// Run one pass into `target`. Never fails the boot: a source that cannot be
/// read is counted and skipped, and a pass out of budget resumes next boot.
pub(crate) fn adopt(target: &Store, inp: &Inputs) -> Report {
    let started = Instant::now();
    let mut report = Report::default();
    let sources = discover_sources(inp);
    let mut ledger = load_ledger(inp.shared_dir);
    let conn = match target.conn().lock() {
        Ok(c) => c,
        Err(e) => e.into_inner(),
    };
    for path in &sources.all {
        if started.elapsed() > inp.budget {
            report.budget_exhausted = true;
            break;
        }
        let key = path.display().to_string();
        let stamp = stamp_of(path);
        let for_accounts = sources.accounts.iter().any(|a| same_file(a, path));
        // A source already processed in this role, unchanged since, is skipped.
        // The role is part of the key so a source that later becomes a
        // predecessor is still read for accounts.
        let ledger_key = format!("{key}#{}", if for_accounts { "all+accounts" } else { "all" });
        if ledger.sources.get(&ledger_key) == Some(&stamp) {
            report.sources_unchanged += 1;
            continue;
        }
        let src = match open_source(path) {
            Ok(c) => c,
            Err(e) => {
                tracing::debug!(source = %path.display(), error = %e, "adoption: cannot open a source");
                report.sources_failed += 1;
                continue;
            }
        };
        if conn.execute_batch("BEGIN IMMEDIATE").is_err() {
            report.sources_failed += 1;
            continue;
        }
        // Keys are recorded as rows go in; if the transaction does not commit
        // the rows are gone, so the keys must go too. Otherwise they would
        // read as "adopted, then deleted" and the rows would never come back.
        let keys_before = ledger.keys.clone();
        let report_before = report.clone();
        adopt_global_memory(&src, &conn, &mut ledger, &mut report);
        adopt_native_memory(&src, &conn, &mut ledger, &mut report);
        if for_accounts {
            adopt_accounts(&src, &conn, &mut ledger, &mut report);
        }
        if conn.execute_batch("COMMIT").is_err() {
            let _ = conn.execute_batch("ROLLBACK");
            ledger.keys = keys_before;
            report = report_before;
            report.sources_failed += 1;
            continue;
        }
        ledger.sources.insert(ledger_key, stamp);
        report.sources_read += 1;
    }
    drop(conn);
    save_ledger(inp.shared_dir, &ledger);
    report
}

/// The boot entry point, for a channel running with shared auth.
pub(crate) fn run_at_boot(target: &Store, target_path: &Path) {
    let Some(shared_dir) = crate::registry::resolve_global_shared_root() else { return };
    let Some(home) = shared_dir.parent().map(Path::to_path_buf) else { return };
    let channel = crate::backend::reactive::registry::local_channel_id();
    let instance_dir = std::env::var_os("AGENTMUX_INSTANCE_DIR").map(PathBuf::from);
    let inputs = Inputs {
        home: &home,
        shared_dir: &shared_dir,
        channel: &channel,
        instance_dir: instance_dir.as_deref(),
        target_path,
        budget: BOOT_BUDGET,
    };
    let started = Instant::now();
    let r = adopt(target, &inputs);
    if r.changed_anything() || r.sources_failed > 0 || r.budget_exhausted {
        tracing::info!(
            sources_read = r.sources_read,
            sources_unchanged = r.sources_unchanged,
            sources_failed = r.sources_failed,
            global_memory = r.global_memory_added + r.global_memory_updated,
            native_memory = r.native_memory_added + r.native_memory_updated,
            accounts = r.accounts_added,
            refused = r.rows_refused,
            kept_deleted = r.deleted_stay_deleted,
            budget_exhausted = r.budget_exhausted,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "adoption: carried earlier per-channel auth state into the shared store"
        );
    } else {
        tracing::debug!(sources_unchanged = r.sources_unchanged, "adoption: nothing new to carry over");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A shared-schema store at `path` (parents created).
    fn store_at(path: &Path) -> Store {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        Store::open_shared(path).unwrap()
    }

    fn sql(s: &Store, q: &str) {
        s.conn().lock().unwrap().execute_batch(q).unwrap();
    }

    fn count(s: &Store, q: &str) -> i64 {
        s.conn().lock().unwrap().query_row(q, [], |r| r.get(0)).unwrap()
    }

    fn text(s: &Store, q: &str) -> String {
        s.conn().lock().unwrap().query_row(q, [], |r| r.get(0)).unwrap()
    }

    fn bundle(s: &Store, id: &str, name: &str, global: bool, system: bool, updated: i64, body: &str) {
        sql(
            s,
            &format!(
                "INSERT INTO db_bundles (id, name, is_global, is_system, updated_at, instructions) \
                 VALUES ('{id}', '{name}', {}, {}, {updated}, '{body}'); \
                 INSERT INTO db_bundle_versions (id, bundle_id, name, instructions, content_hash) \
                 VALUES ('v-{id}-{updated}', '{id}', '{name}', '{body}', 'h{updated}');",
                global as i32, system as i32
            ),
        );
    }

    fn native(s: &Store, agent: &str, file: &str, updated: i64, body: &str) {
        sql(
            s,
            &format!(
                "INSERT INTO db_agent_native_memory (agent_id, filename, content, updated_at) \
                 VALUES ('{agent}', '{file}', '{body}', {updated}); \
                 INSERT INTO db_agent_native_memory_versions (id, agent_id, filename, content, content_hash, created_at) \
                 VALUES ('nv-{agent}-{file}-{updated}', '{agent}', '{file}', '{body}', 'h{updated}', {updated});"
            ),
        );
    }

    fn account(s: &Store, id: &str) {
        sql(
            s,
            &format!(
                "INSERT INTO db_accounts (id, name, provider, kind, secret_ref) \
                 VALUES ('{id}', 'acct-{id}', 'claude', 'oauth', '{{\"type\":\"OAuthConfigDir\",\"dir\":\"/old/{id}\"}}')"
            ),
        );
    }

    fn credential(s: &Store, agent: &str) {
        sql(s, &format!("INSERT INTO db_agent_credentials (agent_id, client_id) VALUES ('{agent}', 'client-{agent}')"));
    }

    struct Fixture {
        _tmp: tempfile::TempDir,
        home: PathBuf,
        shared: PathBuf,
        target_path: PathBuf,
        target: Store,
    }

    impl Fixture {
        fn new() -> Self {
            let tmp = tempfile::tempdir().unwrap();
            let home = tmp.path().to_path_buf();
            let shared = home.join("shared");
            let target_path = shared.join("store.db");
            let target = store_at(&target_path);
            Fixture { _tmp: tmp, home, shared, target_path, target }
        }

        fn channel_store(&self, channel: &str) -> Store {
            store_at(&self.home.join("channels").join(channel).join("identity-store.db"))
        }

        fn store_path(&self, channel: &str) -> PathBuf {
            self.home.join("channels").join(channel).join("identity-store.db")
        }

        fn run(&self, channel: &str) -> Report {
            let instance = self.home.join("channels").join(channel);
            let inputs = Inputs {
                home: &self.home,
                shared_dir: &self.shared,
                channel,
                instance_dir: Some(&instance),
                target_path: &self.target_path,
                budget: Duration::from_secs(30),
            };
            adopt(&self.target, &inputs)
        }
    }

    fn set_age(path: &Path, secs_ago: u64) {
        let f = fs::OpenOptions::new().write(true).open(path).unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(secs_ago)).unwrap();
    }

    #[test]
    fn a_build_channel_names_its_branch_prefix() {
        assert_eq!(build_prefix("local-main-b28b7a-051fbf53"), Some("local-main-b28b7a-"));
        assert_eq!(
            build_prefix("local-agent1-release-patch-bump-abe85c-306df097"),
            Some("local-agent1-release-patch-bump-abe85c-")
        );
        // not a per-build channel: no 8-hex suffix, or not a local- build
        assert_eq!(build_prefix("stable"), None);
        assert_eq!(build_prefix("dev-some-branch"), None);
        assert_eq!(build_prefix("local-main-b28b7a-xyz"), None);
        assert_eq!(build_prefix("custom-channel-051fbf53"), None);
    }

    #[test]
    fn the_predecessor_is_the_newest_other_build_of_the_same_branch() {
        let f = Fixture::new();
        for ch in ["local-main-b28b7a-00000001", "local-main-b28b7a-00000002", "local-main-b28b7a-00000003", "local-other-111111-00000004"] {
            account(&f.channel_store(ch), &format!("acct-in-{ch}"));
        }
        for (ch, age) in [
            ("local-main-b28b7a-00000001", 900),
            ("local-main-b28b7a-00000002", 500),
            ("local-main-b28b7a-00000003", 10),
            ("local-other-111111-00000004", 100),
        ] {
            set_age(&f.store_path(ch), age);
        }
        let instance = f.home.join("channels/local-main-b28b7a-00000003");
        let inputs = Inputs {
            home: &f.home,
            shared_dir: &f.shared,
            channel: "local-main-b28b7a-00000003",
            instance_dir: Some(&instance),
            target_path: &f.target_path,
            budget: Duration::from_secs(5),
        };
        let s = discover_sources(&inputs);
        let names: Vec<String> =
            s.accounts.iter().map(|p| p.parent().unwrap().file_name().unwrap().to_string_lossy().into_owned()).collect();
        // its own store, then the newest OTHER build of the same branch (not the other branch's, not the older one)
        assert_eq!(names, ["local-main-b28b7a-00000003", "local-main-b28b7a-00000002"]);
        // every channel store is a source for memory, newest first, and the target is never one
        assert_eq!(s.all.len(), 4);
        assert!(s.all.windows(2).all(|w| modified(&w[0]) >= modified(&w[1])));
        assert!(!s.all.iter().any(|p| same_file(p, &f.target_path)));
    }

    #[test]
    fn a_build_that_holds_no_login_is_passed_over_as_the_predecessor() {
        let f = Fixture::new();
        // the newest other build was only opened and closed; the one before it holds the login
        let old = f.channel_store("local-main-b28b7a-00000001");
        account(&old, "acct-real");
        f.channel_store("local-main-b28b7a-00000002");
        f.channel_store("local-main-b28b7a-00000003");
        set_age(&f.store_path("local-main-b28b7a-00000001"), 900);
        set_age(&f.store_path("local-main-b28b7a-00000002"), 100);
        let instance = f.home.join("channels/local-main-b28b7a-00000003");
        let inputs = Inputs {
            home: &f.home,
            shared_dir: &f.shared,
            channel: "local-main-b28b7a-00000003",
            instance_dir: Some(&instance),
            target_path: &f.target_path,
            budget: Duration::from_secs(5),
        };
        let s = discover_sources(&inputs);
        let names: Vec<String> =
            s.accounts.iter().map(|p| p.parent().unwrap().file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["local-main-b28b7a-00000003", "local-main-b28b7a-00000001"]);
        let r = adopt(&f.target, &inputs);
        assert_eq!(r.accounts_added, 1, "{r:?}");
    }

    #[test]
    fn global_memory_is_unioned_newest_wins_and_leaves_the_rest_behind() {
        let f = Fixture::new();
        let a = f.channel_store("local-main-b28b7a-0000000a");
        let b = f.channel_store("local-main-b28b7a-0000000b");
        set_age(&f.store_path("local-main-b28b7a-0000000a"), 200);
        set_age(&f.store_path("local-main-b28b7a-0000000b"), 100);
        bundle(&a, "gm-x", "GitHub and AWS access", true, false, 10, "old text");
        bundle(&a, "junk-1", "Camper - ABF", false, false, 10, "a per-agent bundle");
        bundle(&b, "gm-x", "GitHub and AWS access", true, false, 20, "newer text");
        bundle(&b, "gm-y", "Use gh-agent", true, false, 5, "second entry");
        bundle(&b, "sys-1", "System global", true, true, 5, "seeded by the app");

        let r = f.run("local-main-b28b7a-0000000c");
        assert_eq!(r.global_memory_added, 2, "{r:?}");
        // (a fresh shared store seeds its own `blank` bundle, which is not ours to count)
        assert_eq!(count(&f.target, "SELECT COUNT(*) FROM db_bundles WHERE id <> 'blank'"), 2);
        assert_eq!(
            text(&f.target, "SELECT instructions FROM db_bundles WHERE id = 'gm-x'"),
            "newer text",
            "the newest copy wins"
        );
        assert_eq!(
            count(&f.target, "SELECT COUNT(*) FROM db_bundles WHERE id IN ('junk-1', 'sys-1')"),
            0,
            "non-global and system bundles stay behind"
        );
        assert!(
            count(&f.target, "SELECT COUNT(*) FROM db_bundle_versions WHERE bundle_id = 'gm-x'") >= 1,
            "versions come along"
        );
        // read-only on the sources
        assert_eq!(count(&a, "SELECT COUNT(*) FROM db_bundles WHERE id = 'gm-x'"), 1);
        assert_eq!(count(&a, "SELECT COUNT(*) FROM db_bundles WHERE id <> 'blank'"), 2, "nothing was removed from the source");
    }

    #[test]
    fn native_memory_is_unioned_by_agent_and_file_newest_wins() {
        let f = Fixture::new();
        let a = f.channel_store("local-main-b28b7a-0000000a");
        let b = f.channel_store("local-main-b28b7a-0000000b");
        set_age(&f.store_path("local-main-b28b7a-0000000a"), 200);
        native(&a, "agent-1", "notes.md", 5, "older notes");
        native(&a, "agent-1", "only-in-a.md", 5, "a own");
        native(&b, "agent-1", "notes.md", 9, "newer notes");
        native(&b, "agent-2", "other.md", 1, "b own");

        let r = f.run("local-main-b28b7a-0000000c");
        assert_eq!(r.native_memory_added, 3, "{r:?}");
        assert_eq!(count(&f.target, "SELECT COUNT(*) FROM db_agent_native_memory"), 3);
        assert_eq!(
            text(&f.target, "SELECT content FROM db_agent_native_memory WHERE agent_id='agent-1' AND filename='notes.md'"),
            "newer notes"
        );
        assert!(count(&f.target, "SELECT COUNT(*) FROM db_agent_native_memory_versions") >= 3);
    }

    #[test]
    fn accounts_come_only_from_this_channel_and_its_predecessor_and_credentials_not_at_all() {
        let f = Fixture::new();
        let own = f.channel_store("local-main-b28b7a-0000000c");
        let pred = f.channel_store("local-main-b28b7a-0000000b");
        let stranger = f.channel_store("local-main-b28b7a-0000000a");
        let other_branch = f.channel_store("local-other-111111-0000000d");
        set_age(&f.store_path("local-main-b28b7a-0000000a"), 300);
        set_age(&f.store_path("local-main-b28b7a-0000000b"), 100);
        set_age(&f.store_path("local-other-111111-0000000d"), 50);
        account(&own, "acct-own");
        credential(&own, "agent-own");
        account(&pred, "acct-pred");
        credential(&pred, "agent-pred");
        account(&stranger, "acct-stranger");
        credential(&stranger, "agent-stranger");
        account(&other_branch, "acct-other-branch");

        let r = f.run("local-main-b28b7a-0000000c");
        assert_eq!(r.accounts_added, 2, "{r:?}");
        assert_eq!(count(&f.target, "SELECT COUNT(*) FROM db_agent_credentials"), 0, "credentials are not adopted");
        let ids = text(&f.target, "SELECT group_concat(id) FROM (SELECT id FROM db_accounts ORDER BY id)");
        assert_eq!(ids, "acct-own,acct-pred");
        // the stored folder still points at the source channel, so the login works at once
        assert!(text(&f.target, "SELECT secret_ref FROM db_accounts WHERE id='acct-pred'").contains("/old/acct-pred"));
    }

    #[test]
    fn a_second_pass_reads_nothing_and_changes_nothing() {
        let f = Fixture::new();
        let a = f.channel_store("local-main-b28b7a-0000000a");
        bundle(&a, "gm-x", "Entry", true, false, 10, "text");
        native(&a, "agent-1", "n.md", 1, "x");
        let first = f.run("local-main-b28b7a-0000000c");
        assert!(first.changed_anything() && first.sources_read >= 1);
        let second = f.run("local-main-b28b7a-0000000c");
        assert!(!second.changed_anything(), "{second:?}");
        assert_eq!(second.sources_read, 0, "an unchanged source is not reopened: {second:?}");
        assert!(second.sources_unchanged >= 1);
    }

    #[test]
    fn something_deleted_after_adoption_is_not_put_back() {
        let f = Fixture::new();
        let pred = f.channel_store("local-main-b28b7a-0000000b");
        f.channel_store("local-main-b28b7a-0000000c");
        account(&pred, "acct-pred");
        bundle(&pred, "gm-x", "Entry", true, false, 10, "text");
        native(&pred, "agent-1", "n.md", 1, "x");
        let r = f.run("local-main-b28b7a-0000000c");
        assert_eq!((r.accounts_added, r.global_memory_added, r.native_memory_added), (1, 1, 1), "{r:?}");

        // the owner deletes all three from the shared store...
        sql(
            &f.target,
            "DELETE FROM db_accounts; DELETE FROM db_bundle_versions; DELETE FROM db_bundles; DELETE FROM db_agent_native_memory;",
        );
        // ...and the source changes (the old build is still running and writes something)
        native(&pred, "agent-9", "later.md", 2, "new unrelated entry");
        set_age(&f.store_path("local-main-b28b7a-0000000b"), 1);
        let r2 = f.run("local-main-b28b7a-0000000c");
        assert_eq!(count(&f.target, "SELECT COUNT(*) FROM db_accounts"), 0, "{r2:?}");
        assert_eq!(count(&f.target, "SELECT COUNT(*) FROM db_bundles"), 0);
        assert_eq!(count(&f.target, "SELECT COUNT(*) FROM db_agent_native_memory WHERE agent_id = 'agent-1'"), 0);
        assert_eq!(
            count(&f.target, "SELECT COUNT(*) FROM db_agent_native_memory WHERE agent_id = 'agent-9'"),
            1,
            "genuinely new data still arrives"
        );
        assert!(r2.deleted_stay_deleted >= 3, "{r2:?}");
    }

    #[test]
    fn a_name_the_target_already_has_under_another_id_is_refused_not_fatal() {
        let f = Fixture::new();
        bundle(&f.target, "mine", "Same name", true, false, 1, "target own");
        let a = f.channel_store("local-main-b28b7a-0000000a");
        bundle(&a, "theirs", "Same name", true, false, 99, "source copy");
        bundle(&a, "gm-ok", "Different name", true, false, 1, "fine");
        let r = f.run("local-main-b28b7a-0000000c");
        assert_eq!(r.rows_refused, 1, "{r:?}");
        assert_eq!(r.global_memory_added, 1);
        assert_eq!(text(&f.target, "SELECT instructions FROM db_bundles WHERE name='Same name'"), "target own");
    }

    #[test]
    fn a_source_with_an_older_schema_is_still_read() {
        let f = Fixture::new();
        // an old per-channel store: no is_system column, fewer bundle columns, no versions table
        let dir = f.home.join("channels/local-main-b28b7a-0000000a");
        fs::create_dir_all(&dir).unwrap();
        let old = Connection::open(dir.join("identity-store.db")).unwrap();
        old.execute_batch(
            "CREATE TABLE db_bundles (id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, is_global INTEGER NOT NULL DEFAULT 0, instructions TEXT NOT NULL DEFAULT '', updated_at INTEGER NOT NULL DEFAULT 0);
             INSERT INTO db_bundles (id, name, is_global, instructions, updated_at) VALUES ('gm-old', 'Old entry', 1, 'from an old build', 3);
             CREATE TABLE db_agent_native_memory (agent_id TEXT NOT NULL, filename TEXT NOT NULL, content TEXT NOT NULL, updated_at INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (agent_id, filename));
             INSERT INTO db_agent_native_memory VALUES ('agent-1', 'f.md', 'old memory', 3);",
        )
        .unwrap();
        drop(old);
        let r = f.run("local-main-b28b7a-0000000c");
        assert_eq!((r.global_memory_added, r.native_memory_added), (1, 1), "{r:?}");
        assert_eq!(text(&f.target, "SELECT instructions FROM db_bundles WHERE id='gm-old'"), "from an old build");
    }

    #[test]
    fn a_broken_source_is_counted_and_the_pass_carries_on() {
        let f = Fixture::new();
        let dir = f.home.join("channels/local-main-b28b7a-0000000a");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("identity-store.db"), b"this is not a sqlite database at all").unwrap();
        let good = f.channel_store("local-main-b28b7a-0000000b");
        bundle(&good, "gm-x", "Entry", true, false, 1, "text");
        let r = f.run("local-main-b28b7a-0000000c");
        assert_eq!(r.global_memory_added, 1, "{r:?}");
    }

    #[test]
    fn the_shared_store_is_never_read_as_a_source() {
        let f = Fixture::new();
        // a store at a channel path that IS the target
        bundle(&f.target, "gm-own", "Already shared", true, false, 1, "x");
        let inputs = Inputs {
            home: &f.home,
            shared_dir: &f.shared,
            channel: "local-main-b28b7a-0000000c",
            instance_dir: None,
            target_path: &f.target_path,
            budget: Duration::from_secs(5),
        };
        // the target lives under shared/, outside channels/, and is excluded even if listed
        let s = discover_sources(&inputs);
        assert!(s.all.is_empty() && s.accounts.is_empty());
        let r = adopt(&f.target, &inputs);
        assert_eq!(r.sources_read, 0, "{r:?}");
        assert_eq!(count(&f.target, "SELECT COUNT(*) FROM db_bundles WHERE id <> 'blank'"), 1);
    }

    #[test]
    fn a_source_whose_commit_fails_leaves_no_ledger_trace_and_is_retried() {
        let f = Fixture::new();
        let a = f.channel_store("local-main-b28b7a-0000000a");
        bundle(&a, "gm-x", "Entry", true, false, 10, "text");
        // A deferred foreign key violated by a trigger makes COMMIT (not the INSERT) fail.
        sql(
            &f.target,
            "CREATE TABLE fk_parent (id TEXT PRIMARY KEY);
             CREATE TABLE fk_child (pid TEXT REFERENCES fk_parent(id) DEFERRABLE INITIALLY DEFERRED);
             CREATE TRIGGER break_commit AFTER INSERT ON db_bundles BEGIN INSERT INTO fk_child VALUES ('missing'); END;",
        );
        let r = f.run("local-main-b28b7a-0000000c");
        assert_eq!(r.sources_failed, 1, "{r:?}");
        assert_eq!(r.global_memory_added, 0, "a rolled-back source counts nothing: {r:?}");
        assert_eq!(count(&f.target, "SELECT COUNT(*) FROM db_bundles WHERE id <> 'blank'"), 0);

        // The keys must not have been recorded, or the retry would read them as "deleted".
        sql(&f.target, "DROP TRIGGER break_commit");
        let again = f.run("local-main-b28b7a-0000000c");
        assert_eq!(again.global_memory_added, 1, "{again:?}");
        assert_eq!(again.deleted_stay_deleted, 0, "{again:?}");
        assert_eq!(count(&f.target, "SELECT COUNT(*) FROM db_bundles WHERE id = 'gm-x'"), 1);
    }

    #[test]
    fn two_boots_saving_the_ledger_merge_instead_of_overwriting() {
        let f = Fixture::new();
        let mut one = Ledger::default();
        one.keys.insert("gm:one".into());
        let mut two = Ledger::default();
        two.keys.insert("gm:two".into());
        save_ledger(&f.shared, &one);
        // the second boot never saw the first one's keys
        save_ledger(&f.shared, &two);
        let saved = load_ledger(&f.shared);
        assert!(saved.keys.contains("gm:one") && saved.keys.contains("gm:two"), "{:?}", saved.keys);
        let leftovers: Vec<_> = fs::read_dir(&f.shared)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp files left behind: {leftovers:?}");
    }

    #[test]
    fn an_unreadable_ledger_is_kept_aside_not_overwritten() {
        let f = Fixture::new();
        fs::write(f.shared.join(LEDGER_FILE), b"{ this is not json").unwrap();
        let ledger = load_ledger(&f.shared);
        assert!(ledger.keys.is_empty());
        assert!(f.shared.join(format!("{LEDGER_FILE}.corrupt")).is_file(), "the unreadable file is kept for inspection");
    }

    /// Phase 0 of `SPEC_SHARED_AUTH_ACROSS_CHANNELS_2026_10_03.md`: run the real
    /// pass over a real `~/.agentmux` into a throwaway store and print what it
    /// would carry over, counts only. The sources are opened read-only and the
    /// target is a temp file, so nothing real is written.
    ///
    /// ```text
    /// AGENTMUX_ADOPTION_DRYRUN_HOME=~/.agentmux
    /// AGENTMUX_ADOPTION_DRYRUN_CHANNEL=local-main-b28b7a-ffffffff
    /// cargo test -p agentmux-srv --bins -- --ignored --nocapture adoption_dry_run
    /// ```
    #[test]
    #[ignore = "reads a real home; run by hand"]
    fn adoption_dry_run_against_a_real_home() {
        let Some(home) = std::env::var_os("AGENTMUX_ADOPTION_DRYRUN_HOME").map(PathBuf::from) else {
            eprintln!("AGENTMUX_ADOPTION_DRYRUN_HOME is not set; nothing to do");
            return;
        };
        let channel = std::env::var("AGENTMUX_ADOPTION_DRYRUN_CHANNEL").unwrap_or_else(|_| "local-main-b28b7a-ffffffff".into());
        let tmp = tempfile::tempdir().unwrap();
        let shared = tmp.path().join("shared");
        let target_path = shared.join("store.db");
        let target = store_at(&target_path);
        let instance = tmp.path().join("this-channel");
        let inputs = Inputs { home: &home, shared_dir: &shared, channel: &channel, instance_dir: Some(&instance), target_path: &target_path, budget: Duration::from_secs(120) };
        let src = discover_sources(&inputs);
        eprintln!("sources: {} (accounts from {} of them)", src.all.len(), src.accounts.len());
        for p in &src.accounts {
            eprintln!("  accounts from: {}", p.parent().and_then(|d| d.file_name()).unwrap().to_string_lossy());
        }
        let started = Instant::now();
        let r = adopt(&target, &inputs);
        eprintln!("{r:#?}
elapsed: {} ms", started.elapsed().as_millis());
        eprintln!(
            "target now holds: {} global memory, {} native memory, {} accounts, {} agent credentials",
            count(&target, "SELECT COUNT(*) FROM db_bundles WHERE is_global = 1 AND is_system = 0"),
            count(&target, "SELECT COUNT(*) FROM db_agent_native_memory"),
            count(&target, "SELECT COUNT(*) FROM db_accounts"),
            count(&target, "SELECT COUNT(*) FROM db_agent_credentials"),
        );
    }
}
