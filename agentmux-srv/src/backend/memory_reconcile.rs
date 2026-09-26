// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Reconcile an agent's memory record with the provider folder its spawn is
//! about to use (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.1.3, M3b).
//!
//! Runs before the provider process starts, so a new account, working
//! directory or channel starts its first session WITH the agent's memory.
//! Only a directory proven exclusive (`memory_dir_claims`) is touched.
//!
//! Per file, comparing disk, the record's head, and the version last
//! projected into this directory:
//! - disk equals the head → record the projection, nothing else (rule 0:
//!   covers a crash between a write and its record);
//! - the record changed, disk didn't → write the head to disk (or, for a
//!   deletion, remove the file — only if it still holds what was projected);
//! - disk changed, the record didn't → capture it as a new version;
//! - both changed → conflict: disk's version becomes the head, marked
//!   `conflicts_with` the record's, which is written beside it as
//!   `<stem>__conflict_<short>.md` and recorded, so it follows the agent;
//! - the first time the record meets an exclusive directory, its files are
//!   adopted together as the baseline.
//!
//! Safety rules, each from a data-loss review of the first version:
//! - A file that can't be read is left alone — never taken for deleted, and
//!   never overwritten. A folder that can't be listed skips the pass. A
//!   folder that doesn't exist, or holds none of the files projected into
//!   it, has its projections forgotten first and is refilled — never
//!   tombstoned, even if the refill stops part-way.
//! - MEMORY.md is reconciled first, and every file is decided against a
//!   fresh read of the record, never a snapshot from the start of the pass.
//! - MEMORY.md is edited (to index a conflict file) only when disk, the
//!   head and this directory's projection all agree on it; otherwise a
//!   later pass adds the line.
//! - A deletion re-reads the file and removes it only if it still holds the
//!   projected content, and only in a pass that finished everything else.
//! - The pass has a time budget; exclusivity that can't be proven in time
//!   counts as shared.
//! - The folder is the one the CLI really uses: keyed by the repository's
//!   main checkout, not the working directory
//!   ([`crate::backend::claude_layout::memory_project_root`]). A folder the
//!   spawn or a settings file may redirect is left alone.
//! - Each file is read again when its turn comes, and again right before
//!   it is overwritten; a provider write in between wins, and is captured
//!   next pass.
//! - One pass per agent at a time, across AgentMux processes (a lease in
//!   the global store); a second one skips.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::backend::memory_dir_claims::{self as claims, Exclusivity};
use crate::backend::memory_record::{self as record, AppendOutcome, Heads, NewVersion};
use crate::backend::storage::error::StoreError;
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::Store;

/// The whole pass, before the spawn proceeds without it.
pub(crate) const RECONCILE_BUDGET: Duration = Duration::from_secs(1);
const INDEX_FILE: &str = "MEMORY.md";
const CONFLICT_MARK: &str = "__conflict_";

/// What a spawn's memory directory is.
pub(crate) struct SpawnMemory<'a> {
    pub uid: &'a str,
    pub provider: &'a str,
    pub config_dir: Option<&'a str>,
    pub cwd: &'a str,
    /// The spawn itself may point the CLI's memory somewhere else
    /// ([`spawn_overrides_memory_dir`]).
    pub overridden: bool,
    /// Where the agent's earlier per-channel history is imported from
    /// ([`crate::backend::memory_history_import`]); `None`: every channel
    /// store on this machine.
    pub history_stores: Option<&'a [std::path::PathBuf]>,
}

/// Whether a spawn's environment or arguments may move the CLI's memory
/// folder away from `projects/<root>/memory`: an override variable, or a
/// settings file passed on the command line (it could set
/// `autoMemoryDirectory`).
pub(crate) fn spawn_overrides_memory_dir(env: &std::collections::HashMap<String, String>, args: &[String]) -> bool {
    // CLAUDE_CODE_PROJECT_DIR_NAME replaces the `projects/<name>` folder
    // name for every working directory (the CLI's `wyo`).
    const VARS: [&str; 3] = ["CLAUDE_COWORK_MEMORY_PATH_OVERRIDE", "CLAUDE_CODE_REMOTE_MEMORY_DIR", "CLAUDE_CODE_PROJECT_DIR_NAME"];
    VARS.iter().any(|v| env.get(*v).is_some_and(|x| !x.is_empty()) || std::env::var_os(v).is_some_and(|x| !x.is_empty()))
        || args.iter().any(|a| {
            ["--settings", "--managed-settings"].iter().any(|f| a == f || a.starts_with(&format!("{f}=")))
        })
}

/// Whether a settings file the CLI reads mentions `autoMemoryDirectory`,
/// which moves its memory folder. Any mention counts, parsed or not.
/// Remote managed settings can't be seen from here.
fn settings_override_memory_dir(config_dir: &Path, root: &Path) -> bool {
    let mut files = vec![
        config_dir.join("settings.json"),
        root.join(".claude").join("settings.json"),
        root.join(".claude").join("settings.local.json"),
    ];
    if cfg!(target_os = "macos") {
        files.push("/Library/Application Support/ClaudeCode/managed-settings.json".into());
    } else if cfg!(windows) {
        files.push(r"C:\Program Files\ClaudeCode\managed-settings.json".into());
        files.push(r"C:\ProgramData\ClaudeCode\managed-settings.json".into());
    } else {
        files.push("/etc/claude-code/managed-settings.json".into());
    }
    // Drop-in fragments beside each managed-settings file.
    let drop_ins: Vec<std::path::PathBuf> = files
        .iter()
        .filter(|f| f.file_name().is_some_and(|n| n == "managed-settings.json"))
        .filter_map(|f| std::fs::read_dir(f.with_file_name("managed-settings.d")).ok())
        .flat_map(|d| d.flatten().map(|e| e.path()))
        .collect();
    files.extend(drop_ins);
    files.iter().any(|f| std::fs::read(f).is_ok_and(|b| b.windows(19).any(|w| w == b"autoMemoryDirectory")))
}

/// A lease on `uid`'s reconcile, across every AgentMux process on this
/// machine: two passes for one agent at once — two spawns of it, in one
/// process or two — each act on a view of the folder the other is
/// changing. The second one skips; its spawn goes ahead.
const PASS_LEASE_FILE: &str = "lease.json";

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct PassLease {
    #[serde(default)]
    owner: String,
    #[serde(default)]
    until_ms: i64,
}

fn pass_zone(uid: &str) -> String {
    format!("memory-pass:{}", &record::sha256_hex(uid.as_bytes())[..32])
}

/// Take the lease until `until`; `None` while another pass holds it.
pub(crate) fn take_pass_lease(fs: &FileStore, uid: &str, until: Instant) -> Result<Option<String>, StoreError> {
    let now = agentmux_common::time::now_ms();
    // Held past the budget by a margin, in case the pass overruns it
    // mid-file; a crashed pass frees it once that passes.
    let until_ms = now + until.saturating_duration_since(Instant::now()).as_millis() as i64 + 5_000;
    let owner = uuid::Uuid::new_v4().simple().to_string();
    fs.zone_txn(&pass_zone(uid), |z| {
        let held = z.read(PASS_LEASE_FILE)?.and_then(|b| serde_json::from_slice::<PassLease>(&b).ok());
        // No pass's lease legitimately runs more than a minute ahead; one
        // further out was taken while the clock was ahead, and would
        // otherwise block every pass until the clock caught up.
        if held.is_some_and(|l| l.until_ms > now && l.until_ms <= now + 60_000) {
            return Ok(None);
        }
        let lease = PassLease { owner: owner.clone(), until_ms };
        z.put(PASS_LEASE_FILE, &serde_json::to_vec(&lease).map_err(|e| StoreError::Other(e.to_string()))?)?;
        Ok(Some(owner.clone()))
    })
}

pub(crate) fn release_pass_lease(fs: &FileStore, uid: &str, owner: &str) {
    let released = fs.zone_txn(&pass_zone(uid), |z| {
        let held = z.read(PASS_LEASE_FILE)?.and_then(|b| serde_json::from_slice::<PassLease>(&b).ok());
        if held.is_some_and(|l| l.owner == owner) {
            z.put(PASS_LEASE_FILE, &serde_json::to_vec(&PassLease::default()).map_err(|e| StoreError::Other(e.to_string()))?)?;
        }
        Ok(())
    });
    if let Err(e) = released {
        tracing::warn!(uid, error = %e, "memory reconcile: lease not released; it expires on its own");
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Report {
    pub skipped: Option<&'static str>,
    pub adopted: usize,
    /// Versions imported from the per-channel history, the first time.
    pub imported: usize,
    /// Files held for the adoption list at first sighting: another agent's
    /// record already holds their content.
    pub held: usize,
    pub written: usize,
    pub captured: usize,
    pub conflicts: usize,
    pub deleted: usize,
    pub unreadable: usize,
    /// The budget ran out; the rest waits for the next spawn.
    pub deferred: bool,
}

/// The pass's time budget. `files_left` lets tests stop a pass part-way.
struct Budget {
    deadline: Instant,
    files_left: Option<usize>,
}

impl Budget {
    fn spent(&mut self) -> bool {
        if Instant::now() > self.deadline {
            return true;
        }
        match &mut self.files_left {
            Some(0) => true,
            Some(n) => {
                *n -= 1;
                false
            }
            None => false,
        }
    }
}

/// Reconcile before a spawn. Never fails the spawn: every error is logged
/// and reported as skipped.
pub(crate) fn reconcile_before_spawn(fs: &FileStore, mstore: &Store, m: &SpawnMemory<'_>, budget: Duration) -> Report {
    run(fs, mstore, m, Budget { deadline: Instant::now() + budget, files_left: None })
}

fn run(fs: &FileStore, mstore: &Store, m: &SpawnMemory<'_>, mut budget: Budget) -> Report {
    let mut report = Report::default();
    if let Err(e) = reconcile_inner(fs, mstore, m, &mut budget, &mut report) {
        tracing::warn!(uid = m.uid, error = %e, "memory reconcile stopped");
        report.skipped = Some("error");
    }
    report
}

/// A memory file as read from disk.
enum OnDisk {
    Body(Vec<u8>),
    /// Present but unreadable (permissions, over the size cap): left alone.
    Unreadable,
}

/// One memory file as it is on disk now; `None` when it doesn't exist.
fn read_one(dir: &Path, name: &str) -> Option<OnDisk> {
    let path = dir.join(name);
    match std::fs::metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Ok(meta) if !meta.is_file() => Some(OnDisk::Unreadable),
        Ok(meta) if meta.len() as usize > record::MAX_BODY_BYTES => Some(OnDisk::Unreadable),
        Ok(_) => Some(std::fs::read(&path).map(OnDisk::Body).unwrap_or(OnDisk::Unreadable)),
        Err(_) => Some(OnDisk::Unreadable),
    }
}

/// Whether `name` still holds content `sha` (`None`: still absent), read
/// again immediately before overwriting it — the provider may have
/// written it since this pass decided. What it wrote is captured next pass.
fn disk_still_holds(dir: &Path, name: &str, sha: Option<&str>) -> bool {
    match read_one(dir, name) {
        None => sha.is_none(),
        Some(OnDisk::Body(b)) => sha == Some(record::sha256_hex(&b).as_str()),
        Some(OnDisk::Unreadable) => false,
    }
}

/// The folder's memory files, or `None` when the folder doesn't exist. Any
/// other listing error fails the pass rather than read as "no files".
fn read_disk(dir: &Path) -> Result<Option<BTreeMap<String, OnDisk>>, StoreError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(StoreError::Other(format!("memory reconcile: list {}: {e}", dir.display()))),
    };
    let mut out = BTreeMap::new();
    for entry in entries {
        let entry = entry.map_err(|e| StoreError::Other(format!("memory reconcile: list: {e}")))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if crate::server::native_memory_handlers::validate_filename(&name).is_err() {
            continue;
        }
        let path = entry.path();
        let body = match std::fs::metadata(&path) {
            Ok(meta) if !meta.is_file() => continue,
            Ok(meta) if meta.len() as usize > record::MAX_BODY_BYTES => OnDisk::Unreadable,
            Ok(_) => std::fs::read(&path).map(OnDisk::Body).unwrap_or(OnDisk::Unreadable),
            Err(_) => OnDisk::Unreadable,
        };
        out.insert(name, body);
    }
    Ok(Some(out))
}

fn reconcile_inner(
    fs: &FileStore,
    mstore: &Store,
    m: &SpawnMemory<'_>,
    budget: &mut Budget,
    report: &mut Report,
) -> Result<(), StoreError> {
    if m.provider != "claude" {
        report.skipped = Some("provider");
        return Ok(());
    }
    if m.cwd.trim().is_empty() {
        report.skipped = Some("no working directory");
        return Ok(());
    }
    let dir = crate::server::native_memory_handlers::memory_dir_for_cwd(m.config_dir.unwrap_or(""), m.cwd);
    // `dir` is `<config>/projects/<name>/memory`.
    let config_dir = dir.parent().and_then(Path::parent).and_then(Path::parent).unwrap_or(&dir);
    let root = crate::backend::claude_layout::memory_project_root(&crate::backend::base::expand_home_dir_safe(m.cwd));
    if m.overridden || settings_override_memory_dir(config_dir, &root) {
        tracing::info!(uid = m.uid, "memory reconcile: the CLI's memory folder may be overridden; left as is");
        report.skipped = Some("memory folder overridden");
        return Ok(());
    }
    if let Exclusivity::Shared(reason) = claims::check_and_claim(fs, mstore, m.uid, &dir, m.cwd, budget.deadline)? {
        tracing::info!(uid = m.uid, dir = %dir.display(), ?reason, "memory reconcile: directory not exclusive; left as is");
        report.skipped = Some("shared directory");
        return Ok(());
    }
    let Some(lease) = take_pass_lease(fs, m.uid, budget.deadline)? else {
        tracing::info!(uid = m.uid, "memory reconcile: another pass for this agent is running; skipped");
        report.skipped = Some("another pass running");
        return Ok(());
    };
    let result = reconcile_dir(fs, m.uid, &dir, m.history_stores, budget, report);
    release_pass_lease(fs, m.uid, &lease);
    result
}

fn reconcile_dir(
    fs: &FileStore,
    uid: &str,
    dir: &Path,
    history_stores: Option<&[std::path::PathBuf]>,
    budget: &mut Budget,
    report: &mut Report,
) -> Result<(), StoreError> {
    let dir_id = claims::dir_id(dir);
    let disk = read_disk(dir)?;
    let dir_missing = disk.is_none();
    let disk = disk.unwrap_or_default();
    report.unreadable = disk.values().filter(|d| matches!(d, OnDisk::Unreadable)).count();

    // The agent's earlier history, once, now that its folder is proven its
    // own. History only; a failure leaves it for a later pass.
    let disk_sha = |name: &str| match disk.get(name) {
        Some(OnDisk::Body(b)) => Some(record::sha256_hex(b)),
        _ => None,
    };
    match crate::backend::memory_history_import::import_once(fs, uid, history_stores, disk_sha, budget.deadline) {
        Ok(Some(n)) => report.imported = n,
        Ok(None) => {}
        Err(e) => tracing::warn!(uid, error = %e, "memory reconcile: history import failed; retried next pass"),
    }

    // A folder that doesn't exist, or exists but holds none of the files
    // projected into it, no longer holds what was written there: forget
    // those projections first (in the record, so a refill cut short by the
    // budget or a crash still reads the rest as "never projected" next
    // time) and refill it, rather than read the absence as deletions.
    let heads_now = record::heads(fs, uid)?;
    let projected_here = heads_now.projected.get(&dir_id).is_some_and(|p| !p.is_empty());
    let none_left = !disk.keys().any(|n| heads_now.projected.get(&dir_id).is_some_and(|p| p.contains_key(n)));
    if projected_here && (dir_missing || none_left) {
        tracing::warn!(uid, dir = %dir.display(), "memory folder no longer holds its files; refilling it from the record");
        record::reset_projections(fs, uid, &dir_id)?;
    }

    if record::heads(fs, uid)?.files.is_empty() {
        adopt_baseline(fs, uid, &dir_id, &disk, budget, report)?;
        return Ok(());
    }

    // Only names the disk scan also recognises: a name it skips would be
    // written, then read back as absent and tombstoned.
    let mut names: Vec<String> = record::heads(fs, uid)?
        .files
        .keys()
        .filter(|n| crate::server::native_memory_handlers::validate_filename(n).is_ok())
        .cloned()
        .collect();
    names.extend(disk.keys().cloned());
    names.sort_by(|a, b| (a != INDEX_FILE, a).cmp(&(b != INDEX_FILE, b)));
    names.dedup();

    let mut deletions: Vec<Deletion> = Vec::new();
    let mut shas = ShaCache::default();
    for name in &names {
        if budget.spent() {
            report.deferred = true;
            break;
        }
        reconcile_file(fs, uid, dir, &dir_id, name, report, &mut deletions, &mut shas, true)?;
    }
    if report.deferred {
        return Ok(());
    }
    for d in deletions {
        delete_if_unchanged(fs, uid, dir, &dir_id, &d, report)?;
    }
    index_conflict_files(fs, uid, dir, &dir_id)?;
    Ok(())
}

/// The first time the record meets this agent's own exclusive directory:
/// every file in it, index and topics together, becomes the baseline.
fn adopt_baseline(
    fs: &FileStore,
    uid: &str,
    dir_id: &str,
    disk: &BTreeMap<String, OnDisk>,
    budget: &mut Budget,
    report: &mut Report,
) -> Result<(), StoreError> {
    // Content another agent's record already holds isn't this agent's to
    // adopt: held for the adoption list (spec §2.1.2). All of it first, in
    // one transaction, so a baseline cut short by the budget — whose rest
    // the next pass captures as new files — never takes it either.
    let shas: Vec<(String, String)> = disk
        .iter()
        .filter_map(|(name, d)| match d {
            OnDisk::Body(b) => Some((name.clone(), record::sha256_hex(b))),
            OnDisk::Unreadable => None,
        })
        .collect();
    let foreign = record::held_by_other_records(fs, uid, &shas.iter().map(|(_, s)| s.clone()).collect::<Vec<_>>())?;
    let held: Vec<(String, String)> = shas.into_iter().filter(|(_, sha)| foreign.contains(sha)).collect();
    record::hold_for_adoption(fs, uid, dir_id, &held)?;
    report.held = held.len();
    for (name, body) in disk {
        let OnDisk::Body(body) = body else { continue };
        if held.iter().any(|(n, _)| n == name) {
            continue;
        }
        if budget.spent() {
            // The rest are captured, as new files, by the next pass.
            report.deferred = true;
            return Ok(());
        }
        let outcome = record::append_version(
            fs,
            uid,
            NewVersion {
                file: name,
                body: Some(body),
                expected_parent: None,
                merged_parent: None,
                conflicts_with: None,
                source: "adopted",
                source_detail: "first sighting, own spawn dir",
                project_to: Some(dir_id),
            },
        )?;
        if matches!(outcome, AppendOutcome::Appended(_)) {
            report.adopted += 1;
        }
    }
    Ok(())
}

/// The content `version` of `name` holds, from the log.
/// Versions' content hashes for one pass, read from the log once — not
/// once per file, which made a pass quadratic in the log's length. A
/// version appended during the pass is found by reading it again.
#[derive(Default)]
pub(crate) struct ShaCache {
    shas: std::collections::HashMap<String, Option<String>>,
}

impl ShaCache {
    fn sha_of(&mut self, fs: &FileStore, uid: &str, version: &str) -> Result<Option<String>, StoreError> {
        if let Some(sha) = self.shas.get(version) {
            return Ok(sha.clone());
        }
        self.shas = record::version_shas(fs, uid)?;
        Ok(self.shas.get(version).cloned().flatten())
    }
}

fn held_here<'h>(heads: &'h Heads, dir_id: &str, name: &str) -> Option<&'h str> {
    heads.held.get(dir_id).and_then(|m| m.get(name)).map(String::as_str)
}

fn projected_version(heads: &Heads, dir_id: &str, name: &str) -> Option<String> {
    heads.projected.get(dir_id).and_then(|p| p.get(name)).cloned()
}

/// A queued deletion: remove `name` if it still holds `expected_sha`.
struct Deletion {
    name: String,
    tombstone: String,
    projected: Option<String>,
    expected_sha: String,
}

#[allow(clippy::too_many_arguments)]
fn reconcile_file(
    fs: &FileStore,
    uid: &str,
    dir: &Path,
    dir_id: &str,
    name: &str,
    report: &mut Report,
    deletions: &mut Vec<Deletion>,
    shas: &mut ShaCache,
    may_retry: bool,
) -> Result<(), StoreError> {
    // Always decide against the disk and the record as they are now, never
    // a listing from the start of the pass: an earlier file in this pass may
    // have changed the record, and the provider may have written the file.
    let on_disk = match read_one(dir, name) {
        Some(OnDisk::Unreadable) => return Ok(()),
        Some(OnDisk::Body(b)) => Some(b),
        None => None,
    };
    let on_disk = on_disk.as_deref();
    let heads = record::heads(fs, uid)?;
    // Held for the adoption list: left alone while it still holds that.
    if on_disk.is_some_and(|b| held_here(&heads, dir_id, name) == Some(record::sha256_hex(b).as_str())) {
        return Ok(());
    }
    let head = heads.files.get(name).cloned();
    let head_sha = head.as_ref().and_then(|h| h.sha256.clone());
    let disk_sha = on_disk.map(record::sha256_hex);
    let projected = projected_version(&heads, dir_id, name);

    // Rule 0: disk already holds the head (or both are absent).
    if disk_sha == head_sha {
        if let Some(h) = &head {
            if projected.as_deref() != Some(h.version.as_str()) {
                record::record_projected(fs, uid, dir_id, name, &h.version, projected_version(&heads, dir_id, name).as_deref())?;
            }
        }
        return Ok(());
    }

    let projected_sha = match &projected {
        Some(p) => shas.sha_of(fs, uid, p)?,
        None => None,
    };
    let disk_changed = match &projected {
        Some(_) => disk_sha != projected_sha,
        // Never projected here: anything on disk is news to the record.
        None => disk_sha.is_some(),
    };
    let record_changed = match (&projected, &head) {
        (Some(p), Some(h)) => &h.version != p,
        (None, Some(h)) => h.sha256.is_some(),
        (_, None) => false,
    };

    match (disk_changed, record_changed) {
        (false, true) => {
            let h = head.expect("record_changed implies a head");
            match &h.sha256 {
                Some(sha) => {
                    let body = record::body(fs, uid, sha)?
                        .ok_or_else(|| StoreError::Other(format!("memory record: body {sha} missing")))?;
                    if !disk_still_holds(dir, name, disk_sha.as_deref()) {
                        return Ok(());
                    }
                    write_atomic(dir, name, &body)?;
                    record::record_projected(fs, uid, dir_id, name, &h.version, projected_version(&heads, dir_id, name).as_deref())?;
                    report.written += 1;
                }
                // A deletion elsewhere; disk still holds what was projected.
                None => {
                    if let (Some(expected_sha), Some(_)) = (projected_sha, on_disk) {
                        deletions.push(Deletion {
                            name: name.to_string(),
                            tombstone: h.version.clone(),
                            projected: projected.clone(),
                            expected_sha,
                        });
                    }
                }
            }
        }
        (true, false) => {
            // A deletion is captured only against a real projection here.
            if on_disk.is_none() && projected.is_none() {
                return Ok(());
            }
            let outcome = record::append_version(
                fs,
                uid,
                NewVersion {
                    file: name,
                    body: on_disk,
                    expected_parent: head.as_ref().map(|h| h.version.as_str()),
                    merged_parent: None,
                    conflicts_with: None,
                    source: "provider",
                    source_detail: "",
                    project_to: Some(dir_id),
                },
            )?;
            match outcome {
                AppendOutcome::Appended(_) => report.captured += 1,
                AppendOutcome::StaleParent { .. } if may_retry => {
                    return reconcile_file(fs, uid, dir, dir_id, name, report, deletions, shas, false);
                }
                _ => {}
            }
        }
        (true, true) => {
            let h = head.expect("record_changed implies a head");
            let Some(disk_body) = on_disk else {
                // Deleted here, changed elsewhere: keep the record's version.
                if let Some(sha) = &h.sha256 {
                    if let Some(body) = record::body(fs, uid, sha)? {
                        if !disk_still_holds(dir, name, None) {
                            return Ok(());
                        }
                        write_atomic(dir, name, &body)?;
                        record::record_projected(fs, uid, dir_id, name, &h.version, projected_version(&heads, dir_id, name).as_deref())?;
                        report.written += 1;
                    }
                }
                return Ok(());
            };
            // The other side is written and recorded FIRST: if anything
            // fails before the winner is recorded, the conflict is simply
            // raised again next pass; the reverse order could lose it.
            if let Some(sha) = &h.sha256 {
                if let Some(theirs) = record::body(fs, uid, sha)? {
                    keep_conflict_copy(fs, uid, dir, dir_id, &conflict_file_name(name, &h.version), &theirs)?;
                }
            }
            let outcome = record::append_version(
                fs,
                uid,
                NewVersion {
                    file: name,
                    body: Some(disk_body),
                    expected_parent: Some(&h.version),
                    merged_parent: None,
                    conflicts_with: Some(&h.version),
                    source: "provider",
                    source_detail: "conflict",
                    project_to: Some(dir_id),
                },
            )?;
            match outcome {
                AppendOutcome::Appended(_) => report.conflicts += 1,
                AppendOutcome::StaleParent { .. } if may_retry => {
                    return reconcile_file(fs, uid, dir, dir_id, name, report, deletions, shas, false);
                }
                _ => {}
            }
        }
        (false, false) => {}
    }
    Ok(())
}

/// Write the other version of a conflicted file beside it and record it, so
/// it follows the agent to its other folders (and a deletion of it, once
/// merged, does too).
fn keep_conflict_copy(fs: &FileStore, uid: &str, dir: &Path, dir_id: &str, name: &str, body: &[u8]) -> Result<(), StoreError> {
    if dir.join(name).exists() {
        return Ok(());
    }
    write_atomic(dir, name, body)?;
    record::append_version(
        fs,
        uid,
        NewVersion {
            file: name,
            body: Some(body),
            expected_parent: None,
            merged_parent: None,
            conflicts_with: None,
            source: "agentmux",
            source_detail: "conflict copy",
            project_to: Some(dir_id),
        },
    )?;
    Ok(())
}

/// Remove a file the record deleted, if it still holds what was projected.
fn delete_if_unchanged(fs: &FileStore, uid: &str, dir: &Path, dir_id: &str, d: &Deletion, report: &mut Report) -> Result<(), StoreError> {
    let heads = record::heads(fs, uid)?;
    let still_tombstone = heads.files.get(&d.name).is_some_and(|h| h.version == d.tombstone && h.sha256.is_none());
    let still_projected = projected_version(&heads, dir_id, &d.name) == d.projected;
    if !still_tombstone || !still_projected {
        return Ok(());
    }
    let path = dir.join(&d.name);
    let Ok(current) = std::fs::read(&path) else { return Ok(()) };
    if record::sha256_hex(&current) != d.expected_sha {
        return Ok(());
    }
    std::fs::remove_file(&path).map_err(|e| StoreError::Other(format!("memory reconcile: remove: {e}")))?;
    record::record_projected(fs, uid, dir_id, &d.name, &d.tombstone, d.projected.as_deref())?;
    report.deleted += 1;
    Ok(())
}

/// Index every conflict copy in MEMORY.md — Claude loads only the index, so
/// an unindexed file would never be seen. Only when disk, the head and this
/// directory's projection all agree on MEMORY.md; otherwise a later pass.
fn index_conflict_files(fs: &FileStore, uid: &str, dir: &Path, dir_id: &str) -> Result<(), StoreError> {
    let heads = record::heads(fs, uid)?;
    // Each conflict file gets its line once: one someone removed stays gone.
    let conflict_files: Vec<&String> = heads
        .live()
        .map(|(n, _)| n)
        .filter(|n| n.contains(CONFLICT_MARK) && !heads.indexed_conflicts.contains(*n))
        .collect();
    if conflict_files.is_empty() {
        return Ok(());
    }
    let head = heads.files.get(INDEX_FILE).filter(|h| h.sha256.is_some());
    let projected = projected_version(&heads, dir_id, INDEX_FILE);
    let path = dir.join(INDEX_FILE);
    let on_disk = match std::fs::read(&path) {
        Ok(b) => Some(b),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Ok(()),
    };
    let clean = match (head, &on_disk) {
        (Some(h), Some(b)) => projected.as_deref() == Some(h.version.as_str()) && h.sha256.as_deref() == Some(&record::sha256_hex(b)),
        (None, None) => true,
        _ => false,
    };
    if !clean {
        return Ok(());
    }
    // Appended to the bytes as they are: an index that isn't UTF-8 keeps
    // every byte it had.
    let disk_sha = on_disk.as_deref().map(record::sha256_hex);
    let mut next = on_disk.unwrap_or_default();
    let before = next.clone();
    let mut already: Vec<String> = Vec::new();
    for name in &conflict_files {
        if next.windows(name.len()).any(|w| w == name.as_bytes()) {
            already.push(name.to_string());
            continue;
        }
        if !next.is_empty() && !next.ends_with(b"\n") {
            next.push(b'\n');
        }
        let of = name.split(CONFLICT_MARK).next().unwrap_or(name);
        next.extend_from_slice(
            format!("- [{of}.md — another copy]({name}) — AgentMux kept both versions of {of}.md; merge them and delete this file\n")
                .as_bytes(),
        );
    }
    if next == before {
        record::mark_conflicts_indexed(fs, uid, &already)?;
        return Ok(());
    }
    let outcome = record::append_version(
        fs,
        uid,
        NewVersion {
            file: INDEX_FILE,
            body: Some(&next),
            expected_parent: head.map(|h| h.version.as_str()),
            merged_parent: None,
            conflicts_with: None,
            source: "agentmux",
            source_detail: "conflict index line",
            project_to: Some(dir_id),
        },
    )?;
    // Written to disk only once recorded against the head it was based on.
    if matches!(outcome, AppendOutcome::Appended(_)) && disk_still_holds(dir, INDEX_FILE, disk_sha.as_deref()) {
        write_atomic(dir, INDEX_FILE, &next)?;
        let names: Vec<String> = conflict_files.iter().map(|n| n.to_string()).collect();
        record::mark_conflicts_indexed(fs, uid, &names)?;
    }
    Ok(())
}

/// How long a memory file must go unchanged before a capture while the
/// agent runs takes it.
pub(crate) const CAPTURE_SETTLE: Duration = Duration::from_secs(2);

/// A file the provider changed in `dir` since it was last projected there,
/// with the head it will be recorded on.
struct PendingCapture {
    name: String,
    body: Vec<u8>,
    head: String,
}

/// What [`capture_while_running`] would record now: files whose content
/// differs from what was projected here, while the record hasn't moved on
/// since (the head is still that projection). Files the record changed
/// elsewhere meanwhile are left for the next spawn's reconcile, which
/// raises the conflict; deletions too, with its missing-folder guards.
fn pending_captures(fs: &FileStore, uid: &str, dir: &Path, dir_id: &str, settle: Duration) -> Result<Vec<PendingCapture>, StoreError> {
    let heads = record::heads(fs, uid)?;
    let Some(projected) = heads.projected.get(dir_id) else { return Ok(Vec::new()) };
    let Some(disk) = read_disk(dir)? else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    for (name, on_disk) in disk {
        let OnDisk::Body(body) = on_disk else { continue };
        // Still being written (the watcher fires as the write starts): a
        // torn body would become the head, and the next spawn elsewhere
        // would project it. The next event or sweep takes it once settled.
        let modified = std::fs::metadata(dir.join(&name)).and_then(|m| m.modified());
        if modified.map_or(true, |t| t.elapsed().map_or(true, |age| age < settle)) {
            continue;
        }
        let sha = record::sha256_hex(&body);
        if held_here(&heads, dir_id, &name) == Some(sha.as_str()) {
            continue;
        }
        let head = heads.files.get(&name);
        if head.is_some_and(|h| h.sha256.as_deref() == Some(sha.as_str())) {
            continue;
        }
        let projected_here = projected.get(&name);
        match (head, projected_here) {
            // The provider changed a file this folder holds the head of.
            (Some(h), Some(p)) if &h.version == p => out.push(PendingCapture { name, body, head: h.version.clone() }),
            // A new file the record has never had.
            (None, None) => out.push(PendingCapture { name, body, head: String::new() }),
            _ => {}
        }
    }
    Ok(out)
}

/// Record what the provider wrote to `dir` while the agent runs — the drift
/// detector's hook (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.1.3).
/// Capture only: nothing is written to the folder. Only a folder a spawn
/// has already reconciled (it holds projections), whose claim `uid` still
/// holds alone, and only files unchanged for `settle` ([`CAPTURE_SETTLE`]).
/// Returns how many versions were recorded.
pub(crate) fn capture_while_running(fs: &FileStore, uid: &str, dir: &Path, settle: Duration) -> Result<usize, StoreError> {
    let dir_id = claims::dir_id(dir);
    // Cheap and read-only first: most sweeps find nothing to record.
    if pending_captures(fs, uid, dir, &dir_id, settle)?.is_empty() {
        return Ok(0);
    }
    if !claims::held_exclusively(fs, uid, dir)? {
        return Ok(0);
    }
    let Some(lease) = take_pass_lease(fs, uid, Instant::now() + RECONCILE_BUDGET)? else {
        // A spawn's reconcile is running; it records this itself.
        return Ok(0);
    };
    let result = (|| {
        let mut recorded = 0;
        // Decided again under the lease, against a fresh read.
        for p in pending_captures(fs, uid, dir, &dir_id, settle)? {
            let outcome = record::append_version(
                fs,
                uid,
                NewVersion {
                    file: &p.name,
                    body: Some(&p.body),
                    expected_parent: (!p.head.is_empty()).then_some(p.head.as_str()),
                    merged_parent: None,
                    conflicts_with: None,
                    source: "provider",
                    source_detail: "while running",
                    project_to: Some(&dir_id),
                },
            )?;
            if matches!(outcome, AppendOutcome::Appended(_)) {
                recorded += 1;
            }
        }
        Ok(recorded)
    })();
    release_pass_lease(fs, uid, &lease);
    result
}

/// An AgentMux write to an agent's memory — MemoryWrite, the Armory editor,
/// a revert, a bundle import — recorded before the file is written
/// (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.1.1), so the next spawn
/// doesn't take it for the provider's, and it follows the agent at once.
///
/// Only into the agent's own folder (a claim it holds alone), on the version
/// that folder holds: if the record changed the file elsewhere since, nothing
/// is recorded and the next spawn's reconcile keeps both. Never fails the
/// write: an error is logged, and a later capture records the file.
pub(crate) fn record_agentmux_write(uid: &str, dir: &Path, file: &str, body: &[u8], source: &str, source_detail: &str) {
    let Some(fs) = crate::backend::agent_session::global_transcript_store() else { return };
    if let Err(e) = record_agentmux_write_in(fs, uid, dir, file, body, source, source_detail) {
        tracing::warn!(uid, file, error = %e, "memory record: AgentMux write not recorded; a later capture records it");
    }
}

/// [`record_agentmux_write`] against `fs`. Returns whether it was recorded.
pub(crate) fn record_agentmux_write_in(
    fs: &FileStore,
    uid: &str,
    dir: &Path,
    file: &str,
    body: &[u8],
    source: &str,
    source_detail: &str,
) -> Result<bool, StoreError> {
    if record::validate_file(file).is_err() || !claims::held_exclusively(fs, uid, dir)? {
        return Ok(false);
    }
    let dir_id = claims::dir_id(dir);
    let heads = record::heads(fs, uid)?;
    let projected = projected_version(&heads, &dir_id, file);
    let head = heads.files.get(file).map(|h| h.version.clone());
    // What this write replaces here: what this folder was last given. A
    // file the record has but never projected here is left to reconcile.
    let expected = match (&projected, &head) {
        (Some(p), _) => Some(p.clone()),
        (None, None) => None,
        (None, Some(_)) => return Ok(false),
    };
    let outcome = record::append_version(
        fs,
        uid,
        NewVersion {
            file,
            body: Some(body),
            expected_parent: expected.as_deref(),
            merged_parent: None,
            conflicts_with: None,
            source,
            source_detail,
            project_to: Some(&dir_id),
        },
    )?;
    Ok(matches!(outcome, AppendOutcome::Appended(_) | AppendOutcome::Unchanged(_)))
}

/// `<stem>__conflict_<short>.md`, the stem cut so the whole name stays
/// within `validate_filename`'s 200-character stem.
pub(crate) fn conflict_file_name(name: &str, version: &str) -> String {
    let stem = name.strip_suffix(".md").unwrap_or(name);
    // A conflict on a conflict copy is named after the original, not
    // stacked (`x__conflict_a__conflict_b.md`).
    let stem = stem.split(CONFLICT_MARK).next().unwrap_or(stem);
    let short: String = version.trim_start_matches("v_").chars().take(8).collect();
    let suffix = format!("{CONFLICT_MARK}{short}");
    let keep = 200usize.saturating_sub(suffix.len()).min(stem.len());
    format!("{}{suffix}.md", &stem[..keep])
}

fn write_atomic(dir: &Path, name: &str, body: &[u8]) -> Result<(), StoreError> {
    std::fs::create_dir_all(dir).map_err(|e| StoreError::Other(format!("memory reconcile: mkdir: {e}")))?;
    let tmp = dir.join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4().simple()));
    std::fs::write(&tmp, body).map_err(|e| StoreError::Other(format!("memory reconcile: write: {e}")))?;
    std::fs::rename(&tmp, dir.join(name)).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        StoreError::Other(format!("memory reconcile: rename: {e}"))
    })
}

#[cfg(test)]
mod tests;
