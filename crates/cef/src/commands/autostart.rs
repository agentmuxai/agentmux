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
// The launcher owns auto-start registration (`crates/launcher/src/autostart`:
// Scheduled Task / LaunchAgent / XDG autostart). Rather than duplicate that
// platform code in the host, we invoke the launcher binary itself with its
// existing, already-tested verbs. The launcher handles them before any
// single-instance or startup work, so running it alongside the live instance
// is safe. Its path comes from `AGENTMUX_LAUNCHER_EXE`, which the launcher
// stamps on the host's env at spawn.

#[cfg(windows)]
use agentmux_common::win32::NoWindow;
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

fn run_launcher(args: &[&str]) -> Result<String, String> {
    let exe = std::env::var_os(ENV_LAUNCHER_EXE)
        .ok_or_else(|| "auto-start unavailable: launcher path unknown".to_string())?;
    let mut cmd = Command::new(exe);
    cmd.args(args);
    #[cfg(target_os = "windows")]
    {
        cmd.no_window();
    }
    let out = cmd.output().map_err(|e| format!("auto-start: failed to run launcher: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!("auto-start: {}", stderr.trim()));
    }
    Ok(stdout)
}

/// Map launcher stdout for `--autostart-status --json` (the launcher's `Status`) to the fields the UI reads, on top of
/// `{ available, enabled }`: `state`, `target`, `ownerChannel`, `thisChannel`, `movable`. `None` for output that is
/// not that object, such as an older launcher's plain `enabled`.
pub fn parse_status_json(stdout: &str) -> Option<serde_json::Value> {
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).ok()?;
    let state = v.get("state")?.as_str()?;
    if !matches!(state, "off" | "on" | "broken" | "needs_approval") {
        return None;
    }
    let text = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
    Some(serde_json::json!({
        "available": true,
        "enabled": state != "off",
        "state": state,
        "target": text("target"),
        "ownerChannel": text("owner_channel"),
        "thisChannel": text("this_channel"),
        "movable": v.get("movable").and_then(|x| x.as_bool()).unwrap_or(false),
        "package": text("package"),
    }))
}

/// `autostart_status` → `{ available, enabled }` and, from a launcher that reports it, `state`, `target`,
/// `ownerChannel`, `thisChannel` and `movable` (`SPEC_START_WITH_OS_2026_09_25.md` §3.8). Never errors: an unknown
/// launcher path reports `available: false` so the UI can disable the row.
pub async fn autostart_status() -> Result<serde_json::Value, String> {
    let res = tokio::task::spawn_blocking(|| match run_launcher(&["--autostart-status", "--json"]).ok().as_deref().and_then(parse_status_json) {
        Some(json) => Ok(json),
        // An older launcher answers the plain verb only; it ignores `--json` and prints `enabled` or `disabled`.
        None => run_launcher(&["--autostart-status"]).map(|out| match parse_status(&out) {
            Some(enabled) => serde_json::json!({ "available": true, "enabled": enabled }),
            None => serde_json::json!({ "available": false, "enabled": false }),
        }),
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(res.unwrap_or_else(|_| serde_json::json!({ "available": false, "enabled": false })))
}

/// The Settings-UI autostart commands: `autostart_status`, and `autostart_open_login_items` (macOS: open System Settings
/// at Login Items, where a login item that waits for the person's approval is allowed).
pub async fn route(cmd: &str) -> Result<serde_json::Value, String> {
    match cmd {
        "autostart_open_login_items" => open_login_items().await,
        _ => autostart_status().await,
    }
}

/// `autostart_open_login_items`: ask the launcher to open System Settings at Login Items (macOS), where a start-at-login
/// entry that needs the person's approval is allowed. Errors say why, for the Settings row to show.
pub async fn open_login_items() -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(|| run_launcher(&["--open-login-items"]).map(|_| serde_json::Value::Null))
        .await
        .map_err(|e| e.to_string())?
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

    #[test]
    fn parses_the_launchers_json_status() {
        let on = parse_status_json(r#"{"state":"on","target":"/a/AgentMux","owner_channel":"local-x","this_channel":"stable","movable":true}"#).unwrap();
        assert_eq!(
            on,
            serde_json::json!({ "available": true, "enabled": true, "state": "on", "target": "/a/AgentMux", "ownerChannel": "local-x", "thisChannel": "stable", "movable": true, "package": null })
        );
        // The Store package's startup task, as Windows says it: the person turned it off there.
        let pkg = parse_status_json(r#"{"state":"needs_approval","target":null,"owner_channel":null,"this_channel":"stable","movable":false,"package":"disabled_by_user","activation":"launch"}"#).unwrap();
        assert_eq!((pkg["state"].clone(), pkg["package"].clone()), ("needs_approval".into(), "disabled_by_user".into()));
        let off = parse_status_json(r#"{"state":"off","target":null,"owner_channel":null,"this_channel":"stable","movable":false}"#).unwrap();
        assert_eq!((off["enabled"].clone(), off["state"].clone(), off["ownerChannel"].clone()), (false.into(), "off".into(), serde_json::Value::Null));
        assert_eq!(parse_status_json(r#"{"state":"broken","this_channel":"s"}"#).unwrap()["enabled"], true, "broken still has an entry");
        assert_eq!(parse_status_json("enabled\n"), None, "an older launcher's plain answer");
        assert_eq!(parse_status_json(r#"{"state":"weird"}"#), None);
        assert_eq!(parse_status_json(""), None);
    }
}
