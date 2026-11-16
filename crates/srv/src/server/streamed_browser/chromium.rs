// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The server-side Chromium: found, started once with a profile of its own and a CDP pipe, shared by
//! the panes, and stopped when the last pane has been gone a while.
//! docs/specs/SPEC_STREAMED_BROWSER_PANE_DEMO_2026_11_15.md §2 and §3.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use super::cdp::Cdp;

/// At most this many panes at once: a CPU budget, not a memory one (report finding 4).
pub const MAX_PANES: usize = 4;
/// How long Chromium is kept after its last pane closed.
const IDLE_STOP: Duration = Duration::from_secs(60);
/// `1` in the environment asks `--no-sandbox` (the demo only).
pub const NO_SANDBOX_ENV: &str = "AGENTMUX_STREAMED_BROWSER_NO_SANDBOX";
/// A path to the Chromium binary, when it is not on the PATH under a usual name.
pub const BINARY_ENV: &str = "AGENTMUX_STREAMED_BROWSER_BIN";
const USUAL_NAMES: &[&str] = &["chromium", "chromium-browser", "google-chrome", "google-chrome-stable", "chrome"];

/// The Chromium arguments: headless, its own profile, its pipe, and nothing it does not need.
pub fn args(profile: &Path, no_sandbox: bool) -> Vec<String> {
    let mut a = vec![
        "--headless=new".to_string(),
        "--remote-debugging-pipe".to_string(),
        format!("--user-data-dir={}", profile.display()),
        "--disable-gpu".to_string(),
        "--no-first-run".to_string(),
        "--no-default-browser-check".to_string(),
        "--disable-extensions".to_string(),
        "--disable-background-networking".to_string(),
        "--disable-sync".to_string(),
        "--disable-default-apps".to_string(),
        "--mute-audio".to_string(),
        "--hide-scrollbars".to_string(),
        "about:blank".to_string(),
    ];
    if no_sandbox {
        a.insert(1, "--no-sandbox".to_string());
    }
    a
}

/// The profile folder under a data directory: nothing else is ever a profile of this browser.
pub fn profile_under(data: &Path) -> PathBuf {
    data.join("streamed-browser").join("profile")
}

/// The profile folder: under srv's data directory, never a desktop profile.
fn profile_dir() -> Option<PathBuf> {
    // The directory srv itself keeps its databases in (`--wavedata` first), so two servers never share a profile.
    crate::headless::data_dir().map(profile_under)
}

fn find_binary() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os(BINARY_ENV).filter(|p| !p.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    USUAL_NAMES
        .iter()
        .find_map(|n| which::which(n).ok())
        .ok_or_else(|| "this server has no Chromium to run a page in".to_string())
}

/// How many panes are open, held to a limit.
#[derive(Default)]
pub struct Slots {
    open: usize,
}

impl Slots {
    pub fn take(&mut self, max: usize) -> bool {
        if self.open >= max {
            return false;
        }
        self.open += 1;
        true
    }
    /// Give one back; the count left.
    pub fn give(&mut self) -> usize {
        self.open = self.open.saturating_sub(1);
        self.open
    }
    pub fn is_empty(&self) -> bool {
        self.open == 0
    }
}

struct Pool {
    browser: Option<Arc<Cdp>>,
    slots: Slots,
    /// Counts every release, so an idle stop knows whether anything happened since it was set.
    epoch: u64,
}

fn pool() -> &'static Mutex<Pool> {
    static POOL: OnceLock<Mutex<Pool>> = OnceLock::new();
    POOL.get_or_init(|| Mutex::new(Pool { browser: None, slots: Slots::default(), epoch: 0 }))
}

/// A pane's hold on the browser; letting go of it frees its place.
pub struct Lease {
    pub cdp: Arc<Cdp>,
}

impl Drop for Lease {
    fn drop(&mut self) {
        let epoch = {
            let mut p = pool().lock().unwrap();
            p.epoch += 1;
            if p.slots.give() > 0 {
                return;
            }
            p.epoch
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else { return };
        runtime.spawn(async move {
            tokio::time::sleep(IDLE_STOP).await;
            // Holding the start lock, so a pane opened now waits until the old browser is gone before it starts one.
            let _starting = starting().lock().await;
            let idle = {
                let mut p = pool().lock().unwrap();
                if p.epoch == epoch && p.slots.is_empty() { p.browser.take() } else { None }
            };
            // Ask it to close; its pipe then ends and the client and the process are let go.
            if let Some(browser) = idle {
                let _ = browser.call("Browser.close", serde_json::json!({}), None).await;
                for _ in 0..50 {
                    if browser.is_dead() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        });
    }
}

/// Starting the browser is one at a time: two panes opened together must not start two Chromiums on one profile.
fn starting() -> &'static tokio::sync::Mutex<()> {
    static START: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    START.get_or_init(|| tokio::sync::Mutex::new(()))
}

fn running() -> Option<Arc<Cdp>> {
    pool().lock().unwrap().browser.clone().filter(|c| !c.is_dead())
}

/// A place in the shared browser, starting it when it is not running.
pub async fn lease() -> Result<Lease, String> {
    if !pool().lock().unwrap().slots.take(MAX_PANES) {
        return Err(format!("the server's browser is busy ({MAX_PANES} panes are open)"));
    }
    if let Some(cdp) = running() {
        return Ok(Lease { cdp });
    }
    let _starting = starting().lock().await;
    // A pane that waited here finds the browser the one before it started.
    if let Some(cdp) = running() {
        return Ok(Lease { cdp });
    }
    match start().await {
        Ok(cdp) => {
            pool().lock().unwrap().browser = Some(cdp.clone());
            Ok(Lease { cdp })
        }
        Err(e) => {
            pool().lock().unwrap().slots.give();
            Err(e)
        }
    }
}

#[cfg(unix)]
async fn start() -> Result<Arc<Cdp>, String> {
    let binary = find_binary()?;
    let profile = profile_dir().ok_or("this server has no data folder to keep the browser's profile in")?;
    std::fs::create_dir_all(&profile).map_err(|e| format!("cannot make the browser's profile folder: {e}"))?;
    let no_sandbox = std::env::var(NO_SANDBOX_ENV).is_ok_and(|v| v == "1");
    if no_sandbox {
        tracing::warn!("streamed browser: Chromium starts WITHOUT its sandbox ({NO_SANDBOX_ENV}=1); for the demo only");
    }
    let (cmd_read, cmd_write) = pipe_pair()?;
    let (resp_read, resp_write) = pipe_pair()?;
    let (cmd_read_fd, resp_write_fd) = (cmd_read.as_raw_fd(), resp_write.as_raw_fd());
    let mut command = tokio::process::Command::new(&binary);
    command.args(args(&profile, no_sandbox)).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).kill_on_drop(true);
    // The browser opens pages from the web: it gets none of srv's own environment (the key, the local address), as a
    // provider CLI started for a login gets none.
    crate::backend::pane_env::sanitize_external_command(&mut command);
    // SAFETY: only dup2 and close run between fork and exec, both async-signal-safe.
    unsafe {
        command.pre_exec(move || {
            if libc::dup2(cmd_read_fd, 3) < 0 || libc::dup2(resp_write_fd, 4) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = command.spawn().map_err(|e| format!("cannot start {}: {e}", binary.display()))?;
    drop((cmd_read, resp_write));
    let reader = tokio::fs::File::from_std(std::fs::File::from(resp_read));
    let writer = tokio::fs::File::from_std(std::fs::File::from(cmd_write));
    let cdp = Cdp::over(reader, writer);
    keep_child(child, cdp.clone());
    if let Err(e) = cdp.call("Browser.getVersion", serde_json::json!({}), None).await {
        // The process is ended within a second (`keep_child`), so a stuck start leaves nothing holding the profile.
        cdp.abandon();
        return Err(format!("the browser did not start: {e}"));
    }
    // Downloads are refused in the demo (spec §8); a later PR gives them a place.
    let _ = cdp.call("Browser.setDownloadBehavior", serde_json::json!({ "behavior": "deny" }), None).await;
    close_popups(&cdp).await;
    Ok(cdp)
}

/// A window a page opens (a target with an opener) is closed as it appears: the demo has no place for it.
#[cfg(unix)]
async fn close_popups(cdp: &Arc<Cdp>) {
    let mut events = cdp.browser_events();
    let _ = cdp.call("Target.setDiscoverTargets", serde_json::json!({ "discover": true }), None).await;
    let cdp = cdp.clone();
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            if event.method != "Target.targetCreated" {
                continue;
            }
            let info = &event.params["targetInfo"];
            if info["type"] == "page" && info.get("openerId").is_some_and(|o| !o.is_null()) {
                if let Some(id) = info["targetId"].as_str() {
                    cdp.send("Target.closeTarget", serde_json::json!({ "targetId": id }), None);
                }
            }
        }
    });
}

#[cfg(not(unix))]
async fn start() -> Result<Arc<Cdp>, String> {
    Err("the streamed browser runs on Linux and macOS servers".into())
}

#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
#[cfg(unix)]
use std::os::unix::process::CommandExt as _;

/// A pipe, both ends close-on-exec and above descriptor 4, so `dup2` onto 3 and 4 never lands on one.
#[cfg(unix)]
fn pipe_pair() -> Result<(OwnedFd, OwnedFd), String> {
    let mut fds = [0i32; 2];
    // SAFETY: `fds` is two ints, as pipe(2) wants.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(format!("cannot make a pipe: {}", std::io::Error::last_os_error()));
    }
    let lift = |fd: i32| -> Result<OwnedFd, String> {
        // SAFETY: fcntl on a descriptor this function owns; the original is closed after the copy.
        let moved = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 10) };
        unsafe { libc::close(fd) };
        if moved < 0 {
            return Err(format!("cannot move a pipe: {}", std::io::Error::last_os_error()));
        }
        Ok(unsafe { OwnedFd::from_raw_fd(moved) })
    };
    Ok((lift(fds[0])?, lift(fds[1])?))
}

/// Hold the process until the browser's pipe closes, then reap it.
#[cfg(unix)]
fn keep_child(mut child: tokio::process::Child, cdp: Arc<Cdp>) {
    tokio::spawn(async move {
        while !cdp.is_dead() {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        let _ = child.kill().await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chromium_gets_its_own_profile_a_pipe_and_no_sandbox_flag_unless_asked() {
        let a = args(Path::new("/data/streamed-browser/profile"), false);
        assert!(a.contains(&"--remote-debugging-pipe".to_string()));
        assert!(a.contains(&"--user-data-dir=/data/streamed-browser/profile".to_string()));
        assert!(a.contains(&"--headless=new".to_string()));
        assert!(!a.iter().any(|x| x.contains("remote-debugging-port") || x.contains("no-sandbox")));
        let demo = args(Path::new("/p"), true);
        assert!(demo.contains(&"--no-sandbox".to_string()));
        assert_eq!(demo.last().map(String::as_str), Some("about:blank"));
    }

    #[test]
    fn panes_are_limited_and_a_place_given_back_can_be_taken() {
        let mut s = Slots::default();
        for _ in 0..MAX_PANES {
            assert!(s.take(MAX_PANES));
        }
        assert!(!s.take(MAX_PANES));
        assert_eq!(s.give(), MAX_PANES - 1);
        assert!(s.take(MAX_PANES));
        for _ in 0..MAX_PANES + 2 {
            s.give();
        }
        assert!(s.is_empty());
    }

    #[test]
    fn the_profile_is_under_the_data_folder_and_nowhere_else() {
        // The path is built from the data folder only: no home directory, no desktop profile.
        assert_eq!(profile_under(Path::new("/srv/amx/data")), PathBuf::from("/srv/amx/data/streamed-browser/profile"));
    }
}
