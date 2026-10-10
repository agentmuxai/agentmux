// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tower on another machine (SPEC_TOWER_TASK_MANAGER_PANE_2026_10_08.md
//! §8.2): the host's processes, sampled there by AgentMux's helper
//! (`agentmux_remote::procs`) and sent as one small frame per request.
//!
//! - **SSH:** the helper connection file browsing already uses
//!   (`remote::files`): one `serve --stdio` per host, its install consent and
//!   idle close unchanged.
//! - **WSL:** the Linux helper this package ships, run inside the
//!   distribution straight from where it is installed (`wsl.exe --cd <its
//!   folder> --exec ./agentmux-remote serve --stdio`): nothing is copied in,
//!   no network.
//!
//! Frames carry names, never command lines. A pane asks every couple of
//! seconds while it is visible; two asking at once share a frame, so the
//! CPU window isn't halved.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use agentmux_remote::fsproto::ProcFrame;

use crate::backend::remote::files::RemoteFiles;
use crate::backend::remote::sessions::AskIn;
use crate::backend::remote::ConnTarget;
use crate::backend::rpc_types::{TowerHost, TowerProcess, TowerSnapshot};

/// Rows per list (busiest, largest) a frame carries.
pub const TOP: u32 = 100;

/// A second request within this of the last shares its frame.
const REUSE_WITHIN: Duration = Duration::from_millis(900);

/// A WSL helper nobody asked for this long is stopped.
const WSL_IDLE_CLOSE: Duration = Duration::from_secs(120);

/// Whether `conn` names this computer (no connection, `local`).
pub fn is_local(conn: &str) -> bool {
    matches!(ConnTarget::parse(conn), Ok(ConnTarget::Local)) || conn.trim().is_empty()
}

/// A sample of `conn`'s processes. `ask`: where ssh's prompts go (the pane
/// that asked).
pub async fn sample(conn: &str, filter: &str, ask: Option<AskIn<'_>>) -> Result<TowerSnapshot, String> {
    // One slot per (machine, filter), held across the helper call: a second
    // request arriving meanwhile waits for this frame and shares it instead of
    // asking the helper again (whose CPU window would then be near zero).
    let slot = slot((conn.to_string(), filter.to_string()));
    let mut slot = slot.lock().await;
    if let Some((at, snap)) = slot.as_ref() {
        if at.elapsed() < REUSE_WITHIN {
            return Ok(snap.clone());
        }
    }
    let helper = match ConnTarget::parse(conn)? {
        ConnTarget::Ssh(_) => crate::backend::remote::files::connect(conn, ask).await.map_err(|e| e.message)?,
        ConnTarget::Wsl(distro) => wsl_helper(&distro).await?,
        ConnTarget::Local => return Err("tower: this computer is sampled locally".to_string()),
    };
    let frame = helper.procs(TOP, filter).await.map_err(|e| e.message)?;
    let snap = to_snapshot(conn, &helper.os, frame);
    *slot = Some((Instant::now(), snap.clone()));
    Ok(snap)
}

type Slot = Arc<tokio::sync::Mutex<Option<(Instant, TowerSnapshot)>>>;

/// The slot for `key`, made on first use. Slots whose frame has gone stale
/// and that nobody holds are dropped as others are made, so the map stays
/// as small as the set of machines being watched.
fn slot(key: (String, String)) -> Slot {
    static SLOTS: OnceLock<Mutex<HashMap<(String, String), Slot>>> = OnceLock::new();
    let mut slots = SLOTS.get_or_init(Default::default).lock().unwrap();
    slots.retain(|k, s| {
        *k == key
            || Arc::strong_count(s) > 1
            || s.try_lock().map_or(true, |g| g.as_ref().is_some_and(|(at, _)| at.elapsed() < STALE_SLOT))
    });
    slots.entry(key).or_default().clone()
}

/// A slot unused this long is forgotten.
const STALE_SLOT: Duration = Duration::from_secs(60);

/// A helper's frame as the pane's snapshot: a Host view, no tasks (a remote
/// machine has no AgentMux panes to group by).
pub fn to_snapshot(conn: &str, os: &str, f: ProcFrame) -> TowerSnapshot {
    let rows: Vec<TowerProcess> = f
        .rows
        .into_iter()
        .map(|r| TowerProcess {
            id: format!("{}:{}", r.pid, r.start_key),
            pid: r.pid,
            ppid: r.ppid,
            name: r.name,
            started_at_ms: r.started_at_ms,
            cpu: r.cpu_milli.map(|m| m as f64 / 1000.0),
            mem: r.mem,
            mem_resident: r.mem_resident,
            mem_commit: None,
            role: None,
            task: None,
            detail: None,
        })
        .collect();
    TowerSnapshot {
        ts_ms: agentmux_common::time::now_ms() as u64,
        hostname: conn.to_string(),
        os: os.to_string(),
        cpu_count: f.cpu_count,
        memory_metric: f.memory_metric,
        interval_ms: crate::backend::tower_sampler::INTERVAL.as_millis() as u32,
        remote: true,
        tasks: Vec::new(),
        // The helper's frame already carries the machine's totals in `host`.
        machine: None,
        host: Some(TowerHost {
            processes: rows,
            unmeasured: f.unmeasured,
            cpu: f.cpu_milli.map(|m| m as f64 / 1000.0),
            mem: f.mem,
            total: f.total,
            matched: f.matched,
        }),
    }
}

// ── WSL ────────────────────────────────────────────────────────────────────

struct WslHelper {
    files: Arc<RemoteFiles>,
    last_used: Mutex<Instant>,
}

fn wsl_pool() -> &'static tokio::sync::Mutex<HashMap<String, Arc<WslHelper>>> {
    static POOL: OnceLock<tokio::sync::Mutex<HashMap<String, Arc<WslHelper>>>> = OnceLock::new();
    POOL.get_or_init(|| {
        // Stop the helpers nobody is looking at (once, for the whole pool).
        tokio::spawn(async {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                wsl_pool()
                    .lock()
                    .await
                    .retain(|_, h| h.files.is_open() && h.last_used.lock().unwrap().elapsed() < WSL_IDLE_CLOSE);
            }
        });
        Default::default()
    })
}

/// The Linux helper target for WSL on this machine's architecture.
fn wsl_target() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "aarch64-unknown-linux-musl"
    } else {
        "x86_64-unknown-linux-musl"
    }
}

async fn wsl_helper(distro: &str) -> Result<Arc<RemoteFiles>, String> {
    let mut pool = wsl_pool().lock().await;
    if let Some(h) = pool.get(distro).filter(|h| h.files.is_open()) {
        *h.last_used.lock().unwrap() = Instant::now();
        return Ok(h.files.clone());
    }
    let files = start_wsl(distro).await?;
    pool.insert(distro.to_string(), Arc::new(WslHelper { files: files.clone(), last_used: Mutex::new(Instant::now()) }));
    Ok(files)
}

#[cfg(windows)]
async fn start_wsl(distro: &str) -> Result<Arc<RemoteFiles>, String> {
    use agentmux_common::win32::NoWindow;
    let helper = crate::backend::remote::helper_install::local_path(env!("CARGO_PKG_VERSION"), wsl_target())?;
    let dir = helper.parent().ok_or("the helper has no folder")?;
    // The file found: the packaged `agentmux-remote`, or a developer's flat
    // `agentmux-remote-<version>-<target>` (`AGENTMUX_REMOTE_HELPER_DIR`).
    let file = helper.file_name().ok_or("the helper has no name")?.to_string_lossy().into_owned();
    let mut cmd = tokio::process::Command::new("wsl.exe");
    // Not ours to configure, and it needs nothing of AgentMux's environment.
    crate::backend::pane_env::sanitize_external_command(&mut cmd);
    cmd.arg("-d")
        .arg(distro)
        .arg("--cd")
        .arg(dir)
        .arg("--exec")
        .arg(format!("./{file}"))
        .args(["serve", "--stdio"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .no_window();
    let mut child = cmd.spawn().map_err(|e| format!("couldn't start WSL ({distro}): {e}"))?;
    let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        return Err("WSL started without its pipes".to_string());
    };
    let conn = format!("wsl://{distro}");
    RemoteFiles::over(&conn, stdout, stdin, Box::new(child)).await.map_err(|e| e.message)
}

#[cfg(not(windows))]
async fn start_wsl(distro: &str) -> Result<Arc<RemoteFiles>, String> {
    let _ = wsl_target();
    Err(format!("WSL ({distro}) is only on Windows"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentmux_remote::fsproto::ProcRow;

    #[test]
    fn a_frame_becomes_a_host_only_snapshot() {
        let snap = to_snapshot(
            "build-box",
            "linux",
            ProcFrame {
                cpu_count: 16,
                memory_metric: "anonymous resident + swap".into(),
                total: 410,
                matched: 410,
                unmeasured: 0,
                cpu_milli: Some(2500),
                mem: 8 << 30,
                rows: vec![ProcRow {
                    pid: 77,
                    ppid: Some(1),
                    start_key: 123,
                    started_at_ms: Some(1),
                    name: "cargo".into(),
                    cpu_milli: Some(1250),
                    mem: Some(1 << 30),
                    mem_resident: None,
                }],
            },
        );
        assert!(snap.remote && snap.tasks.is_empty());
        assert_eq!((snap.hostname.as_str(), snap.os.as_str(), snap.cpu_count), ("build-box", "linux", 16));
        let host = snap.host.unwrap();
        assert_eq!((host.total, host.processes.len()), (410, 1));
        assert!((host.cpu.unwrap() - 2.5).abs() < 1e-9);
        let p = &host.processes[0];
        assert_eq!(p.id, "77:123");
        assert_eq!(p.cpu, Some(1.25));
    }

    #[test]
    fn this_computer_is_local() {
        assert!(is_local(""));
        assert!(is_local("local"));
        assert!(!is_local("user@build-box"));
        assert!(!is_local("wsl://Ubuntu"));
    }

    /// A real WSL distribution through the Linux helper, run in place. Opt-in:
    /// set `AGENTMUX_TEST_WSL_DISTRO` to a distribution and
    /// `AGENTMUX_REMOTE_HELPER_DIR` to a folder holding
    /// `x86_64-unknown-linux-musl/agentmux-remote`.
    #[tokio::test]
    #[cfg(windows)]
    async fn a_wsl_distribution_is_sampled_through_the_bundled_helper() {
        let Ok(distro) = std::env::var("AGENTMUX_TEST_WSL_DISTRO") else { return };
        let conn = format!("wsl://{distro}");
        let first = sample(&conn, "", None).await.unwrap();
        let host = first.host.as_ref().unwrap();
        assert_eq!(first.os, "linux");
        assert!(host.total > 0 && !host.processes.is_empty(), "{first:?}");
        assert!(host.processes.iter().any(|p| p.pid == 1), "PID 1 (init) is listed");
        tokio::time::sleep(Duration::from_millis(1200)).await;
        let second = sample(&conn, "", None).await.unwrap();
        assert!(second.host.unwrap().processes.iter().any(|p| p.cpu.is_some()), "rates on the second frame");
    }

    /// The real path end to end, minus ssh: the helper's own request handling
    /// in this process, behind the pool `remote::files` hands out.
    #[tokio::test]
    async fn an_ssh_host_is_sampled_through_its_helper() {
        let home = tempfile::tempdir().unwrap();
        crate::backend::remote::files::connect_in_process("tower-test-host", home.path().to_path_buf()).await;
        let me = std::process::id().to_string();
        let snap = sample("tower-test-host", &me, None).await.unwrap();
        let host = snap.host.unwrap();
        assert!(host.processes.iter().any(|p| p.pid == std::process::id()), "{host:?}");
        assert!(host.total > host.matched || host.total == host.matched);
        // A second ask within the window is the same frame.
        let again = sample("tower-test-host", &me, None).await.unwrap();
        assert_eq!(again.ts_ms, snap.ts_ms);
    }

    /// Two panes asking at the same moment share one frame: the second waits
    /// for the first instead of asking the helper again.
    #[tokio::test]
    async fn concurrent_asks_share_one_frame() {
        let home = tempfile::tempdir().unwrap();
        crate::backend::remote::files::connect_in_process("tower-test-host-2", home.path().to_path_buf()).await;
        let (a, b) = tokio::join!(sample("tower-test-host-2", "x", None), sample("tower-test-host-2", "x", None));
        assert_eq!(a.unwrap().ts_ms, b.unwrap().ts_ms);
    }
}
