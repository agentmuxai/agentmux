// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One screen-mode session: a virtual X screen, a window manager (when the server has one) and an ordinary Chrome on it, started the way a
//! person starts Chrome: no debugging pipe or port, no automation or headless switch, no script put into a page. The only extra
//! arguments are a window size and the page to open (the network is the browser's managed policy). The browser's own sandbox is always on: no switch turns it off,
//! and a server asked to start without it refuses.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use super::super::chromium;

/// The virtual screen's colour depth.
const DEPTH: u32 = 24;
/// How long a new first session waits for the browser of the pane before it to finish going away.
const LEFTOVER_WAIT: Duration = Duration::from_secs(5);

/// What one session keeps alive: the processes end when it is dropped.
pub struct Session {
    pub display: String,
    _xvfb: Child,
    _wm: Option<Child>,
    _chrome: Child,
    /// The browser's process group: it and all its helpers (its crash handler, zygote, renderers) are ended together.
    chrome_group: Option<u32>,
    _profile: tempfile::TempDir,
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(group) = self.chrome_group {
            kill_group(group);
        }
    }
}

/// End every process in the process group `group` (the browser and what it started). Nothing is left running that the next session's
/// "no other browser in this container" check would find.
#[cfg(unix)]
fn kill_group(group: u32) {
    // SAFETY: `killpg` takes a group number and a signal and has no other effect; a group that is already gone answers an error that is ignored.
    unsafe {
        libc::killpg(group as libc::pid_t, libc::SIGKILL);
    }
}

#[cfg(not(unix))]
fn kill_group(_group: u32) {}

/// The arguments of the virtual screen: a size, no network listener, and the display number written to standard output.
pub fn xvfb_args(w: u32, h: u32) -> Vec<String> {
    vec!["-displayfd".into(), "1".into(), "-screen".into(), "0".into(), format!("{w}x{h}x{DEPTH}"), "-nolisten".into(), "tcp".into(), "-noreset".into()]
}

/// The arguments of the browser: a profile of its own, the address proxy, a window the size of the screen, and the page to open.
pub fn chrome_args(profile: &Path, w: u32, h: u32, url: &str, has_wm: bool) -> Vec<String> {
    let mut a = vec![
        format!("--user-data-dir={}", profile.display()),
        "--no-first-run".to_string(),
        "--no-default-browser-check".to_string(),
        "--force-webrtc-ip-handling-policy=disable_non_proxied_udp".to_string(),
        "--dns-prefetch-disable".to_string(),
        "--window-position=0,0".to_string(),
        format!("--window-size={w},{h}"),
    ];
    if has_wm {
        a.push("--start-maximized".to_string());
    }
    a.push(url.to_string());
    a
}

/// The browser's own sandbox is part of what keeps a page away from the container, so screen mode never starts without it: the switch the
/// streamed pane has for a test host (`NO_SANDBOX_ENV`) is refused here, in words, and `chrome_args` has no way to add `--no-sandbox`.
pub fn sandbox_on(asked_off: Option<&str>) -> Result<(), String> {
    match asked_off {
        Some("1") => Err(format!(
            "screen mode does not start without the browser's own sandbox ({} is set): run it in a container that lets the browser's sandbox start, or unset that setting",
            chromium::NO_SANDBOX_ENV
        )),
        _ => Ok(()),
    }
}

/// A process that is stopped when its handle is dropped, with the environment it needs and nothing else of srv's.
fn command(program: &Path, args: &[String], display: Option<&str>) -> Command {
    let mut c = Command::new(program);
    c.args(args).stdin(Stdio::null()).stderr(Stdio::null()).kill_on_drop(true);
    c.env_clear().envs(chromium::allowed_env(std::env::vars()));
    if let Some(d) = display {
        c.env("DISPLAY", d);
    }
    c
}

/// Start the screen, the window manager and the browser, and say which display they are on.
pub async fn start(w: u32, h: u32, url: &str, first: bool) -> Result<Session, String> {
    sandbox_on(std::env::var(chromium::NO_SANDBOX_ENV).ok().as_deref())?;
    let xvfb_bin = which::which("Xvfb").map_err(|_| "this server has no Xvfb to make a virtual screen with".to_string())?;
    let chrome_bin = chromium::find_binary()?;
    // The one way out for the browser, as for every pane: if the proxy cannot start, nothing runs.
    let proxy_port = super::super::address_proxy::start().await.map_err(|e| format!("the browser's address proxy did not start: {e}"))?;
    // The browser of the pane that just ended may still be going away (a page that is reloaded closes the old pane and opens the new one at
    // once): wait for it to be gone before the check that this container holds one browser.
    if first {
        let until = std::time::Instant::now() + LEFTOVER_WAIT;
        while super::policy::browser_running() && std::time::Instant::now() < until {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    // What the browser may open and the network it has (the proxy's port) are its managed policy: nothing starts without it.
    super::policy::ensure(&chrome_bin, first, proxy_port)?;

    let mut xvfb = command(&xvfb_bin, &xvfb_args(w, h), None).stdout(Stdio::piped()).spawn().map_err(|e| format!("cannot start Xvfb: {e}"))?;
    let out = xvfb.stdout.take().ok_or("Xvfb has no output")?;
    let mut line = String::new();
    tokio::time::timeout(Duration::from_secs(15), BufReader::new(out).read_line(&mut line))
        .await
        .map_err(|_| "the virtual screen did not say which display it is".to_string())?
        .map_err(|e| format!("the virtual screen's display could not be read: {e}"))?;
    let number: u32 = line.trim().parse().map_err(|_| format!("the virtual screen said {:?}, not a display number", line.trim()))?;
    let display = format!(":{number}");

    let profile = tempfile::Builder::new().prefix("agentmux-screen-browser-").tempdir().map_err(|e| format!("cannot make the browser's profile folder: {e}"))?;
    // The window manager runs with settings that bind nothing (wm.rs): a client's keys and mouse reach the browser and nothing else.
    let wm = match which::which("openbox") {
        Ok(bin) => command(&bin, &super::wm::write(&profile.path().join("wm"))?, Some(&display)).spawn().ok(),
        Err(_) => None,
    };
    let args = chrome_args(profile.path(), w, h, url, wm.is_some());
    let mut chrome = command(&chrome_bin, &args, Some(&display));
    // The browser leads a process group of its own, so that it and everything it starts can be ended together.
    #[cfg(unix)]
    chrome.process_group(0);
    let chrome = chrome.spawn().map_err(|e| format!("cannot start {}: {e}", chrome_bin.display()))?;
    let chrome_group = chrome.id();
    Ok(Session { display, _xvfb: xvfb, _wm: wm, _chrome: chrome, chrome_group, _profile: profile })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_browser_is_started_like_a_persons_browser() {
        let a = chrome_args(Path::new("/tmp/p"), 1024, 640, "https://a.example/", true);
        for banned in ["--remote-debugging", "--enable-automation", "--headless", "--disable-blink-features", "--enable-blink-features", "--user-agent", "--load-extension", "--disable-web-security"] {
            assert!(!a.iter().any(|x| x.starts_with(banned)), "{banned} is not allowed: {a:?}");
        }
        assert!(a.contains(&"--user-data-dir=/tmp/p".to_string()));
        assert!(a.contains(&"--no-first-run".to_string()) && a.contains(&"--no-default-browser-check".to_string()));
        assert_eq!(a.last().map(String::as_str), Some("https://a.example/"));
        assert!(!a.contains(&"--no-sandbox".to_string()));
    }

    #[test]
    fn the_browser_gets_its_network_from_policy_and_no_switch_that_chrome_calls_unsupported() {
        let a = chrome_args(Path::new("/tmp/p"), 800, 600, "about:blank", false);
        // The network is the container's policy (policy.rs), not switches: no switch that Chrome calls unsupported, no way around the proxy.
        assert!(!a.iter().any(|x| x.starts_with("--proxy-") || x == "--no-proxy-server" || x.starts_with("--host-resolver-rules") || x == "--disable-quic"), "{a:?}");
        let policy = super::super::policy::wanted(41234);
        assert_eq!(policy["ProxyServer"], "socks5://127.0.0.1:41234");
        assert_eq!(policy["ProxyMode"], "fixed_servers");
        assert_eq!(policy["QuicAllowed"], false);
    }

    #[test]
    fn the_window_is_the_screen() {
        let a = chrome_args(Path::new("/tmp/p"), 1280, 720, "about:blank", false);
        assert!(a.contains(&"--window-size=1280,720".to_string()) && a.contains(&"--window-position=0,0".to_string()));
        assert!(!a.contains(&"--start-maximized".to_string()));
        assert!(chrome_args(Path::new("/tmp/p"), 1280, 720, "about:blank", true).contains(&"--start-maximized".to_string()));
    }

    #[test]
    fn a_start_with_the_sandbox_off_is_refused_in_words_and_no_argument_turns_it_off() {
        assert!(sandbox_on(None).is_ok() && sandbox_on(Some("0")).is_ok() && sandbox_on(Some("")).is_ok());
        let words = sandbox_on(Some("1")).unwrap_err();
        assert!(words.contains("sandbox") && words.contains(chromium::NO_SANDBOX_ENV), "{words}");
        for has_wm in [false, true] {
            let a = chrome_args(Path::new("/tmp/p"), 800, 600, "about:blank", has_wm);
            assert!(!a.iter().any(|x| x.contains("sandbox")), "{a:?}");
        }
    }

    /// A browser that starts helpers leaves none of them behind: ending its group ends them all.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn ending_the_group_ends_what_the_browser_started_too() {
        use tokio::io::{AsyncBufReadExt, BufReader};
        // A "browser" (the shell) that starts a helper (a long sleep) and prints the helper's number. Started the way the browser is.
        let args = vec!["-c".to_string(), "/bin/sleep 60 & echo $!; wait".to_string()];
        let mut leader = command(Path::new("/bin/sh"), &args, None);
        leader.stdout(Stdio::piped()).process_group(0);
        let mut leader = leader.spawn().expect("sh starts");
        let mut line = String::new();
        BufReader::new(leader.stdout.take().unwrap()).read_line(&mut line).await.unwrap();
        let helper: u32 = line.trim().parse().expect("the helper's number");
        // A process that has ended but is not yet collected (a zombie) is gone for this purpose.
        let alive = |pid: u32| {
            std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| !s.rsplit(')').next().unwrap_or("").trim_start().starts_with('Z'))
        };
        assert!(alive(helper));
        kill_group(leader.id().expect("the shell is running"));
        leader.wait().await.unwrap();
        let until = std::time::Instant::now() + Duration::from_secs(3);
        while alive(helper) && std::time::Instant::now() < until {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(!alive(helper), "the helper outlived its group");
    }

    #[test]
    fn the_virtual_screen_has_the_size_asked_for_and_no_network_listener() {
        let a = xvfb_args(1024, 640);
        assert!(a.windows(2).any(|p| p == ["-screen", "0"]) && a.contains(&"1024x640x24".to_string()));
        assert!(a.windows(2).any(|p| p == ["-nolisten", "tcp"]));
        assert!(a.windows(2).any(|p| p == ["-displayfd", "1"]));
    }
}
