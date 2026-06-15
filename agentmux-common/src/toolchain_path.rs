// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! GUI-launch PATH enrichment.
//!
//! When AgentMux launches from Finder / Dock / a mounted DMG as a `.app`,
//! it inherits launchd's stripped PATH (`/usr/bin:/bin:/usr/sbin:/sbin`).
//! nvm- and Homebrew-installed `node` / `npm` / `git` live in the user's
//! *login-shell* PATH, which a GUI launch never sees — so the CLI installer
//! (`Command::new("npm")`) dies with `npm: command not found`, and spawned
//! agent CLIs can't find their interpreter either.
//!
//! [`resolve_login_path`] reconstructs a usable PATH from the user's login
//! shell (`$SHELL -lic 'printf … "$PATH"'`, with a hard timeout) unioned with
//! well-known toolchain directories. It is **purely additive** — inherited
//! system directories are never dropped — and a no-op on Windows (whose GUI
//! apps inherit a usable PATH).
//!
//! See `docs/specs/SPEC_TOOLCHAIN_MANAGER_2026-06-15.md` §3.

use std::collections::HashSet;
use std::path::Path;

/// How the effective PATH was produced — surfaced in logs and (later) the
/// Toolchain modal so PATH problems are diagnosable rather than mysterious.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathSource {
    /// The inherited PATH was already sufficient (or we're on Windows); unchanged.
    Inherited,
    /// Augmented from the user's login shell (`$SHELL -lic`).
    LoginShell,
    /// Login-shell capture failed; augmented from well-known dirs only.
    FallbackDirs,
}

impl PathSource {
    pub fn as_str(self) -> &'static str {
        match self {
            PathSource::Inherited => "inherited",
            PathSource::LoginShell => "login-shell",
            PathSource::FallbackDirs => "fallback-dirs",
        }
    }
}

/// The enriched PATH plus a record of how it was derived.
#[derive(Debug, Clone)]
pub struct EnrichedPath {
    pub path: String,
    pub source: PathSource,
}

/// The launchd-default PATH a Finder/Dock-launched macOS `.app` inherits.
const LAUNCHD_DEFAULT_DIRS: &[&str] = &["/usr/bin", "/bin", "/usr/sbin", "/sbin"];

/// Build a usable PATH for spawning the toolchain (`node`/`npm`/`git`/`docker`
/// and the provider CLIs). Additive over the inherited PATH; never removes an
/// inherited entry. On Windows this returns the inherited PATH unchanged.
pub fn resolve_login_path() -> EnrichedPath {
    let inherited = std::env::var("PATH").unwrap_or_default();

    #[cfg(not(unix))]
    {
        return EnrichedPath {
            path: inherited,
            source: PathSource::Inherited,
        };
    }

    #[cfg(unix)]
    {
        let login_shell = capture_login_shell_path(&resolve_user_shell());
        build_enriched(&inherited, login_shell.as_deref(), &wellknown_dirs())
    }
}

/// True when the current PATH is the stripped launchd default (every entry is
/// one of `/usr/bin`, `/bin`, `/usr/sbin`, `/sbin`) — i.e. a GUI launch that
/// never saw the user's shell. Used by the srv to decide whether the cheap
/// guard should pay for a login-shell spawn on a *direct* launch.
pub fn looks_like_launchd_default(path: &str) -> bool {
    let defaults: HashSet<&str> = LAUNCHD_DEFAULT_DIRS.iter().copied().collect();
    let entries: Vec<&str> = path.split(':').filter(|e| !e.is_empty()).collect();
    !entries.is_empty() && entries.iter().all(|e| defaults.contains(e))
}

/// Enrich the *current process'* PATH in place, but only when it looks like the
/// stripped launchd default. Idempotent and cheap when the PATH is already
/// healthy (no shell spawn). Returns the source used. Intended for the srv's
/// direct-launch fallback; the host enriches the srv's PATH unconditionally
/// when it spawns it (it knows it is the GUI entry point).
pub fn enrich_current_process_path() -> PathSource {
    let current = std::env::var("PATH").unwrap_or_default();
    if !looks_like_launchd_default(&current) {
        return PathSource::Inherited;
    }
    let enriched = resolve_login_path();
    if enriched.source != PathSource::Inherited {
        std::env::set_var("PATH", &enriched.path);
    }
    enriched.source
}

/// The user's login shell, falling back to a per-OS default.
#[cfg(unix)]
fn resolve_user_shell() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            if cfg!(target_os = "macos") {
                "/bin/zsh".to_string()
            } else {
                "/bin/bash".to_string()
            }
        })
}

/// Run `<shell> -lic 'printf …"$PATH"'` and return the captured PATH, or `None`
/// on timeout / failure / empty. A sentinel wraps the value so noise printed by
/// the user's rc files to stdout can't corrupt the parse. Bounded by a 2s hard
/// timeout so a slow/hung rc file can never block startup.
#[cfg(unix)]
fn capture_login_shell_path(shell: &str) -> Option<String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    use std::time::Duration;

    // `printf` without a trailing newline; markers isolate our value from any
    // banner the rc files emit to stdout.
    let script = r#"printf '__AMUX_PATH__%s__AMUX_END__' "$PATH""#;

    let mut child = Command::new(shell)
        .args(["-lic", script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let mut stdout = child.stdout.take()?;
    // Read on a worker thread so a hung child can't block us past the timeout.
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = String::new();
        let _ = stdout.read_to_string(&mut buf);
        let _ = tx.send(buf);
    });

    let buf = match rx.recv_timeout(Duration::from_secs(2)) {
        Ok(b) => {
            let _ = child.wait();
            b
        }
        Err(_) => {
            // Timed out — kill the child; the reader thread unblocks when the
            // pipe closes and exits on its own (its send is discarded).
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
    };

    parse_sentinel(&buf)
}

/// Extract the PATH wrapped between the sentinels, ignoring surrounding noise.
fn parse_sentinel(buf: &str) -> Option<String> {
    const START: &str = "__AMUX_PATH__";
    const END: &str = "__AMUX_END__";
    let s = buf.find(START)? + START.len();
    let e = buf[s..].find(END)? + s;
    let path = &buf[s..e];
    if path.is_empty() {
        None
    } else {
        Some(path.to_string())
    }
}

/// Well-known toolchain directories, existence-checked and de-duplicated. An
/// absent directory costs nothing (filtered out), so the list can be generous.
#[cfg(unix)]
fn wellknown_dirs() -> Vec<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut candidates: Vec<String> = Vec::new();

    if !home.is_empty() {
        if let Some(bin) = nvm_current_bin(&home) {
            candidates.push(bin);
        }
    }
    candidates.push("/opt/homebrew/bin".to_string());
    candidates.push("/opt/homebrew/sbin".to_string());
    candidates.push("/usr/local/bin".to_string());
    if !home.is_empty() {
        candidates.push(format!("{home}/.local/bin"));
        candidates.push(format!("{home}/.cargo/bin"));
    }
    candidates.push("/opt/local/bin".to_string());

    #[cfg(target_os = "linux")]
    {
        candidates.push("/snap/bin".to_string());
        candidates.push("/var/lib/flatpak/exports/bin".to_string());
        if !home.is_empty() {
            candidates.push(format!("{home}/.local/share/flatpak/exports/bin"));
        }
    }

    candidates
        .into_iter()
        .filter(|d| Path::new(d).is_dir())
        .collect()
}

/// Resolve the `bin` dir of the nvm "current/default" Node, if nvm is present.
/// Prefers the `default` alias; falls back to the highest installed version.
#[cfg(unix)]
fn nvm_current_bin(home: &str) -> Option<String> {
    let versions_dir = format!("{home}/.nvm/versions/node");
    if !Path::new(&versions_dir).is_dir() {
        return None;
    }

    // Prefer the `default` alias (e.g. "v20.18.0", "20", "lts/*", "node").
    if let Ok(alias) = std::fs::read_to_string(format!("{home}/.nvm/alias/default")) {
        let a = alias.trim();
        if !a.is_empty() {
            let v = if a.starts_with('v') {
                a.to_string()
            } else {
                format!("v{a}")
            };
            let bin = format!("{versions_dir}/{v}/bin");
            if Path::new(&bin).is_dir() {
                return Some(bin);
            }
        }
    }

    // Fall back to the highest installed version (numeric, not lexical, so
    // v10 > v9).
    let mut versions: Vec<(u64, u64, u64, String)> = std::fs::read_dir(&versions_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.starts_with('v'))
        .map(|n| {
            let (a, b, c) = parse_semverish(&n);
            (a, b, c, n)
        })
        .collect();
    versions.sort();
    versions
        .last()
        .map(|(_, _, _, n)| format!("{versions_dir}/{n}/bin"))
}

/// Parse a leading "vMAJOR.MINOR.PATCH" into a comparable tuple. Missing or
/// non-numeric parts sort as 0.
fn parse_semverish(v: &str) -> (u64, u64, u64) {
    let core = v.strip_prefix('v').unwrap_or(v);
    let mut it = core.split('.');
    let p = |o: Option<&str>| o.and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
    (p(it.next()), p(it.next()), p(it.next()))
}

/// Pure merge: priority order is login-shell entries, then well-known dirs,
/// then the inherited PATH — de-duplicated, first occurrence wins. Inherited
/// entries are always preserved (appended), so the result is a superset.
fn build_enriched(inherited: &str, login_shell: Option<&str>, wellknown: &[String]) -> EnrichedPath {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut out: Vec<&str> = Vec::new();
    let used_login = login_shell.is_some();

    if let Some(ls) = login_shell {
        for e in ls.split(':') {
            if !e.is_empty() && seen.insert(e) {
                out.push(e);
            }
        }
    }

    let before = out.len();
    for d in wellknown {
        if !d.is_empty() && seen.insert(d.as_str()) {
            out.push(d.as_str());
        }
    }
    let wellknown_added = out.len() > before;

    for e in inherited.split(':') {
        if !e.is_empty() && seen.insert(e) {
            out.push(e);
        }
    }

    let path = out.join(":");
    // Did we actually change anything vs. just the inherited set?
    let changed = path != normalize(inherited);
    let source = if !changed {
        PathSource::Inherited
    } else if used_login {
        PathSource::LoginShell
    } else if wellknown_added {
        PathSource::FallbackDirs
    } else {
        PathSource::Inherited
    };

    EnrichedPath { path, source }
}

/// De-dup + drop empties, preserving order — for comparing "did we change it".
fn normalize(path: &str) -> String {
    let mut seen = HashSet::new();
    path.split(':')
        .filter(|e| !e.is_empty() && seen.insert(*e))
        .collect::<Vec<_>>()
        .join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sentinel_ignores_rc_noise() {
        let buf = "welcome to my shell!\nsome banner\n__AMUX_PATH__/opt/homebrew/bin:/usr/bin__AMUX_END__";
        assert_eq!(
            parse_sentinel(buf).as_deref(),
            Some("/opt/homebrew/bin:/usr/bin")
        );
    }

    #[test]
    fn parse_sentinel_none_when_missing_or_empty() {
        assert_eq!(parse_sentinel("no markers here"), None);
        assert_eq!(parse_sentinel("__AMUX_PATH____AMUX_END__"), None);
    }

    #[test]
    fn launchd_default_detection() {
        assert!(looks_like_launchd_default("/usr/bin:/bin:/usr/sbin:/sbin"));
        assert!(looks_like_launchd_default("/bin:/usr/bin"));
        assert!(!looks_like_launchd_default(
            "/opt/homebrew/bin:/usr/bin:/bin"
        ));
        assert!(!looks_like_launchd_default(""));
    }

    #[test]
    fn semverish_sorts_numerically() {
        assert!(parse_semverish("v10.0.0") > parse_semverish("v9.99.99"));
        assert_eq!(parse_semverish("v20.18.1"), (20, 18, 1));
        assert_eq!(parse_semverish("vbogus"), (0, 0, 0));
    }

    #[test]
    fn build_prepends_login_shell_and_preserves_inherited() {
        let e = build_enriched(
            "/usr/bin:/bin",
            Some("/opt/homebrew/bin:/usr/bin:/bin"),
            &[],
        );
        // login-shell entry comes first; inherited /bin still present once.
        assert_eq!(e.path, "/opt/homebrew/bin:/usr/bin:/bin");
        assert_eq!(e.source, PathSource::LoginShell);
    }

    #[test]
    fn build_falls_back_to_wellknown_when_no_login_shell() {
        let e = build_enriched(
            "/usr/bin:/bin",
            None,
            &["/opt/homebrew/bin".to_string()],
        );
        assert_eq!(e.path, "/opt/homebrew/bin:/usr/bin:/bin");
        assert_eq!(e.source, PathSource::FallbackDirs);
    }

    #[test]
    fn build_reports_inherited_when_nothing_added() {
        // login shell returns exactly the inherited set (e.g. launched from a
        // terminal that already had the system PATH and nothing else).
        let e = build_enriched("/usr/bin:/bin", Some("/usr/bin:/bin"), &[]);
        assert_eq!(e.path, "/usr/bin:/bin");
        assert_eq!(e.source, PathSource::Inherited);
    }

    #[test]
    fn build_dedups_across_all_sources() {
        let e = build_enriched(
            "/usr/bin:/bin:/usr/bin",
            Some("/opt/homebrew/bin:/usr/bin"),
            &["/opt/homebrew/bin".to_string(), "/usr/local/bin".to_string()],
        );
        assert_eq!(e.path, "/opt/homebrew/bin:/usr/bin:/usr/local/bin:/bin");
    }

    #[cfg(unix)]
    #[test]
    fn capture_via_real_sh_roundtrips_path() {
        // /bin/sh honors -c; -lic is accepted (login+interactive flags are
        // tolerated). The script echoes $PATH between the sentinels.
        let got = capture_login_shell_path("/bin/sh");
        // On any unix dev/CI box /bin/sh exists and $PATH is non-empty.
        assert!(got.is_some(), "expected a captured PATH from /bin/sh");
        assert!(!got.unwrap().is_empty());
    }
}
