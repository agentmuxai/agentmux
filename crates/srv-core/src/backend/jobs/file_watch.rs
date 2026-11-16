// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! File watch: a path in a workspace changed, and a job on the `file_watch` source starts a run
//! (docs/specs/SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md §3.2).
//!
//! A job's filter names the folder (`path`, absolute, a project and not a home folder or a drive), optionally a
//! `glob` the changed file's name under it must match (`**/*.rs`; `*` and `?` stay inside one folder level, `**`
//! crosses them) and a `change` (`created`, `modified` or `removed`). The shared [`FsWatchPool`] does the watching:
//! one recursive watch per folder, however many jobs name it. Changes are **debounced**: the first matching change
//! starts a batch, and the run starts once the folder has been quiet for two seconds (or the batch is thirty
//! seconds old), with one event for all of it, so a save of fifty files is one run and not fifty.
//!
//! The event carries **names only**: `root`, `paths` (names under the folder, at most twenty), `count`, `first`
//! and `change`. Never file contents, and never a path outside the folder. A name is data somebody else chose:
//! it is cut to 200 characters and loses control characters, and the run must treat it as text, not as an instruction.
//! Files under a `.git` folder never count: a repository's own bookkeeping is noise, not a change to the work.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Map, Value};

use super::definition::{event_source, trigger_kind};
use crate::backend::cron::{CronScheduler, JobEvent};
use crate::backend::fs_watch::{FsWatchEvent, FsWatchEventKind, FsWatchPool, Subscription};
use crate::backend::mps::{Broker, MuxEvent, EVENT_CRON_CHANGED};
use crate::backend::storage::cron::CronJob;
use crate::backend::storage::store::Store;

/// A folder must be at least this many levels deep: a project, not a home folder or a drive.
const MIN_DEPTH: usize = 3;
/// The folder has to be quiet this long before the batch's run starts, and a batch is never held longer than the second.
const QUIET_MS: i64 = 2_000;
const MAX_WAIT_MS: i64 = 30_000;
/// The most names one event carries (the rest are only counted), and the longest a name is.
const MAX_NAMES: usize = 20;
const MAX_NAME_CHARS: usize = 200;
/// A glob stays small: it is matched against every change under the folder.
const MAX_GLOB_LEVELS: usize = 12;
const MAX_GLOB_WILDCARDS: usize = 12;
/// How many distinct files a batch tells apart: past it more are only counted, so a flood cannot grow it without bound.
const MAX_SEEN: usize = 10_000;
/// How often the jobs are read again, how often a batch is looked at.
const RECONCILE: Duration = Duration::from_secs(30);
const TICK: Duration = Duration::from_millis(500);

const KEYS: [&str; 3] = ["path", "glob", "change"];
const CHANGES: [&str; 3] = ["created", "modified", "removed"];

/// A `file_watch` trigger's filter: a folder, and optionally a glob and a kind of change.
pub fn validate_filter(filter: &Map<String, Value>) -> Result<(), String> {
    let Some(path) = filter.get("path").and_then(Value::as_str) else {
        return Err("trigger: a file_watch filter names its folder as \"path\"".to_string());
    };
    folder_ok(path)?;
    if let Some(key) = filter.keys().find(|k| !KEYS.contains(&k.as_str())) {
        return Err(format!("trigger: file_watch has no filter key {key:?}; it has {}", KEYS.join(", ")));
    }
    if let Some(glob) = filter.get("glob") {
        match glob.as_str() {
            Some(g) if !g.is_empty() && !g.contains('\0') && !g.starts_with('/') && !g.split('/').any(|s| s == "..") => {
                if g.split('/').count() > MAX_GLOB_LEVELS || g.chars().filter(|c| matches!(c, '*' | '?')).count() > MAX_GLOB_WILDCARDS {
                    return Err(format!("trigger: file_watch glob has at most {MAX_GLOB_LEVELS} levels and {MAX_GLOB_WILDCARDS} wildcards"));
                }
            }
            _ => return Err("trigger: file_watch glob is a pattern like \"**/*.rs\", relative to the folder".to_string()),
        }
    }
    if let Some(change) = filter.get("change") {
        if !change.as_str().is_some_and(|c| CHANGES.contains(&c)) {
            return Err(format!("trigger: file_watch change is one of {}", CHANGES.join(", ")));
        }
    }
    Ok(())
}

/// The folder is absolute, has no `..` in it and is at least [`MIN_DEPTH`] levels deep.
fn folder_ok(path: &str) -> Result<(), String> {
    let p = Path::new(path);
    let depth = p.components().filter(|c| matches!(c, Component::Normal(_))).count();
    if !p.is_absolute() || p.components().any(|c| c == Component::ParentDir) {
        return Err("trigger: file_watch path is an absolute folder, with no \"..\" in it".to_string());
    }
    if depth < MIN_DEPTH {
        return Err(format!("trigger: file_watch path is at least {MIN_DEPTH} folders deep: a project, not a home folder or a drive"));
    }
    Ok(())
}

/// Whether `name` (folder levels joined by `/`) matches the glob: `*` and `?` stay in one level, `**` crosses any number.
/// Table-driven, so the work is the pattern's length times the name's and never more, whatever the pattern.
pub fn glob_matches(pattern: &str, name: &str) -> bool {
    let (pat, parts): (Vec<&str>, Vec<&str>) = (pattern.split('/').collect(), name.split('/').collect());
    // ok[i][j]: the pattern's levels from i match the name's levels from j.
    let mut ok = vec![vec![false; parts.len() + 1]; pat.len() + 1];
    ok[pat.len()][parts.len()] = true;
    for i in (0..pat.len()).rev() {
        for j in (0..=parts.len()).rev() {
            ok[i][j] = if pat[i] == "**" {
                ok[i + 1][j] || (j < parts.len() && ok[i][j + 1])
            } else {
                j < parts.len() && ok[i + 1][j + 1] && level(pat[i], parts[j])
            };
        }
    }
    ok[0][0]
}

/// One level: `*` any run of characters, `?` one character. One place to fall back to, so a star never costs more than a pass over the text.
fn level(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    let (mut pi, mut ti) = (0, 0);
    let mut back: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && p[pi] == '*' {
            back = Some((pi, ti));
            pi += 1;
        } else if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if let Some((star, at)) = back {
            pi = star + 1;
            ti = at + 1;
            back = Some((star, at + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|c| *c == '*')
}

/// A job's file watch: the folder, the glob and the kind of change it wants.
#[derive(Debug, Clone, PartialEq)]
pub struct Spec {
    pub root: PathBuf,
    pub glob: String,
    pub change: Option<&'static str>,
}

/// What an enabled event job on `file_watch` watches, or `None` for any other job or a filter that does not read.
pub fn spec_of(job: &CronJob) -> Option<Spec> {
    if !job.enabled || job.trigger_kind != trigger_kind::EVENT {
        return None;
    }
    let t = &job.trigger_event;
    if t.get("source").and_then(Value::as_str) != Some(event_source::FILE_WATCH) {
        return None;
    }
    let filter = t.get("filter")?.as_object()?;
    validate_filter(filter).ok()?;
    Some(Spec {
        root: PathBuf::from(filter.get("path")?.as_str()?),
        glob: filter.get("glob").and_then(Value::as_str).unwrap_or("**").to_string(),
        change: filter.get("change").and_then(Value::as_str).and_then(|c| CHANGES.iter().copied().find(|k| *k == c)),
    })
}

fn change_name(kind: FsWatchEventKind) -> Option<&'static str> {
    match kind {
        FsWatchEventKind::Created => Some("created"),
        FsWatchEventKind::Modified => Some("modified"),
        FsWatchEventKind::Removed => Some("removed"),
        FsWatchEventKind::Other => None,
    }
}

/// A name as the event carries it: cut, and without control characters.
fn clean(name: &str) -> String {
    name.chars().filter(|c| !c.is_control()).take(MAX_NAME_CHARS).collect()
}

/// The changes one job has heard since its batch began.
#[derive(Debug, Default, Clone)]
pub struct Batch {
    first_ms: i64,
    last_ms: i64,
    names: BTreeSet<String>,
    /// Every distinct file, the names beyond [`MAX_NAMES`] included.
    count: usize,
    seen: HashSet<String>,
    changes: BTreeSet<&'static str>,
}

impl Batch {
    /// The batch as an event's input.
    pub fn input(&self, root: &Path) -> Value {
        let paths: Vec<&String> = self.names.iter().take(MAX_NAMES).collect();
        let change = if self.changes.len() == 1 { *self.changes.iter().next().unwrap_or(&"modified") } else { "mixed" };
        json!({
            "root": root.to_string_lossy(),
            "paths": paths,
            "first": paths.first().map(|p| p.as_str()).unwrap_or(""),
            "count": self.count,
            "change": change,
        })
    }
}

/// Every job's open batch.
#[derive(Debug, Default)]
pub struct Batches(HashMap<String, Batch>);

impl Batches {
    pub fn note(&mut self, job: &str, name: &str, change: &'static str, now_ms: i64) {
        let batch = self.0.entry(job.to_string()).or_insert_with(|| Batch { first_ms: now_ms, ..Batch::default() });
        batch.last_ms = now_ms;
        let name = clean(name);
        if batch.seen.len() >= MAX_SEEN {
            batch.count += 1;
        } else if batch.seen.insert(name.clone()) {
            batch.count += 1;
            if batch.names.len() < MAX_NAMES * 4 {
                batch.names.insert(name);
            }
        }
        batch.changes.insert(change);
    }

    /// The batches that are done: quiet long enough, or as old as a batch is let get.
    pub fn take_due(&mut self, now_ms: i64) -> Vec<(String, Batch)> {
        let due: Vec<String> =
            self.0.iter().filter(|(_, b)| now_ms - b.last_ms >= QUIET_MS || now_ms - b.first_ms >= MAX_WAIT_MS).map(|(id, _)| id.clone()).collect();
        due.into_iter().filter_map(|id| self.0.remove(&id).map(|b| (id, b))).collect()
    }

    fn keep_only(&mut self, jobs: &HashSet<&str>) {
        self.0.retain(|id, _| jobs.contains(id.as_str()));
    }
}

struct Watching {
    job: CronJob,
    spec: Spec,
    /// The folder as the watcher reports paths under it.
    root: PathBuf,
}

#[derive(Default)]
struct Live {
    jobs: HashMap<String, Watching>,
    /// Each watched folder, and which folder it was when it was watched: a folder deleted and made again is another one,
    /// and the watch on the old one no longer hears it.
    subs: HashMap<PathBuf, (Subscription, Option<Identity>)>,
    batches: Batches,
}

/// What tells one folder from another of the same path: the device and inode (on Windows the volume and file id), and when it was made.
pub type Identity = (u64, u64, Option<std::time::SystemTime>);

pub fn identity_of(path: &Path) -> Option<Identity> {
    let meta = std::fs::metadata(path).ok()?;
    let made = meta.created().ok();
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some((meta.dev(), meta.ino(), made))
    }
    // NTFS keeps a folder's creation time when one of the same name is made again within a few seconds (file name
    // tunneling), so the time alone cannot tell the new folder from the old: the volume and file id can.
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS};
        let file = std::fs::OpenOptions::new().read(true).custom_flags(FILE_FLAG_BACKUP_SEMANTICS).open(path).ok()?;
        // SAFETY: `info` is a plain struct of integers and the handle stays open for the call.
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        let ok = unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) } != 0;
        if ok {
            let id = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
            return Some((u64::from(info.dwVolumeSerialNumber), id, made));
        }
        Some((0, 0, made))
    }
    #[cfg(not(any(unix, windows)))]
    {
        Some((0, 0, made))
    }
}

pub struct FileWatch {
    pool: Arc<FsWatchPool>,
    live: Mutex<Live>,
}

impl FileWatch {
    pub fn new(pool: Arc<FsWatchPool>) -> Arc<Self> {
        Arc::new(Self { pool, live: Mutex::new(Live::default()) })
    }

    /// Reads which jobs watch which folders: watches a folder once however many jobs name it, lets go of one no job names
    /// now, and forgets a removed job's open batch. A folder that is not there (yet) is not watched.
    pub fn reconcile(&self, jobs: Vec<CronJob>) {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        let mut now: HashMap<String, Watching> = HashMap::new();
        for job in jobs {
            let Some(spec) = spec_of(&job) else { continue };
            let Ok(root) = spec.root.canonicalize() else { continue };
            if !root.is_dir() {
                continue;
            }
            now.insert(job.id.clone(), Watching { job, spec, root });
        }
        let wanted: HashSet<PathBuf> = now.values().map(|w| w.root.clone()).collect();
        // A folder no job names now, or one that is not the folder it was (deleted and made again), is let go;
        // the second kind is then watched afresh below.
        let stale: Vec<PathBuf> =
            live.subs.iter().filter(|(root, (_, was))| !wanted.contains(*root) || *was != identity_of(root)).map(|(root, _)| root.clone()).collect();
        for root in stale {
            if let Some((sub, _)) = live.subs.remove(&root) {
                self.pool.unsubscribe(sub);
            }
        }
        let fresh: Vec<&PathBuf> = wanted.iter().filter(|r| !live.subs.contains_key(*r)).collect();
        for root in fresh {
            let sub = self.pool.subscribe_dir_recursive(root);
            live.subs.insert(root.clone(), (sub, identity_of(root)));
        }
        let ids: HashSet<&str> = now.keys().map(String::as_str).collect();
        live.batches.keep_only(&ids);
        live.jobs = now;
    }

    /// A change the pool reports: noted into the batch of every job it concerns.
    pub fn on_event(&self, ev: &FsWatchEvent, now_ms: i64) {
        let Some(change) = change_name(ev.kind) else { return };
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        let Live { jobs, batches, .. } = &mut *live;
        for (id, w) in jobs.iter() {
            let Ok(rel) = ev.path.strip_prefix(&w.root) else { continue };
            if rel.components().any(|c| c.as_os_str() == ".git") {
                continue;
            }
            let name = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
            if name.is_empty() || !glob_matches(&w.spec.glob, &name) || w.spec.change.is_some_and(|want| want != change) {
                continue;
            }
            batches.note(id, &name, change, now_ms);
        }
    }

    /// The runs that are due: each batch that has been quiet long enough, as the job and its event.
    pub fn due(&self, now_ms: i64) -> Vec<(CronJob, JobEvent)> {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        let due = live.batches.take_due(now_ms);
        due.into_iter()
            .filter_map(|(id, batch)| {
                let w = live.jobs.get(&id)?;
                let event = JobEvent { id: format!("fw:{id}:{}:{}", batch.first_ms, batch.count), source: event_source::FILE_WATCH.to_string(), input: batch.input(&w.root) };
                Some((w.job.clone(), event))
            })
            .collect()
    }
}

/// Start watching for the jobs in `jobs`: reads them now, again every thirty seconds and whenever the jobs change,
/// and starts each due batch's run through `scheduler`.
pub fn spawn(broker: &Arc<Broker>, pool: Arc<FsWatchPool>, jobs: Arc<Store>, scheduler: Arc<CronScheduler>) {
    if tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    let watch = FileWatch::new(pool.clone());
    let wake = Arc::new(tokio::sync::Notify::new());
    let poke = wake.clone();
    // The observer runs inside `Broker::publish`, under its locks: it only pokes.
    broker.add_observer(Arc::new(move |ev: &MuxEvent| {
        if ev.event == EVENT_CRON_CHANGED {
            poke.notify_one();
        }
    }));
    let mut events = pool.events();
    tokio::spawn(async move {
        let reconcile = |watch: Arc<FileWatch>, jobs: Arc<Store>| async move {
            let list = tokio::task::spawn_blocking(move || jobs.cron_list_enabled()).await.ok().and_then(Result::ok).unwrap_or_default();
            let _ = tokio::task::spawn_blocking(move || watch.reconcile(list)).await;
        };
        reconcile(watch.clone(), jobs.clone()).await;
        let mut tick = tokio::time::interval(TICK);
        let mut since = std::time::Instant::now();
        loop {
            tokio::select! {
                ev = events.recv() => match ev {
                    Ok(ev) => watch.on_event(&ev, agentmux_common::time::now_ms()),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                        tracing::warn!(missed, "jobs: file_watch fell behind the watcher and missed changes");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                },
                _ = tick.tick() => {
                    // Each in its own task: admitting a run writes the ledger, and the loop must keep draining the watcher meanwhile.
                    for (job, event) in watch.due(agentmux_common::time::now_ms()) {
                        let scheduler = scheduler.clone();
                        tokio::spawn(async move {
                            let _ = scheduler.fire_event(&job, event).await;
                        });
                    }
                    if since.elapsed() >= RECONCILE {
                        since = std::time::Instant::now();
                        reconcile(watch.clone(), jobs.clone()).await;
                    }
                }
                _ = wake.notified() => {
                    since = std::time::Instant::now();
                    reconcile(watch.clone(), jobs.clone()).await;
                }
            }
        }
    });
}
