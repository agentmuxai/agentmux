// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Locate the POSIX `sh` that `shellexec` (the `!cmd` composer prefix) runs
//! its one-shot `sh -c` commands under.
//!
//! On Unix this is just `sh`. On Windows it was also just `sh`, which only
//! resolves when srv's PATH happens to contain a Git Bash tool directory —
//! true for a dev build started from a Git Bash terminal, false for an
//! installed/portable build started from Explorer. Git for Windows' installer
//! puts only `Git\cmd` on the system PATH by default, and that directory has
//! `git.exe` but no `sh.exe`, so every `!cmd` failed with
//! `shellexec: spawn failed: program not found`.
//!
//! Two traps shape the Windows search order:
//!
//! * `Git\usr\bin\sh.exe` (the real binary) cannot run `ls` unless its own
//!   directory is already on PATH. `Git\bin\sh.exe` is Git's launcher: it
//!   prepends `/mingw64/bin:/usr/bin` itself, so the fallbacks below point at
//!   `bin\sh.exe`, never `usr\bin\sh.exe`.
//! * `C:\Windows\System32\bash.exe` is WSL. A PATH search for `bash` would run
//!   `!cmd` inside a Linux VM with different paths, so the PATH search looks
//!   for `sh.exe` only (WSL ships no `sh.exe`).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Explicit override: a path to a bash/sh executable. Same variable
/// agentmux-bashwrap honours for its own bash lookup.
#[cfg_attr(not(windows), allow(dead_code))]
pub const SHELL_OVERRIDE_ENV: &str = "AGENTMUX_BASH";

/// Resolve the program `shellexec` should spawn as `<program> -c <cmd>`.
pub fn resolve_posix_shell() -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        resolve_windows(&|k| std::env::var_os(k), &|p| p.is_file()).ok_or_else(|| {
            format!(
                "no POSIX shell found — `!` commands need Git for Windows \
                 (https://git-scm.com/download/win), or set {SHELL_OVERRIDE_ENV} \
                 to a bash.exe/sh.exe"
            )
        })
    }
    #[cfg(not(windows))]
    {
        Ok(PathBuf::from("sh"))
    }
}

/// Windows search order, first hit wins:
///
/// 1. `AGENTMUX_BASH`, if it names an existing file.
/// 2. `sh.exe` on PATH — the case that already worked (srv started from Git
///    Bash, or a PATH that includes a Git tool directory). Its directory is on
///    PATH, so the tools it needs are too.
/// 3. The Git install that owns the `git.exe` on PATH → its `bin\sh.exe`.
///    Covers a Git installed anywhere, as long as `git` itself works.
/// 4. Standard Git install roots (machine-wide and per-user) → `bin\sh.exe`.
///
/// Relative PATH entries are skipped: resolving `.` against srv's cwd would
/// let a directory's contents choose the shell.
///
/// Platform-neutral (takes its environment and filesystem as parameters, and
/// splits PATH on `;` itself) so the order is unit-tested on every CI host.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn resolve_windows(
    env: &dyn Fn(&str) -> Option<OsString>,
    is_file: &dyn Fn(&Path) -> bool,
) -> Option<PathBuf> {
    if let Some(p) = env(SHELL_OVERRIDE_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
    {
        if is_file(&p) {
            return Some(p);
        }
        tracing::warn!(path = %p.display(), "{SHELL_OVERRIDE_ENV} is not a file; ignoring");
    }

    let path_dirs: Vec<PathBuf> = env("PATH")
        .map(|v| {
            v.to_string_lossy()
                .split(';')
                .map(|s| s.trim().trim_matches('"'))
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .filter(|p| is_windows_absolute(p))
                .collect()
        })
        .unwrap_or_default();

    if let Some(sh) = path_dirs
        .iter()
        .map(|d| d.join("sh.exe"))
        .find(|p| is_file(p))
    {
        return Some(sh);
    }

    // git.exe lives at <root>\cmd, <root>\bin or <root>\mingw64\bin.
    for git in path_dirs
        .iter()
        .map(|d| d.join("git.exe"))
        .filter(|p| is_file(p))
    {
        for root in git.ancestors().skip(2).take(3) {
            let sh = git_launcher(root);
            if is_file(&sh) {
                return Some(sh);
            }
        }
    }

    let program_roots = ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"]
        .into_iter()
        .filter_map(|k| env(k).map(PathBuf::from));
    let per_user = env("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Programs"));
    program_roots
        .chain(per_user)
        .map(|base| git_launcher(&base.join("Git")))
        .find(|p| is_file(p))
}

/// Git for Windows' `sh` launcher under an install root.
#[cfg_attr(not(windows), allow(dead_code))]
fn git_launcher(root: &Path) -> PathBuf {
    root.join("bin").join("sh.exe")
}

/// `C:\…` / `\\server\…`. Checked by hand rather than `Path::is_absolute`
/// so the rule is the same when the tests run on a Unix CI host. A bare
/// `\foo` is drive-relative on Windows, so it doesn't count.
#[cfg_attr(not(windows), allow(dead_code))]
fn is_windows_absolute(p: &Path) -> bool {
    let s = p.to_string_lossy();
    let b = s.as_bytes();
    (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/'))
        || s.starts_with(r"\\")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    /// Paths are built with `join`, never backslash literals, so `ancestors()`
    /// splits them the same way on a Unix CI host as on Windows.
    fn git_root() -> PathBuf {
        PathBuf::from("C:/").join("Program Files").join("Git")
    }

    fn run(env: &[(&str, String)], files: &[PathBuf]) -> Option<PathBuf> {
        let env: HashMap<String, OsString> = env
            .iter()
            .map(|(k, v)| (k.to_string(), OsString::from(v)))
            .collect();
        let files: HashSet<PathBuf> = files.iter().cloned().collect();
        resolve_windows(&|k| env.get(k).cloned(), &|p| files.contains(p))
    }

    fn path_of(dirs: &[PathBuf]) -> String {
        dirs.iter()
            .map(|d| d.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(";")
    }

    /// The reported bug: an Explorer-launched build sees only `Git\cmd` on
    /// PATH. Must resolve to the launcher, not fail and not `usr\bin\sh.exe`.
    #[test]
    fn git_cmd_only_on_path_resolves_to_bin_launcher() {
        let root = git_root();
        let files = [
            root.join("cmd").join("git.exe"),
            root.join("bin").join("sh.exe"),
            root.join("usr").join("bin").join("sh.exe"),
        ];
        let env = [(
            "PATH",
            path_of(&[PathBuf::from("C:/Windows/system32"), root.join("cmd")]),
        )];
        assert_eq!(run(&env, &files), Some(root.join("bin").join("sh.exe")));
    }

    #[test]
    fn git_on_path_via_mingw64_bin_resolves_to_install_root() {
        let root = PathBuf::from("D:/").join("tools").join("Git");
        let mingw = root.join("mingw64").join("bin");
        let files = [mingw.join("git.exe"), root.join("bin").join("sh.exe")];
        assert_eq!(
            run(&[("PATH", path_of(&[mingw]))], &files),
            Some(root.join("bin").join("sh.exe"))
        );
    }

    /// Today's working case (srv started from Git Bash) keeps its shell.
    #[test]
    fn sh_on_path_wins_over_git_derivation() {
        let root = git_root();
        let usr_bin = root.join("usr").join("bin");
        let files = [
            usr_bin.join("sh.exe"),
            root.join("cmd").join("git.exe"),
            root.join("bin").join("sh.exe"),
        ];
        let env = [("PATH", path_of(&[usr_bin.clone(), root.join("cmd")]))];
        assert_eq!(run(&env, &files), Some(usr_bin.join("sh.exe")));
    }

    /// WSL's System32\bash.exe must never be picked up by the PATH search.
    #[test]
    fn wsl_bash_on_path_is_not_used() {
        let sys32 = PathBuf::from("C:/").join("Windows").join("System32");
        let files = [sys32.join("bash.exe")];
        assert_eq!(run(&[("PATH", path_of(&[sys32]))], &files), None);
    }

    #[test]
    fn falls_back_to_program_files_when_git_not_on_path() {
        let pf = PathBuf::from("C:/").join("Program Files");
        let files = [pf.join("Git").join("bin").join("sh.exe")];
        let env = [
            ("PATH", String::from("C:/Windows")),
            ("ProgramFiles", pf.to_string_lossy().into_owned()),
        ];
        assert_eq!(
            run(&env, &files),
            Some(pf.join("Git").join("bin").join("sh.exe"))
        );
    }

    #[test]
    fn falls_back_to_per_user_install() {
        let local = PathBuf::from("C:/")
            .join("Users")
            .join("u")
            .join("AppData")
            .join("Local");
        let sh = local
            .join("Programs")
            .join("Git")
            .join("bin")
            .join("sh.exe");
        let env = [("LOCALAPPDATA", local.to_string_lossy().into_owned())];
        assert_eq!(run(&env, &[sh.clone()]), Some(sh));
    }

    #[test]
    fn override_wins_when_it_exists_and_is_ignored_when_it_does_not() {
        let root = git_root();
        let custom = PathBuf::from("E:/")
            .join("msys64")
            .join("usr")
            .join("bin")
            .join("bash.exe");
        let launcher = root.join("bin").join("sh.exe");
        let path = ("PATH", path_of(&[root.join("cmd")]));
        let files = [
            custom.clone(),
            root.join("cmd").join("git.exe"),
            launcher.clone(),
        ];
        let env = [
            (SHELL_OVERRIDE_ENV, custom.to_string_lossy().into_owned()),
            path.clone(),
        ];
        assert_eq!(run(&env, &files), Some(custom.clone()));

        // A stale override falls through to the normal search.
        let files = [root.join("cmd").join("git.exe"), launcher.clone()];
        assert_eq!(run(&env, &files), Some(launcher));
    }

    /// A relative PATH entry (`.`) must not let srv's cwd choose the shell.
    #[test]
    fn relative_path_entries_are_skipped() {
        let files = [PathBuf::from(".").join("sh.exe")];
        assert_eq!(run(&[("PATH", String::from("."))], &files), None);
    }

    #[test]
    fn quoted_path_entries_are_honoured() {
        let root = git_root();
        let files = [
            root.join("cmd").join("git.exe"),
            root.join("bin").join("sh.exe"),
        ];
        let env = [(
            "PATH",
            format!("\"{}\"", root.join("cmd").to_string_lossy()),
        )];
        assert_eq!(run(&env, &files), Some(root.join("bin").join("sh.exe")));
    }

    #[test]
    fn nothing_found_is_none() {
        assert_eq!(run(&[("PATH", String::from("C:/Windows"))], &[]), None);
    }
}
