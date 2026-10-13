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
        let login_shell = capture_login_shell_path(&resolve_user_shell(), None);
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

/// What [`enrich_current_process_path_cached`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnrichOutcome {
    pub source: PathSource,
    /// The login-shell PATH came from the cache, not a fresh shell spawn.
    /// The caller should refresh the cache in the background
    /// ([`refresh_login_path_cache`]).
    pub from_cache: bool,
}

/// Like [`enrich_current_process_path`], but reuses the login-shell PATH
/// captured by an earlier launch when nothing it depends on has changed:
/// the shell, the inherited PATH, and the size and mtime of every shell
/// startup file and `/etc/paths{,.d}`. On a hit no shell is spawned, which
/// takes the 1-2 s an interactive login shell costs (oh-my-zsh, conda, nvm)
/// off the boot path. A miss spawns the shell as before and writes the cache.
///
/// Startup files can source other files (a plugin, `nvm alias default`) whose
/// changes the fingerprint can't see, so a hit is only as fresh as the last
/// launch: run [`refresh_login_path_cache`] after boot so the next launch
/// picks such a change up.
pub fn enrich_current_process_path_cached(cache_file: Option<&Path>) -> EnrichOutcome {
    let current = std::env::var("PATH").unwrap_or_default();
    if !looks_like_launchd_default(&current) {
        return EnrichOutcome { source: PathSource::Inherited, from_cache: false };
    }
    match cache_file {
        #[cfg(unix)]
        Some(cache_file) => enrich_from_cache_or_shell(cache_file, &current),
        _ => EnrichOutcome { source: enrich_current_process_path(), from_cache: false },
    }
}

#[cfg(unix)]
fn enrich_from_cache_or_shell(cache_file: &Path, current: &str) -> EnrichOutcome {
    let shell = resolve_user_shell();
    let fingerprint = login_path_fingerprint(&shell, current);
    let (login, from_cache) = match cache::read(cache_file, &fingerprint) {
        Some(login) => (Some(login), true),
        None => {
            let login = capture_login_shell_path(&shell, None);
            if let Some(login) = login.as_deref() {
                cache::write(cache_file, &fingerprint, login);
            }
            (login, false)
        }
    };
    let enriched = build_enriched(current, login.as_deref(), &wellknown_dirs());
    if enriched.source != PathSource::Inherited {
        std::env::set_var("PATH", &enriched.path);
    }
    EnrichOutcome { source: enriched.source, from_cache }
}

/// Re-run the login shell and rewrite the cache that
/// [`enrich_current_process_path_cached`] read. `inherited` is the PATH the
/// process started with (before enrichment), so the fingerprint matches the
/// next launch's. Returns the freshly captured enriched PATH when it differs
/// from `current_enriched`, `None` when nothing changed or the shell
/// couldn't be read. Never touches this process' environment.
#[cfg(unix)]
pub fn refresh_login_path_cache(
    cache_file: &Path,
    inherited: &str,
    current_enriched: &str,
) -> Option<String> {
    let shell = resolve_user_shell();
    let fingerprint = login_path_fingerprint(&shell, inherited);
    // Start the shell from the PATH this process inherited, as the boot-time
    // capture did, not the enriched one now in the environment: a startup
    // file that appends to $PATH would otherwise carry every cached entry
    // forward, and a directory removed from the user's setup would never
    // drop out of the cache.
    let login = capture_login_shell_path(&shell, Some(inherited))?;
    cache::write(cache_file, &fingerprint, &login);
    let enriched = build_enriched(inherited, Some(&login), &wellknown_dirs());
    (enriched.path != current_enriched).then_some(enriched.path)
}

/// Everything the login shell's PATH can depend on that is cheap to check:
/// the shell, the PATH it was started with, and the size + mtime of each
/// startup file bash/zsh read and of `path_helper`'s inputs. A changed or
/// newly created file changes the fingerprint.
#[cfg(unix)]
fn login_path_fingerprint(shell: &str, inherited: &str) -> String {
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(home) = std::env::var("HOME") {
        dirs.push(home.into());
    }
    if let Ok(zdot) = std::env::var("ZDOTDIR") {
        dirs.push(zdot.into());
    }
    fingerprint_with_dirs(shell, inherited, &dirs)
}

/// [`login_path_fingerprint`] with the per-user startup-file directories
/// passed in, so tests don't have to change `HOME`.
#[cfg(unix)]
fn fingerprint_with_dirs(shell: &str, inherited: &str, dirs: &[std::path::PathBuf]) -> String {
    use std::fmt::Write as _;

    let mut files: Vec<std::path::PathBuf> = [
        "/etc/zshenv", "/etc/zprofile", "/etc/zshrc", "/etc/zlogin",
        "/etc/profile", "/etc/bashrc", "/etc/paths", "/etc/paths.d",
    ]
    .iter()
    .map(std::path::PathBuf::from)
    .collect();
    for dir in dirs {
        for name in [
            ".zshenv", ".zprofile", ".zshrc", ".zlogin",
            ".bash_profile", ".bash_login", ".profile", ".bashrc",
        ] {
            files.push(dir.join(name));
        }
    }
    if let Ok(entries) = std::fs::read_dir("/etc/paths.d") {
        let mut extra: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        extra.sort();
        files.extend(extra);
    }

    let mut fp = format!("shell={shell}\ninherited={inherited}\n");
    for f in &files {
        match std::fs::metadata(f) {
            Ok(m) => {
                let mtime = m
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_nanos())
                    .unwrap_or(0);
                let _ = writeln!(fp, "{}={}:{}", f.display(), m.len(), mtime);
            }
            Err(_) => {
                let _ = writeln!(fp, "{}=-", f.display());
            }
        }
    }
    fp
}

#[cfg(unix)]
mod cache {
    use std::path::Path;

    #[derive(serde::Serialize, serde::Deserialize)]
    struct Entry {
        fingerprint: String,
        login_path: String,
    }

    pub(super) fn read(file: &Path, fingerprint: &str) -> Option<String> {
        let text = std::fs::read_to_string(file).ok()?;
        let entry: Entry = serde_json::from_str(&text).ok()?;
        (entry.fingerprint == fingerprint && !entry.login_path.is_empty())
            .then_some(entry.login_path)
    }

    /// Best effort: a cache that can't be written only costs the next launch
    /// a shell spawn. Written via a temp file + rename so a concurrent reader
    /// (another channel's srv starting) never sees half a file.
    pub(super) fn write(file: &Path, fingerprint: &str, login_path: &str) {
        let Some(dir) = file.parent() else { return };
        if std::fs::create_dir_all(dir).is_err() {
            return;
        }
        let entry = Entry {
            fingerprint: fingerprint.to_string(),
            login_path: login_path.to_string(),
        };
        let Ok(json) = serde_json::to_string(&entry) else { return };
        let tmp = file.with_extension(format!("tmp.{}", std::process::id()));
        if std::fs::write(&tmp, json).is_ok() && std::fs::rename(&tmp, file).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
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
fn capture_login_shell_path(shell: &str, path: Option<&str>) -> Option<String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    use std::time::Duration;

    // `printf` without a trailing newline; markers isolate our value from any
    // banner the rc files emit to stdout.
    let script = r#"printf '__AMUX_PATH__%s__AMUX_END__' "$PATH""#;

    let mut cmd = Command::new(shell);
    if let Some(path) = path {
        cmd.env("PATH", path);
    }
    let mut child = cmd
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
#[cfg(any(unix, test))]
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
#[cfg(any(unix, test))]
fn parse_semverish(v: &str) -> (u64, u64, u64) {
    let core = v.strip_prefix('v').unwrap_or(v);
    let mut it = core.split('.');
    let p = |o: Option<&str>| o.and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
    (p(it.next()), p(it.next()), p(it.next()))
}

/// Pure merge: priority order is login-shell entries, then well-known dirs,
/// then the inherited PATH — de-duplicated, first occurrence wins. Inherited
/// entries are always preserved (appended), so the result is a superset.
#[cfg(any(unix, test))]
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
#[cfg(any(unix, test))]
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
        let got = capture_login_shell_path("/bin/sh", None);
        // On any unix dev/CI box /bin/sh exists and $PATH is non-empty.
        assert!(got.is_some(), "expected a captured PATH from /bin/sh");
        assert!(!got.unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn login_path_cache_hits_only_on_the_same_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("cache").join("login-path.json");
        assert_eq!(cache::read(&file, "fp-a"), None, "no file yet");

        cache::write(&file, "fp-a", "/opt/homebrew/bin:/usr/bin");
        assert_eq!(
            cache::read(&file, "fp-a").as_deref(),
            Some("/opt/homebrew/bin:/usr/bin")
        );
        assert_eq!(cache::read(&file, "fp-b"), None, "another fingerprint is a miss");

        std::fs::write(&file, "not json").unwrap();
        assert_eq!(cache::read(&file, "fp-a"), None, "a corrupt file is a miss");
    }

    #[cfg(unix)]
    #[test]
    fn fingerprint_changes_when_a_startup_file_changes() {
        let home = tempfile::tempdir().unwrap();
        let dirs = vec![home.path().to_path_buf()];
        let base = fingerprint_with_dirs("/bin/zsh", "/usr/bin:/bin", &dirs);
        assert_eq!(base, fingerprint_with_dirs("/bin/zsh", "/usr/bin:/bin", &dirs), "stable");

        std::fs::write(home.path().join(".zshrc"), "export PATH=/x:$PATH\n").unwrap();
        let created = fingerprint_with_dirs("/bin/zsh", "/usr/bin:/bin", &dirs);
        assert_ne!(base, created, "a new startup file must miss the cache");

        std::fs::write(home.path().join(".zshrc"), "export PATH=/longer/x:$PATH\n").unwrap();
        assert_ne!(
            created,
            fingerprint_with_dirs("/bin/zsh", "/usr/bin:/bin", &dirs),
            "an edited startup file must miss the cache"
        );

        assert_ne!(base, fingerprint_with_dirs("/bin/bash", "/usr/bin:/bin", &dirs), "shell");
        assert_ne!(base, fingerprint_with_dirs("/bin/zsh", "/usr/bin", &dirs), "inherited PATH");
    }
}
