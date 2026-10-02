// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// XWayland authorization for the host process (#4011).
//
// Used when the host runs under XWayland on a Wayland session, i.e. when
// `AGENTMUX_OZONE_PLATFORM=x11` forces it. GNOME's Mutter starts Xwayland with
// `-auth $XDG_RUNTIME_DIR/.mutter-Xwaylandauth.*` and exports XAUTHORITY only
// into the systemd user environment, so a host started by a login entry, the
// AppImage binfmt path or an agent shell can lack it. Chromium then fails
// with "Missing X server or $DISPLAY" and CEF exits before any window opens.

use std::path::{Path, PathBuf};

/// Make sure XAUTHORITY points at a readable cookie, setting it from the
/// session when it's missing. Must run before CefInitialize so the GPU and
/// renderer processes inherit it.
pub(crate) fn ensure_xauthority() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("XAUTHORITY").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    let found = systemd_user_xauthority()
        .or_else(|| {
            std::env::var_os("XDG_RUNTIME_DIR")
                .and_then(|d| newest_mutter_cookie(Path::new(&d)))
        })
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|h| PathBuf::from(h).join(".Xauthority"))
                .filter(|p| p.is_file())
        })?;
    std::env::set_var("XAUTHORITY", &found);
    tracing::info!("XAUTHORITY was unset; using {}", found.display());
    Some(found)
}

fn systemd_user_xauthority() -> Option<PathBuf> {
    let out = std::process::Command::new("systemctl")
        .args(["--user", "show-environment"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_show_environment(&String::from_utf8_lossy(&out.stdout)).filter(|p| p.is_file())
}

/// `XAUTHORITY` from `systemctl --user show-environment` output.
fn parse_show_environment(out: &str) -> Option<PathBuf> {
    out.lines()
        .find_map(|l| l.strip_prefix("XAUTHORITY="))
        .map(|v| v.trim().trim_matches(|c| c == '\'' || c == '"'))
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// The most recently modified `.mutter-Xwaylandauth.*` file in `dir`.
fn newest_mutter_cookie(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with(".mutter-Xwaylandauth."))
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            Some((meta.modified().ok()?, e.path()))
        })
        .max_by_key(|(mtime, _)| *mtime)
        .map(|(_, path)| path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_xauthority_from_show_environment() {
        let out = "DISPLAY=:0\nXAUTHORITY=/run/user/1000/.mutter-Xwaylandauth.EVK3V3\nWAYLAND_DISPLAY=wayland-0\n";
        assert_eq!(
            parse_show_environment(out),
            Some(PathBuf::from("/run/user/1000/.mutter-Xwaylandauth.EVK3V3"))
        );
    }

    #[test]
    fn strips_quotes_systemd_adds_to_values() {
        assert_eq!(parse_show_environment("XAUTHORITY='/tmp/a b'\n"), Some(PathBuf::from("/tmp/a b")));
    }

    #[test]
    fn missing_or_empty_xauthority_is_none() {
        assert_eq!(parse_show_environment("DISPLAY=:0\n"), None);
        assert_eq!(parse_show_environment("XAUTHORITY=\n"), None);
    }

    #[test]
    fn picks_the_newest_mutter_cookie_and_ignores_other_files() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join(".mutter-Xwaylandauth.OLD111");
        let new = dir.path().join(".mutter-Xwaylandauth.NEW222");
        std::fs::write(&old, b"x").unwrap();
        std::fs::write(dir.path().join("pulse-cookie"), b"x").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&new, b"x").unwrap();
        assert_eq!(newest_mutter_cookie(dir.path()), Some(new));
    }

    #[test]
    fn no_cookie_in_dir_is_none() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("bus"), b"x").unwrap();
        assert_eq!(newest_mutter_cookie(dir.path()), None);
    }
}
