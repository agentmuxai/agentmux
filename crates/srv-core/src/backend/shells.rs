// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The shells installed on this machine, and which one a new local terminal
//! runs: the default-shell setting
//! (docs/specs/SPEC_DEFAULT_SHELL_SETTING_2026_10_13.md).
//!
//! - [`list`]: every shell found, with a name and version, for Settings.
//! - [`resolve_for_spawn`]: the shell a terminal starts with. The block's own
//!   `term:localshellpath`, else the global setting, else the automatic choice
//!   (PowerShell 7, then Windows PowerShell, then Command Prompt on Windows;
//!   `$SHELL`, then `/bin/bash` elsewhere). A chosen shell that isn't there
//!   falls back to the automatic one, so a stale setting never stops a
//!   terminal from opening.
//!
//! Finding candidates is written over an injected environment and filesystem,
//! with Windows paths handled as strings, like `posix_shell::resolve_windows`,
//! so the order is tested on every host.

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::backend::blockcontroller::{META_KEY_TERM_LOCAL_SHELL_OPTS, META_KEY_TERM_LOCAL_SHELL_PATH};
use crate::backend::obj::MetaMapType;
use crate::backend::rpc_types::{EffectiveShell, ShellInfo, ShellList};
use crate::backend::wconfig::SettingsType;

/// How long a shell may take to say its version before it is listed without one.
const VERSION_TIMEOUT: Duration = Duration::from_secs(3);
/// How long a listing is reused: finding versions runs every shell once.
const CACHE_TTL: Duration = Duration::from_secs(30);

const SEPARATORS: [char; 2] = ['\\', '/'];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Pwsh,
    PowerShell,
    Cmd,
    GitBash,
    Bash,
    Zsh,
    Fish,
    Nu,
    Sh,
    Other,
}

/// A path's file name, on either separator, so a Windows path reads the same on any host.
fn file_name(path: &Path) -> String {
    let s = path.to_string_lossy();
    s.rsplit(SEPARATORS).next().unwrap_or(&s).to_string()
}

impl Kind {
    fn of(path: &Path) -> Kind {
        let lower = path.to_string_lossy().to_ascii_lowercase();
        let name = file_name(Path::new(&lower));
        let stem = name.strip_suffix(".exe").unwrap_or(&name);
        let in_git = lower.split(SEPARATORS).any(|part| part == "git");
        match stem {
            "pwsh" => Kind::Pwsh,
            "powershell" => Kind::PowerShell,
            "cmd" => Kind::Cmd,
            "bash" if in_git => Kind::GitBash,
            "bash" => Kind::Bash,
            "zsh" => Kind::Zsh,
            "fish" => Kind::Fish,
            "nu" => Kind::Nu,
            "sh" | "dash" => Kind::Sh,
            _ => Kind::Other,
        }
    }

    fn id(self) -> &'static str {
        match self {
            Kind::Pwsh => "pwsh",
            Kind::PowerShell => "powershell",
            Kind::Cmd => "cmd",
            Kind::GitBash => "gitbash",
            Kind::Bash => "bash",
            Kind::Zsh => "zsh",
            Kind::Fish => "fish",
            Kind::Nu => "nu",
            Kind::Sh => "sh",
            Kind::Other => "other",
        }
    }

    fn name(self, path: &Path) -> String {
        match self {
            Kind::Pwsh => "PowerShell".into(),
            Kind::PowerShell => "Windows PowerShell".into(),
            Kind::Cmd => "Command Prompt".into(),
            Kind::GitBash => "Git Bash".into(),
            Kind::Bash => "Bash".into(),
            Kind::Zsh => "Zsh".into(),
            Kind::Fish => "fish".into(),
            Kind::Nu => "Nushell".into(),
            Kind::Sh | Kind::Other => file_name(path),
        }
    }
}

/// A shell found, before its version is asked.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Candidate {
    path: PathBuf,
    kind: Kind,
}

fn candidate(path: PathBuf) -> Candidate {
    let kind = Kind::of(&path);
    Candidate { path, kind }
}

/// Drop repeats: Windows paths compare case-insensitively, and the same file
/// can be reached twice (`/bin/bash` and `/usr/bin/bash` on a merged-usr system).
fn dedupe(cands: Vec<Candidate>, canonical: &dyn Fn(&Path) -> PathBuf) -> Vec<Candidate> {
    let mut seen = HashSet::new();
    cands
        .into_iter()
        .filter(|c| seen.insert(canonical(&c.path).to_string_lossy().to_ascii_lowercase()))
        .collect()
}

/// `base\rel`, built as a string: `PathBuf::join` would use `/` when the tests
/// run on a Unix host.
fn wjoin(base: &str, rel: &str) -> String {
    format!("{}\\{}", base.trim_end_matches(SEPARATORS), rel)
}

/// The folder holding a Windows path, on either separator.
fn wparent(p: &str) -> Option<&str> {
    let p = p.trim_end_matches(SEPARATORS);
    p.rfind(SEPARATORS).map(|i| &p[..i])
}

/// Windows candidates, most familiar first: PowerShell 7, Windows PowerShell,
/// Command Prompt, Git Bash, MSYS2 and Cygwin bash, then Nushell and fish on PATH.
#[cfg_attr(not(windows), allow(dead_code))]
fn windows_candidates(env: &dyn Fn(&str) -> Option<OsString>, is_file: &dyn Fn(&Path) -> bool) -> Vec<Candidate> {
    let var = |k: &str| env(k).map(|v| v.to_string_lossy().into_owned()).filter(|v| !v.is_empty());
    let path: Vec<String> = var("PATH")
        .map(|v| {
            v.split(';')
                .map(|s| s.trim().trim_matches('"').to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let is = |p: &str| is_file(Path::new(p));
    let on_path = |exe: &str| path.iter().map(|d| wjoin(d, exe)).filter(|p| is(p)).collect::<Vec<_>>();
    let program_roots: Vec<String> = ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"]
        .into_iter()
        .filter_map(var)
        .collect();
    let system_root = var("SystemRoot").unwrap_or_else(|| r"C:\Windows".to_string());

    let mut out: Vec<String> = Vec::new();
    // PowerShell 7: PATH (an MSI or winget install), its default install
    // folders, and the Store's alias.
    out.extend(on_path("pwsh.exe"));
    for root in &program_roots {
        for dir in ["7", "7-preview"] {
            out.push(wjoin(root, &format!(r"PowerShell\{dir}\pwsh.exe")));
        }
    }
    if let Some(local) = var("LOCALAPPDATA") {
        out.push(wjoin(&local, r"Microsoft\WindowsApps\pwsh.exe"));
    }
    out.push(wjoin(&system_root, r"System32\WindowsPowerShell\v1.0\powershell.exe"));
    out.push(var("ComSpec").unwrap_or_else(|| wjoin(&system_root, r"System32\cmd.exe")));
    // Git Bash: Git's `bin\bash.exe` launcher (it sets up the MSYS PATH), from
    // the install that owns the `git.exe` on PATH (at <root>\cmd, <root>\bin
    // or <root>\mingw64\bin), then the standard roots.
    for git in on_path("git.exe") {
        let mut dir = wparent(&git).and_then(wparent);
        for _ in 0..3 {
            let Some(root) = dir else { break };
            out.push(wjoin(root, r"bin\bash.exe"));
            dir = wparent(root);
        }
    }
    for root in &program_roots {
        out.push(wjoin(root, r"Git\bin\bash.exe"));
    }
    if let Some(local) = var("LOCALAPPDATA") {
        out.push(wjoin(&local, r"Programs\Git\bin\bash.exe"));
    }
    out.push(r"C:\msys64\usr\bin\bash.exe".to_string());
    out.push(r"C:\cygwin64\bin\bash.exe".to_string());
    out.extend(on_path("nu.exe"));
    out.extend(on_path("fish.exe"));
    out.into_iter().filter(|p| is(p)).map(|p| candidate(PathBuf::from(p))).collect()
}

/// macOS and Linux candidates: the login shell, then `/etc/shells`, then
/// Homebrew's shells that `/etc/shells` doesn't list.
#[cfg_attr(windows, allow(dead_code))]
fn unix_candidates(login: Option<PathBuf>, etc_shells: &str, is_file: &dyn Fn(&Path) -> bool) -> Vec<Candidate> {
    let mut out: Vec<PathBuf> = login.into_iter().collect();
    out.extend(
        etc_shells
            .lines()
            .map(str::trim)
            .filter(|l| l.starts_with('/'))
            .map(PathBuf::from),
    );
    for dir in ["/opt/homebrew/bin", "/usr/local/bin"] {
        for sh in ["zsh", "bash", "fish", "nu"] {
            out.push(Path::new(dir).join(sh));
        }
    }
    out.into_iter()
        .filter(|p| is_file(p))
        // Not shells to log in with.
        .filter(|p| !matches!(file_name(p).as_str(), "nologin" | "false" | "git-shell"))
        .map(candidate)
        .collect()
}

fn real_env(k: &str) -> Option<OsString> {
    std::env::var_os(k)
}

fn real_is_file(p: &Path) -> bool {
    p.is_file()
}

fn real_canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

fn candidates() -> Vec<Candidate> {
    let found = if cfg!(windows) {
        windows_candidates(&real_env, &real_is_file)
    } else {
        let login = std::env::var_os("SHELL").filter(|s| !s.is_empty()).map(PathBuf::from);
        let etc = std::fs::read_to_string("/etc/shells").unwrap_or_default();
        unix_candidates(login, &etc, &real_is_file)
    };
    dedupe(found, &real_canonical)
}

/// The automatic choice, as before the setting was honoured: PowerShell 7 on
/// PATH, else Windows PowerShell, else Command Prompt; elsewhere `$SHELL`,
/// else `/bin/bash`.
pub fn automatic() -> PathBuf {
    if cfg!(windows) {
        for exe in ["pwsh", "powershell"] {
            if let Ok(p) = which::which(exe) {
                return p;
            }
        }
        std::env::var_os("ComSpec")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("cmd.exe"))
    } else {
        std::env::var_os("SHELL")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/bin/bash"))
    }
}

/// A configured shell, as an existing file: a path that exists, or a bare name
/// (`pwsh`, `nu`) found on PATH.
fn locate(configured: &str) -> Option<PathBuf> {
    if configured.contains(SEPARATORS) {
        let p = PathBuf::from(configured);
        p.is_file().then_some(p)
    } else {
        which::which(configured).ok()
    }
}

/// The shell a new local terminal starts with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnShell {
    pub path: PathBuf,
    /// `block`, `setting`, `automatic`, or `fallback` (a configured shell wasn't found).
    pub source: &'static str,
    /// The configured shell that wasn't found, for `fallback`.
    pub missing: Option<String>,
}

/// Which shell a terminal starts with: the block's `term:localshellpath`, then
/// the global setting, then [`automatic`].
pub fn resolve_for_spawn(block_setting: &str, global_setting: &str) -> SpawnShell {
    resolve_with(block_setting, global_setting, &locate, &automatic)
}

fn resolve_with(
    block_setting: &str,
    global_setting: &str,
    locate: &dyn Fn(&str) -> Option<PathBuf>,
    automatic: &dyn Fn() -> PathBuf,
) -> SpawnShell {
    let (configured, source) = match (block_setting.trim(), global_setting.trim()) {
        (b, _) if !b.is_empty() => (b, "block"),
        (_, g) if !g.is_empty() => (g, "setting"),
        _ => return SpawnShell { path: automatic(), source: "automatic", missing: None },
    };
    match locate(configured) {
        Some(path) => SpawnShell { path, source, missing: None },
        None => SpawnShell { path: automatic(), source: "fallback", missing: Some(configured.to_string()) },
    }
}

/// The shell and extra arguments a block's interactive terminal starts with:
/// [`resolve_for_spawn`] on the block's and the global `term:localshellpath`,
/// and the block's `term:localshellopts`, else the setting's (appended after
/// the shell-integration arguments). Logs a configured shell that wasn't found.
pub fn for_block(block_id: &str, block_meta: &MetaMapType, settings: &SettingsType) -> (PathBuf, Vec<String>) {
    let own_path = block_meta.get(META_KEY_TERM_LOCAL_SHELL_PATH).and_then(|v| v.as_str()).unwrap_or("");
    let shell = resolve_for_spawn(own_path, &settings.term_local_shell_path);
    if let Some(missing) = &shell.missing {
        tracing::warn!(block_id, missing = %missing, using = %shell.path.display(), "configured shell not found; using the automatic one");
    }
    (shell.path, shell_opts(block_meta, &settings.term_local_shell_opts))
}

/// The block's own `term:localshellopts` when it has the key, even empty (an
/// agent's shell pins `[]`), else the setting's.
fn shell_opts(block_meta: &MetaMapType, setting: &[String]) -> Vec<String> {
    match block_meta.get(META_KEY_TERM_LOCAL_SHELL_OPTS).and_then(|v| v.as_array()) {
        Some(own) => own.iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
        None => setting.to_vec(),
    }
}

/// The shell's arguments: the integration ones and the user's
/// `term:localshellopts`. PowerShell's integration ends `-File <script>`, and
/// anything after `-File` goes to the script, so the user's options go before
/// it; elsewhere they follow (bash wants its long options, `--rcfile`, first).
pub fn with_user_opts(integration: &[String], opts: &[String]) -> Vec<String> {
    let at = integration
        .iter()
        .position(|a| a.eq_ignore_ascii_case("-File"))
        .unwrap_or(integration.len());
    let mut out = integration[..at].to_vec();
    out.extend_from_slice(opts);
    out.extend_from_slice(&integration[at..]);
    out
}

/// Ask a shell its version, briefly.
fn version_of(c: &Candidate) -> Option<String> {
    let args: &[&str] = match c.kind {
        Kind::Pwsh | Kind::PowerShell => &["-NoLogo", "-NoProfile", "-Command", "$PSVersionTable.PSVersion.ToString()"],
        Kind::Cmd | Kind::Other => return None,
        _ => &["--version"],
    };
    let mut cmd = std::process::Command::new(&c.path);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use agentmux_common::win32::NoWindow;
        cmd.no_window();
    }
    let mut child = cmd.spawn().ok()?;
    let deadline = Instant::now() + VERSION_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let out = child.wait_with_output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    crate::backend::cli_notice::version_token(text.lines().next().unwrap_or_default())
}

static CACHE: Mutex<Option<(Instant, Vec<ShellInfo>)>> = Mutex::new(None);

/// Every shell found, with versions asked in parallel. Reused for [`CACHE_TTL`].
fn installed() -> Vec<ShellInfo> {
    if let Some((at, shells)) = CACHE.lock().unwrap().as_ref() {
        if at.elapsed() < CACHE_TTL {
            return shells.clone();
        }
    }
    let cands = candidates();
    let versions: Vec<Option<String>> = std::thread::scope(|s| {
        let handles: Vec<_> = cands.iter().map(|c| s.spawn(move || version_of(c))).collect();
        handles.into_iter().map(|h| h.join().ok().flatten()).collect()
    });
    let shells: Vec<ShellInfo> = cands
        .iter()
        .zip(versions)
        .map(|(c, version)| ShellInfo {
            path: c.path.display().to_string(),
            name: c.kind.name(&c.path),
            kind: c.kind.id().to_string(),
            version,
        })
        .collect();
    *CACHE.lock().unwrap() = Some((Instant::now(), shells.clone()));
    shells
}

/// What Settings shows: the shells found, the automatic choice, and what a new
/// terminal starts with under `global_setting`. Blocking: run it off the
/// async runtime.
pub fn list(global_setting: &str) -> ShellList {
    let shells = installed();
    let spawn = resolve_for_spawn("", global_setting);
    ShellList {
        shells,
        automatic: automatic().display().to_string(),
        effective: EffectiveShell {
            path: spawn.path.display().to_string(),
            source: spawn.source.to_string(),
            missing: spawn.missing,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env_of(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let m: HashMap<String, String> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |k: &str| m.get(k).map(OsString::from)
    }

    fn files(paths: &[&str]) -> impl Fn(&Path) -> bool {
        let set: HashSet<String> = paths.iter().map(|p| p.to_ascii_lowercase()).collect();
        move |p: &Path| set.contains(&p.to_string_lossy().to_ascii_lowercase())
    }

    #[test]
    fn windows_lists_the_familiar_shells_in_order() {
        let env = env_of(&[
            ("PATH", r"C:\Program Files\PowerShell\7;C:\Program Files\Git\cmd;C:\tools\nu"),
            ("ProgramFiles", r"C:\Program Files"),
            ("SystemRoot", r"C:\Windows"),
            ("ComSpec", r"C:\Windows\System32\cmd.exe"),
        ]);
        let is_file = files(&[
            r"C:\Program Files\PowerShell\7\pwsh.exe",
            r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe",
            r"C:\Windows\System32\cmd.exe",
            r"C:\Program Files\Git\cmd\git.exe",
            r"C:\Program Files\Git\bin\bash.exe",
            r"C:\tools\nu\nu.exe",
        ]);
        let found = dedupe(windows_candidates(&env, &is_file), &|p| p.to_path_buf());
        let kinds: Vec<&str> = found.iter().map(|c| c.kind.id()).collect();
        assert_eq!(kinds, ["pwsh", "powershell", "cmd", "gitbash", "nu"]);
    }

    #[test]
    fn windows_lists_nothing_that_isnt_there() {
        let env = env_of(&[("SystemRoot", r"C:\Windows")]);
        let found = windows_candidates(&env, &files(&[r"C:\Windows\System32\cmd.exe"]));
        assert_eq!(found.iter().map(|c| c.kind.id()).collect::<Vec<_>>(), ["cmd"]);
    }

    #[test]
    fn unix_lists_the_login_shell_first_and_skips_non_shells() {
        let etc = "# /etc/shells\n/bin/sh\n/bin/bash\n/usr/bin/zsh\n/usr/sbin/nologin\n/bin/false\n";
        let is_file = files(&["/bin/sh", "/bin/bash", "/usr/bin/zsh", "/usr/sbin/nologin", "/bin/false"]);
        let found = unix_candidates(Some(PathBuf::from("/usr/bin/zsh")), etc, &is_file);
        let found = dedupe(found, &|p| p.to_path_buf());
        let paths: Vec<String> = found.iter().map(|c| c.path.display().to_string()).collect();
        assert_eq!(paths, ["/usr/bin/zsh", "/bin/sh", "/bin/bash"]);
    }

    #[test]
    fn the_same_file_reached_twice_is_listed_once() {
        let etc = "/bin/bash\n/usr/bin/bash\n";
        let is_file = files(&["/bin/bash", "/usr/bin/bash"]);
        let canonical = |p: &Path| {
            if p == Path::new("/bin/bash") {
                PathBuf::from("/usr/bin/bash")
            } else {
                p.to_path_buf()
            }
        };
        let found = dedupe(unix_candidates(None, etc, &is_file), &canonical);
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn names_say_what_the_shell_is() {
        assert_eq!(Kind::of(Path::new(r"C:\Program Files\Git\bin\bash.exe")), Kind::GitBash);
        assert_eq!(Kind::of(Path::new(r"C:\msys64\usr\bin\bash.exe")), Kind::Bash);
        assert_eq!(Kind::of(Path::new("/bin/bash")), Kind::Bash);
        assert_eq!(Kind::of(Path::new(r"C:\Windows\System32\cmd.exe")), Kind::Cmd);
        assert_eq!(Kind::GitBash.name(Path::new("x")), "Git Bash");
        assert_eq!(Kind::Other.name(Path::new(r"C:\tools\xonsh.exe")), "xonsh.exe");
    }

    #[test]
    fn the_blocks_own_options_win_over_the_setting() {
        let mut meta = MetaMapType::new();
        let setting = vec!["-NoLogo".to_string()];
        assert_eq!(shell_opts(&meta, &setting), setting);
        meta.insert(META_KEY_TERM_LOCAL_SHELL_OPTS.into(), serde_json::json!(["-l"]));
        assert_eq!(shell_opts(&meta, &setting), vec!["-l".to_string()]);
        // An empty list of its own means none, not the setting's.
        meta.insert(META_KEY_TERM_LOCAL_SHELL_OPTS.into(), serde_json::json!([]));
        assert!(shell_opts(&meta, &setting).is_empty());
    }

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn user_options_go_before_powershells_file_and_after_bashs_rcfile() {
        let pwsh = strings(&["-ExecutionPolicy", "Bypass", "-NoExit", "-File", "wavepwsh.ps1"]);
        assert_eq!(
            with_user_opts(&pwsh, &strings(&["-NoLogo"])),
            strings(&["-ExecutionPolicy", "Bypass", "-NoExit", "-NoLogo", "-File", "wavepwsh.ps1"])
        );
        let bash = strings(&["--rcfile", "bashrc"]);
        assert_eq!(with_user_opts(&bash, &strings(&["-l"])), strings(&["--rcfile", "bashrc", "-l"]));
        assert_eq!(with_user_opts(&[], &strings(&["/K"])), strings(&["/K"]));
    }

    fn auto() -> PathBuf {
        PathBuf::from("auto-shell")
    }

    #[test]
    fn spawn_uses_the_block_then_the_setting_then_automatic() {
        let locate = |s: &str| Some(PathBuf::from(s));
        assert_eq!(resolve_with("blk", "glob", &locate, &auto).source, "block");
        assert_eq!(resolve_with("", "glob", &locate, &auto).path, PathBuf::from("glob"));
        let a = resolve_with("", "  ", &locate, &auto);
        assert_eq!((a.path, a.source), (auto(), "automatic"));
    }

    #[test]
    fn a_shell_that_isnt_there_falls_back_to_automatic_and_says_so() {
        let locate = |_: &str| None;
        let r = resolve_with("", r"C:\gone\nu.exe", &locate, &auto);
        assert_eq!(r.path, auto());
        assert_eq!(r.source, "fallback");
        assert_eq!(r.missing.as_deref(), Some(r"C:\gone\nu.exe"));
    }
}
