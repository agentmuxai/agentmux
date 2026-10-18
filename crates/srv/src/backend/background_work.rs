// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What agents left running in the background, and what a quit does about it
//! (`docs/specs/SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md` §6.2).
//!
//! Everything an agent starts runs in its pane's process tracker (a Windows
//! job object, a Linux cgroup), which nothing can leave, `Start-Process`
//! included; a quit closes the panes and the trackers with them, so it stops
//! a dev server or a whole `task dev` AgentMux an agent started. That stays
//! so (letting processes escape a pane was rejected on purpose:
//! `SPEC_BACKGROUND_TASK_TEARDOWN_SURVIVAL_2026_08_20.md` §0). What changes is
//! that it no longer happens without a word:
//!
//! - **Before:** [`running`] lists it, by agent ([`describe`]): the version
//!   panel shows the list while an upgrade waits to restart, the launcher's
//!   dialog (`updates/stopping.txt`, [`write_for_launcher`]) and an agent
//!   asking for the upgrade are told.
//! - **At the quit** ([`at_quit`], from the quit's session save): it is
//!   recorded, and on Windows another AgentMux among it is told the session
//!   is ending (`WM_QUERYENDSESSION`), so it saves all its windows as at an
//!   OS shutdown before its tracker stops it. Its windows aren't closed: a
//!   close deletes the window's workspace, and stopped as it is, its windows
//!   all come back on its next start. Elsewhere it is stopped with its pane
//!   as before, which keeps its windows the same way.
//! - **After:** each agent that resumes is told what of its own was stopped
//!   ([`on_registered`]), once, so it can start it again.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::backend::process_tracker::TrackedProcess;

/// The record of what the last quit stopped, in the channel folder (beside
/// `upgrade_notice`'s): a version's own data folder is new after an upgrade.
pub const RECORD_FILE: &str = "agentmux-stopped-at-quit.json";
/// How long after a quit resumed agents are still told.
const TELL_FOR: Duration = Duration::from_secs(60 * 60);
/// How long another AgentMux has to save its windows, within the host's
/// 1.5 s for the whole quit save.
const SAVE_WAIT_MS: u32 = 900;
/// The longest command shown for one item.
const MAX_LABEL: usize = 90;

/// Executables of an AgentMux instance: its launcher, host and srv. An
/// agent's own `agentmux-mcp` and `agentmux-bashwrap` are plumbing, not one.
const AGENTMUX_IMAGES: [&str; 4] = ["agentmux", "agentmux-launcher", "agentmux-cef", "agentmux-srv"];
/// The AgentMux process that owns the windows.
const HOST_IMAGE: &str = "agentmux-cef";

/// One process tree an agent left running.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Work {
    /// The agent's name, as it is addressed.
    pub agent: String,
    /// Its id for the note (the registry uid, or the name).
    pub key: String,
    /// The tree's root command, shortened.
    pub label: String,
    /// The tree holds another AgentMux.
    pub agentmux: bool,
    /// When its root started (unix ms), 0 if unknown.
    pub started_at_ms: u64,
    /// Its AgentMux host process, asked to save its windows at a quit.
    #[serde(skip)]
    pub hosts: Vec<u32>,
}

/// Lowercased executable name without directory or `.exe`.
fn image(p: &TrackedProcess) -> String {
    let path = if p.exe.is_empty() { p.command.split_whitespace().next().unwrap_or("") } else { &p.exe };
    let base = path.trim_matches('"').rsplit(['\\', '/']).next().unwrap_or(path).to_ascii_lowercase();
    base.strip_suffix(".exe").map(str::to_string).unwrap_or(base)
}

/// `command` cut to [`MAX_LABEL`] characters, on one line.
fn shorten(command: &str) -> String {
    let one_line = command.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= MAX_LABEL {
        return one_line;
    }
    let mut cut: String = one_line.chars().take(MAX_LABEL - 1).collect();
    cut.push('…');
    cut
}

/// The process trees among `members` (what an agent started: the tracker's
/// `list_block`), one [`Work`] per tree: a root is a member whose parent
/// isn't one.
pub fn trees(agent: &str, key: &str, members: &[TrackedProcess]) -> Vec<Work> {
    let pids: HashSet<u32> = members.iter().map(|p| p.pid).collect();
    let mut children: HashMap<u32, Vec<&TrackedProcess>> = HashMap::new();
    for p in members {
        if let Some(parent) = p.parent_pid.filter(|pp| pids.contains(pp) && *pp != p.pid) {
            children.entry(parent).or_default().push(p);
        }
    }
    let mut out: Vec<Work> = members
        .iter()
        .filter(|p| p.parent_pid.is_none_or(|pp| !pids.contains(&pp) || pp == p.pid))
        .map(|root| {
            let mut tree = vec![root];
            let mut seen = HashSet::from([root.pid]);
            let mut i = 0;
            while i < tree.len() {
                for c in children.get(&tree[i].pid).into_iter().flatten() {
                    if seen.insert(c.pid) {
                        tree.push(c);
                    }
                }
                i += 1;
            }
            let images: Vec<String> = tree.iter().map(|p| image(p)).collect();
            Work {
                agent: agent.to_string(),
                key: key.to_string(),
                label: shorten(if root.command.is_empty() { &root.exe } else { &root.command }),
                agentmux: images.iter().any(|i| AGENTMUX_IMAGES.contains(&i.as_str())),
                started_at_ms: root.started_at_ms,
                hosts: tree
                    .iter()
                    .zip(&images)
                    .filter(|(p, i)| {
                        // The host itself, not its CEF helpers (renderer, GPU):
                        // they have a `--type=` and a host for a parent.
                        let parent_is_host =
                            tree.iter().zip(&images).any(|(q, qi)| Some(q.pid) == p.parent_pid && *qi == HOST_IMAGE);
                        *i == HOST_IMAGE && !p.command.contains("--type=") && !parent_is_host
                    })
                    .map(|(p, _)| p.pid)
                    .collect(),
            }
        })
        .collect();
    out.sort_by_key(|w| (!w.agentmux, w.started_at_ms));
    out
}

/// What every running agent left running now, by agent.
pub fn running() -> Vec<Work> {
    let Some(registry) = crate::backend::process_tracker::registry::global() else { return Vec::new() };
    let handler = crate::backend::reactive::get_global_handler();
    let mut out = Vec::new();
    for r in handler.list_agents() {
        let members = registry.list_block(&r.block_id);
        if members.is_empty() {
            continue;
        }
        let key = r.uid.clone().unwrap_or_else(|| r.agent_id.clone());
        out.extend(trees(&r.agent_id, &key, &members));
    }
    out
}

/// "2 h", "35 min", "a moment", for how long something ran.
fn ran_for(started_at_ms: u64, now_ms: u64) -> Option<String> {
    if started_at_ms == 0 || now_ms <= started_at_ms {
        return None;
    }
    let mins = (now_ms - started_at_ms) / 60_000;
    Some(match mins {
        0 => "a moment".to_string(),
        1..=59 => format!("{mins} min"),
        _ => format!("{} h", mins / 60),
    })
}

/// One line for the user: "Agent-1: an AgentMux instance (`task dev`), running 9 h".
pub fn describe(w: &Work, now_ms: u64) -> String {
    let what = if w.agentmux { format!("an AgentMux instance (`{}`)", w.label) } else { format!("`{}`", w.label) };
    match ran_for(w.started_at_ms, now_ms) {
        Some(t) => format!("{}: {what}, running {t}", w.agent),
        None => format!("{}: {what}", w.agent),
    }
}

/// Where the launcher reads the list for its dialog: beside the staged
/// update (`updates/` in this version's data folder).
pub fn launcher_file() -> Option<PathBuf> {
    std::env::var_os("AGENTMUX_DATA_DIR").filter(|d| !d.is_empty()).map(|d| PathBuf::from(d).join("updates").join("stopping.txt"))
}

/// Write `lines` for the launcher's dialog (an empty list removes the file).
pub fn write_for_launcher(lines: &[String]) {
    let Some(path) = launcher_file() else { return };
    if lines.is_empty() {
        let _ = std::fs::remove_file(&path);
        return;
    }
    let written = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::write(&path, lines.join("\n")));
    if let Err(e) = written {
        tracing::warn!(path = %path.display(), error = %e, "background work: couldn't write the list for the launcher");
    }
}

// ── At the quit ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct Record {
    at_ms: i64,
    items: Vec<Work>,
}

fn record_path(dir: &Path) -> PathBuf {
    dir.join(RECORD_FILE)
}

fn read(dir: &Path) -> Option<Record> {
    std::fs::read(record_path(dir)).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

fn write(dir: &Path, record: &Record) {
    let path = record_path(dir);
    if record.items.is_empty() {
        let _ = std::fs::remove_file(&path);
        return;
    }
    let tmp = path.with_extension("json.tmp");
    let written = serde_json::to_vec_pretty(record)
        .map_err(|e| e.to_string())
        .and_then(|b| std::fs::write(&tmp, b).map_err(|e| e.to_string()))
        .and_then(|()| std::fs::rename(&tmp, &path).map_err(|e| e.to_string()));
    if let Err(e) = written {
        tracing::warn!(path = %path.display(), error = %e, "background work: couldn't save what the quit stopped");
    }
}

/// The channel folder the record lives in.
fn channel_dir() -> Option<PathBuf> {
    agentmux_common::DataPaths::from_env().map(|p| p.instance_dir)
}

/// A quit is about to close every pane (`SaveSessionSnapshot` with
/// `for_quit`): record what agents left running, for their note after the
/// next start, and on Windows tell another AgentMux among it that the
/// session is ending, so it saves its windows before its tracker stops it.
pub async fn at_quit() {
    let work = tokio::task::spawn_blocking(running).await.unwrap_or_default();
    if work.is_empty() {
        return;
    }
    let now = agentmux_common::time::now_ms_u64();
    tracing::info!(stopping = ?work.iter().map(|w| describe(w, now)).collect::<Vec<_>>(), "background work: the quit stops what agents left running");
    if let Some(dir) = channel_dir() {
        write(&dir, &Record { at_ms: now as i64, items: work.clone() });
    }
    let hosts: Vec<u32> = work.iter().flat_map(|w| w.hosts.iter().copied()).collect();
    if !hosts.is_empty() {
        // All at once, so several instances still answer within the one wait.
        let _ = tokio::task::spawn_blocking(move || {
            std::thread::scope(|s| {
                for pid in &hosts {
                    s.spawn(move || save_windows_of(*pid));
                }
            })
        })
        .await;
    }
}

/// Tell host process `pid` the session is ending, as Windows does at a
/// shutdown: `WM_QUERYENDSESSION` to one of its visible windows, which saves
/// every window (`client/wndproc.rs`'s session-end flush), and waits for the
/// answer up to [`SAVE_WAIT_MS`]. Nothing is closed.
#[cfg(windows)]
fn save_windows_of(pid: u32) {
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, TRUE};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, IsWindowVisible, SendMessageTimeoutW, SMTO_ABORTIFHUNG,
        WM_QUERYENDSESSION,
    };
    struct Find {
        pid: u32,
        found: HWND,
    }
    unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: lparam is the `Find` passed below, alive for the whole call.
        let find = unsafe { &mut *(lparam as *mut Find) };
        let mut owner = 0u32;
        // SAFETY: hwnd comes from EnumWindows; owner is a valid out pointer.
        unsafe { GetWindowThreadProcessId(hwnd, &mut owner) };
        if owner == find.pid && unsafe { IsWindowVisible(hwnd) } != 0 {
            find.found = hwnd;
            return 0;
        }
        TRUE
    }
    let mut find = Find { pid, found: std::ptr::null_mut() };
    // SAFETY: the callback only reads its window and writes `find`.
    unsafe { EnumWindows(Some(each), &mut find as *mut Find as LPARAM) };
    if find.found.is_null() {
        return;
    }
    let mut answer = 0usize;
    // SAFETY: a message to a window handle; the timeout bounds the wait.
    unsafe { SendMessageTimeoutW(find.found, WM_QUERYENDSESSION, 0, 0, SMTO_ABORTIFHUNG, SAVE_WAIT_MS, &mut answer) };
    tracing::info!(pid, "background work: told another AgentMux to save its windows");
}

/// Off Windows nothing is sent: stopped with its pane, another AgentMux keeps
/// its windows for its next start.
#[cfg(not(windows))]
fn save_windows_of(_pid: u32) {}

// ── After the next start ────────────────────────────────────────────────────

struct State {
    dir: PathBuf,
    record: Mutex<Record>,
    /// Agents a note is on its way to.
    telling: Mutex<HashSet<String>>,
}

static STATE: OnceLock<Option<State>> = OnceLock::new();

/// At srv start: what the last quit stopped, if it was recent.
pub fn init() {
    let state = channel_dir().and_then(|dir| {
        let record = read(&dir)?;
        let fresh = agentmux_common::time::now_ms() - record.at_ms < TELL_FOR.as_millis() as i64;
        if !fresh {
            write(&dir, &Record::default());
            return None;
        }
        tracing::info!(items = record.items.len(), "background work: agents will be told what the last quit stopped");
        Some(State { dir, record: Mutex::new(record), telling: Mutex::new(HashSet::new()) })
    });
    let _ = STATE.set(state);
}

/// The note for `items`, all of one agent's, stopped by the quit at `at_ms`.
pub fn note_text(items: &[Work], at_ms: i64) -> String {
    let at = chrono::DateTime::from_timestamp_millis(at_ms)
        .map(|t| t.with_timezone(&chrono::Local).format(" at %H:%M").to_string())
        .unwrap_or_default();
    let list: Vec<String> = items
        .iter()
        .map(|w| {
            let started = chrono::DateTime::from_timestamp_millis(w.started_at_ms as i64)
                .filter(|_| w.started_at_ms > 0)
                .map(|t| t.with_timezone(&chrono::Local).format(", started %H:%M").to_string())
                .unwrap_or_default();
            if w.agentmux {
                format!("an AgentMux instance (`{}`{started})", w.label)
            } else {
                format!("`{}`{started}", w.label)
            }
        })
        .collect();
    format!(
        "[AgentMux] When AgentMux quit{at}, it stopped what you had running in the background: {}. Start it again if you still need it.",
        list.join("; ")
    )
}

/// An agent registered in `block_id`: if the last quit stopped something of
/// its own, tell it once. Called under the handler lock, so the agent is
/// looked up, and told, from a task of its own.
pub fn on_registered(block_id: &str) {
    let Some(Some(state)) = STATE.get() else { return };
    {
        let record = state.record.lock().unwrap_or_else(|e| e.into_inner());
        // Told only within the hour after the quit, however long srv has run.
        if record.items.is_empty() || agentmux_common::time::now_ms() - record.at_ms >= TELL_FOR.as_millis() as i64 {
            return;
        }
    }
    let block_id = block_id.to_string();
    tokio::spawn(async move {
        let Some(Some(state)) = STATE.get() else { return };
        let handler = crate::backend::reactive::get_global_handler();
        let Some(key) = handler
            .list_agents()
            .into_iter()
            .find(|r| r.block_id == block_id)
            .map(|r| r.uid.unwrap_or(r.agent_id))
        else {
            return;
        };
        let (items, at_ms) = {
            let record = state.record.lock().unwrap_or_else(|e| e.into_inner());
            (record.items.iter().filter(|w| w.key == key).cloned().collect::<Vec<_>>(), record.at_ms)
        };
        if items.is_empty() || !state.telling.lock().unwrap_or_else(|e| e.into_inner()).insert(key.clone()) {
            return;
        }
        let text = note_text(&items, at_ms);
        use crate::backend::upgrade_notice::{deliver_with_retry, Undelivered, RETRIES, RETRY_GAP};
        let delivered = deliver_with_retry(
            || {
                let target = handler
                    .list_agents()
                    .into_iter()
                    .find(|r| r.block_id == block_id)
                    .map(|r| r.uid.unwrap_or(r.agent_id))
                    .ok_or_else(|| Undelivered::Gone("not registered any more".into()))?;
                handler.deliver_system_note(&target, &text).map_err(Undelivered::Retry)
            },
            RETRIES,
            RETRY_GAP,
        )
        .await;
        match delivered {
            Ok(()) => {
                tracing::info!(agent = %key, "background work: told the agent what the quit stopped");
                let mut record = state.record.lock().unwrap_or_else(|e| e.into_inner());
                record.items.retain(|w| w.key != key);
                write(&state.dir, &record);
            }
            Err(e) => {
                tracing::info!(agent = %key, error = %e, "background work: note not delivered; the next registration tries again");
                state.telling.lock().unwrap_or_else(|e| e.into_inner()).remove(&key);
            }
        }
    });
}

#[cfg(test)]
#[path = "background_work_tests.rs"]
mod tests;
