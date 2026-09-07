// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Identity shown in the splash footer: `user@host` and `v<version>` (+ a dev
//! label on non-stable builds). Gathered once in the launcher and handed to each
//! platform's splash backend, so the three render identical content.
//!
//! All sourcing is dependency-light and best-effort — a missing field falls back
//! to a placeholder and never blocks or crashes the splash.

use std::process::Command;

pub struct SplashInfo {
    pub user: String,
    pub host: String,
    pub version: String,
    /// `Some` only on non-stable builds (dev / explicit channel); `None` on a
    /// stable release (footer then shows just the version).
    pub dev_label: Option<String>,
}

impl SplashInfo {
    pub fn gather() -> Self {
        SplashInfo {
            user: username(),
            host: hostname(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            dev_label: dev_label(),
        }
    }

    /// The footer lines to render (each clamped to `max_chars`):
    /// line 1 = `user@host`, line 2 = `v<version>` (+ ` DEV` on dev builds).
    pub fn footer_lines(&self, max_chars: usize) -> Vec<String> {
        let l1 = ellipsize(&format!("{}@{}", self.user, self.host), max_chars);
        let l2 = match &self.dev_label {
            Some(d) => format!("v{} {}", self.version, d.to_uppercase()),
            None => format!("v{}", self.version),
        };
        vec![l1, ellipsize(&l2, max_chars)]
    }
}

fn username() -> String {
    for k in ["USER", "USERNAME", "LOGNAME"] {
        if let Ok(v) = std::env::var(k) {
            let v = sanitize(v.trim());
            if !v.is_empty() {
                return v;
            }
        }
    }
    cmd_first_line("whoami", &[])
        .map(|s| sanitize(s.trim()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "user".into())
}

fn hostname() -> String {
    // Prefer env (no subprocess); COMPUTERNAME on Windows, HOSTNAME elsewhere.
    for k in ["HOSTNAME", "COMPUTERNAME", "HOST"] {
        if let Ok(v) = std::env::var(k) {
            let v = short_host(v.trim());
            if !v.is_empty() {
                return v;
            }
        }
    }
    cmd_first_line("hostname", &[])
        .map(|s| short_host(s.trim()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "host".into())
}

/// `None` on a stable release; otherwise a short badge.
/// - `AGENTMUX_DEV=1` (set by `task dev`) → `dev`.
/// - explicit `AGENTMUX_CHANNEL` other than `stable` → that channel.
/// (Baked per-build local channels without an env override are treated as stable
/// for the footer in v1 — see SPEC §3 "dev_label".)
fn dev_label() -> Option<String> {
    if std::env::var("AGENTMUX_DEV")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        return Some("dev".into());
    }
    if let Ok(ch) = std::env::var("AGENTMUX_CHANNEL") {
        let ch = sanitize(ch.trim());
        if !ch.is_empty() && ch != "stable" {
            return Some(ch);
        }
    }
    None
}

/// First non-empty stdout line of `cmd args`, trimmed. `None` on any failure.
fn cmd_first_line(cmd: &str, args: &[&str]) -> Option<String> {
    let mut command = Command::new(cmd);
    command.args(args);
    #[cfg(windows)]
    {
        // CREATE_NO_WINDOW: console-flash suppression — std::process::Command
        // needs the CommandExt trait to call creation_flags.
        use std::os::windows::process::CommandExt;
        use agentmux_common::win32::CREATE_NO_WINDOW;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let out = command.output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .find(|l| !l.is_empty())
}

/// Keep printable ASCII (0x20..0x7E); replace anything else with '?'. The splash
/// font only carries ASCII, so this keeps the footer legible for odd names.
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if (' '..='~').contains(&c) { c } else { '?' })
        .collect()
}

/// Short hostname: first dot-segment, sanitized (`devbox.local` → `devbox`).
fn short_host(s: &str) -> String {
    sanitize(s.split('.').next().unwrap_or(s))
}

/// Middle-ellipsize `s` to at most `max` chars so the footer never widens the card.
fn ellipsize(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_string();
    }
    if max <= 3 {
        return chars.iter().take(max).collect();
    }
    let keep = max - 3;
    let left = keep.div_ceil(2);
    let right = keep - left;
    let head: String = chars[..left].iter().collect();
    let tail: String = chars[chars.len() - right..].iter().collect();
    format!("{head}...{tail}")
}
