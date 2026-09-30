// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Settings-UI bridge to the launcher's auto-start status —
// docs/specs/SPEC_OS_NOTIFICATIONS_SYSTEM_2026_09_24.md §4.1.
//
// Read-only. Turning start-at-login on or off is the `app:startatlogin`
// setting, which the launcher applies to the OS login entry
// (docs/specs/SPEC_START_WITH_OS_2026_09_25.md §3.9); this only reports
// whether an entry is registered, so Settings can show when that failed.
//
// The launcher owns auto-start registration (`agentmux-launcher/src/autostart`:
// Scheduled Task / LaunchAgent / XDG autostart). Rather than duplicate that
// platform code in the host, we invoke the launcher binary itself with its
// existing, already-tested verbs. The launcher handles them before any
// single-instance or startup work, so running it alongside the live instance
// is safe. Its path comes from `AGENTMUX_LAUNCHER_EXE`, which the launcher
// stamps on the host's env at spawn.

use std::process::Command;

const ENV_LAUNCHER_EXE: &str = "AGENTMUX_LAUNCHER_EXE";

/// Map launcher stdout for `--autostart-status` to a bool.
pub fn parse_status(stdout: &str) -> Option<bool> {
    match stdout.trim() {
        "enabled" => Some(true),
        "disabled" => Some(false),
        _ => None,
    }
}

fn run_launcher(flag: &'static str) -> Result<String, String> {
    let exe = std::env::var_os(ENV_LAUNCHER_EXE)
        .ok_or_else(|| "auto-start unavailable: launcher path unknown".to_string())?;
    let mut cmd = Command::new(exe);
    cmd.arg(flag);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd.output().map_err(|e| format!("auto-start: failed to run launcher: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!("auto-start: {}", stderr.trim()));
    }
    Ok(stdout)
}

/// `autostart_status` → `{ available, enabled }`. Never errors: an unknown
/// launcher path reports `available: false` so the UI can disable the row.
pub async fn autostart_status() -> Result<serde_json::Value, String> {
    let res = tokio::task::spawn_blocking(|| run_launcher("--autostart-status"))
        .await
        .map_err(|e| e.to_string())?;
    Ok(match res.ok().as_deref().and_then(parse_status) {
        Some(enabled) => serde_json::json!({ "available": true, "enabled": enabled }),
        None => serde_json::json!({ "available": false, "enabled": false }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_launcher_status_output() {
        assert_eq!(parse_status("enabled\n"), Some(true));
        assert_eq!(parse_status("disabled\r\n"), Some(false));
        assert_eq!(parse_status(""), None);
        assert_eq!(parse_status("auto-start enabled (C:\\x)"), None);
    }
}
