// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One screen-mode session: a virtual X screen, a window manager (when the server has one) and an ordinary Chrome on it, started the way a
//! person starts Chrome: no debugging pipe or port, no automation or headless switch, no script put into a page. The only extra
//! arguments are the address proxy's (the network policy every pane has), a window size and, for the demo only, the sandbox switch.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use super::super::chromium;

/// The virtual screen's colour depth.
const DEPTH: u32 = 24;

/// What one session keeps alive: the processes end when it is dropped.
pub struct Session {
    pub display: String,
    _xvfb: Child,
    _wm: Option<Child>,
    _chrome: Child,
    _profile: tempfile::TempDir,
}

/// The arguments of the virtual screen: a size, no network listener, and the display number written to standard output.
pub fn xvfb_args(w: u32, h: u32) -> Vec<String> {
    vec!["-displayfd".into(), "1".into(), "-screen".into(), "0".into(), format!("{w}x{h}x{DEPTH}"), "-nolisten".into(), "tcp".into(), "-noreset".into()]
}

/// The arguments of the browser: a profile of its own, the address proxy, a window the size of the screen, and the page to open.
pub fn chrome_args(profile: &Path, proxy_port: u16, w: u32, h: u32, url: &str, no_sandbox: bool, has_wm: bool) -> Vec<String> {
    let mut a = vec![
        format!("--user-data-dir={}", profile.display()),
        "--no-first-run".to_string(),
        "--no-default-browser-check".to_string(),
        format!("--proxy-server=socks5://127.0.0.1:{proxy_port}"),
        "--proxy-bypass-list=<-loopback>".to_string(),
        "--host-resolver-rules=MAP * ~NOTFOUND , EXCLUDE 127.0.0.1".to_string(),
        "--disable-quic".to_string(),
        "--force-webrtc-ip-handling-policy=disable_non_proxied_udp".to_string(),
        "--dns-prefetch-disable".to_string(),
        "--window-position=0,0".to_string(),
        format!("--window-size={w},{h}"),
    ];
    if has_wm {
        a.push("--start-maximized".to_string());
    }
    if no_sandbox {
        a.push("--no-sandbox".to_string());
    }
    a.push(url.to_string());
    a
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

/// The browser's command, with its policy for this one instance: in a task-scoped container the container's own policy folder is the instance's;
/// on a shared host the browser starts inside its own mount namespace, where only the policy folder is this instance's own.
fn browser_command(chrome_bin: &Path, args: &[String], profile: &Path, display: &str) -> Result<Command, String> {
    if super::policy::form() == super::policy::Form::Task {
        return Ok(command(chrome_bin, args, Some(display)));
    }
    let policy_src = profile.join("policy");
    super::policy::write_instance(&policy_src)?;
    let bwrap = which::which(super::confine::PROGRAM).map_err(|_| "screen mode needs bubblewrap (the `bwrap` program)".to_string())?;
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let mut all = super::confine::args(&policy_src, &super::policy::policy_dir(chrome_bin), home.as_deref());
    all.push(chrome_bin.display().to_string());
    all.extend(args.iter().cloned());
    Ok(command(&bwrap, &all, Some(display)))
}

/// Start the screen, the window manager and the browser, and say which display they are on.
pub async fn start(w: u32, h: u32, url: &str) -> Result<Session, String> {
    let xvfb_bin = which::which("Xvfb").map_err(|_| "this server has no Xvfb to make a virtual screen with".to_string())?;
    let chrome_bin = chromium::find_binary()?;
    // What the browser may open is its managed policy: nothing starts without it.
    super::policy::ensure(&chrome_bin)?;
    // The one way out for the browser, as for every pane: if the proxy cannot start, nothing runs.
    let proxy_port = super::super::address_proxy::start().await.map_err(|e| format!("the browser's address proxy did not start: {e}"))?;

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
    let no_sandbox = std::env::var(chromium::NO_SANDBOX_ENV).is_ok_and(|v| v == "1");
    let args = chrome_args(profile.path(), proxy_port, w, h, url, no_sandbox, wm.is_some());
    let chrome = browser_command(&chrome_bin, &args, profile.path(), &display)?
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", chrome_bin.display()))?;
    Ok(Session { display, _xvfb: xvfb, _wm: wm, _chrome: chrome, _profile: profile })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_browser_is_started_like_a_persons_browser() {
        let a = chrome_args(Path::new("/tmp/p"), 41234, 1024, 640, "https://a.example/", false, true);
        for banned in ["--remote-debugging", "--enable-automation", "--headless", "--disable-blink-features", "--enable-blink-features", "--user-agent", "--load-extension", "--disable-web-security"] {
            assert!(!a.iter().any(|x| x.starts_with(banned)), "{banned} is not allowed: {a:?}");
        }
        assert!(a.contains(&"--user-data-dir=/tmp/p".to_string()));
        assert!(a.contains(&"--no-first-run".to_string()) && a.contains(&"--no-default-browser-check".to_string()));
        assert_eq!(a.last().map(String::as_str), Some("https://a.example/"));
        assert!(!a.contains(&"--no-sandbox".to_string()));
    }

    #[test]
    fn the_browser_has_no_way_around_the_address_proxy() {
        let a = chrome_args(Path::new("/tmp/p"), 41234, 800, 600, "about:blank", false, false);
        assert!(a.contains(&"--proxy-server=socks5://127.0.0.1:41234".to_string()));
        assert!(a.contains(&"--proxy-bypass-list=<-loopback>".to_string()));
        assert!(a.contains(&"--host-resolver-rules=MAP * ~NOTFOUND , EXCLUDE 127.0.0.1".to_string()));
        assert!(a.contains(&"--disable-quic".to_string()));
        assert!(!a.iter().any(|x| x == "--no-proxy-server" || x.starts_with("--proxy-auto-detect") || x.starts_with("--proxy-pac-url")));
    }

    #[test]
    fn the_window_is_the_screen_and_the_demo_sandbox_switch_is_only_asked_for() {
        let a = chrome_args(Path::new("/tmp/p"), 1, 1280, 720, "about:blank", true, false);
        assert!(a.contains(&"--window-size=1280,720".to_string()) && a.contains(&"--window-position=0,0".to_string()));
        assert!(a.contains(&"--no-sandbox".to_string()));
        assert!(!a.contains(&"--start-maximized".to_string()));
        assert!(chrome_args(Path::new("/tmp/p"), 1, 1280, 720, "about:blank", false, true).contains(&"--start-maximized".to_string()));
    }

    #[test]
    fn the_virtual_screen_has_the_size_asked_for_and_no_network_listener() {
        let a = xvfb_args(1024, 640);
        assert!(a.windows(2).any(|p| p == ["-screen", "0"]) && a.contains(&"1024x640x24".to_string()));
        assert!(a.windows(2).any(|p| p == ["-nolisten", "tcp"]));
        assert!(a.windows(2).any(|p| p == ["-displayfd", "1"]));
    }
}
