// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Put `gh-agent`'s publish guard in front of every `git push` an agent makes.
//!
//! `gh-agent` writes a git hooks directory whose `pre-push` scans what a push
//! to a public repository would publish and refuses it on a hit; the other
//! hooks in it run the repository's own hook of that name. Git only uses it if
//! `core.hooksPath` points there, and a repository's own `core.hooksPath` (this
//! repo sets `.githooks`) beats a global one. Configuration from the
//! environment (`GIT_CONFIG_COUNT`, `GIT_CONFIG_KEY_<n>`, `GIT_CONFIG_VALUE_<n>`)
//! beats both, so that is how agents get it.
//!
//! The directory is computed the way `gh-agent hooks-path` computes it, from
//! the agent's own env first and then this process's, rather than by running
//! `gh-agent` on every spawn. It is only used once `gh-agent` has written it
//! (any `gh-agent` 0.5+ run as git's credential helper does); an agent started
//! before that gets it at its next start. Without it the spawn goes ahead
//! unchanged: the guard is an extra check, never a reason to fail.
//!
//! Like `GH_CONFIG_DIR` from `backend::gh_guard`, the entry is reserved: a value
//! for `core.hooksPath` already in the env is replaced, not kept.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Once;

const COUNT: &str = "GIT_CONFIG_COUNT";
const HOOKS_PATH_KEY: &str = "core.hooksPath";
/// Larger counts already in the env are not trusted (and not looped over).
const MAX_ENTRIES: usize = 256;
/// The line `gh-agent` writes into every hook it generates.
const MARKER: &str = "gh-agent publish guard";

/// Block meta on an agent's PtyShell: the hooks directory, which the PTY spawn
/// sets through `GIT_CONFIG_*` (`blockcontroller::shell::lifecycle`). A PTY
/// shell's env is built from its own block, not from the agent's, so the
/// setting travels on that block, as `gh_guard`'s directory does.
pub const META_KEY_PTYSHELL_GIT_HOOKS_PATH: &str = "ptyshell:githookspath";

/// Where `gh-agent` writes its hooks: `GH_AGENT_HOOKS_DIR`, else
/// `%LOCALAPPDATA%\gh-agent-hooks` on Windows and
/// `$XDG_DATA_HOME/gh-agent/hooks` (default `~/.local/share`) elsewhere.
pub fn gh_agent_hooks_dir(get: impl Fn(&str) -> Option<String>, home: &Path, windows: bool) -> PathBuf {
    let get = |k: &str| get(k).filter(|v| !v.is_empty());
    if let Some(dir) = get("GH_AGENT_HOOKS_DIR") {
        return PathBuf::from(dir);
    }
    if windows {
        let base = get("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| home.join("AppData").join("Local"));
        return base.join("gh-agent-hooks");
    }
    let base = get("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local").join("share"));
    base.join("gh-agent").join("hooks")
}

/// Whether `dir` holds hooks `gh-agent` wrote.
pub fn is_guard_hooks_dir(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join("pre-push")).map(|s| s.contains(MARKER)).unwrap_or(false)
}

/// The variables that set `key = value` on top of the `GIT_CONFIG_*` entries
/// `get` already sees: the slot that names `key` (case-insensitively, as git
/// does for the section and name) is reused, otherwise one is appended.
pub fn git_config_entries(get: impl Fn(&str) -> Option<String>, key: &str, value: &str) -> Vec<(String, String)> {
    let count: usize = get(COUNT)
        .and_then(|c| c.trim().parse().ok())
        .filter(|c: &usize| *c <= MAX_ENTRIES)
        .unwrap_or(0);
    for i in 0..count {
        if get(&format!("GIT_CONFIG_KEY_{i}")).is_some_and(|k| k.eq_ignore_ascii_case(key)) {
            return vec![(format!("GIT_CONFIG_VALUE_{i}"), value.to_string()), (COUNT.to_string(), count.to_string())];
        }
    }
    vec![
        (format!("GIT_CONFIG_KEY_{count}"), key.to_string()),
        (format!("GIT_CONFIG_VALUE_{count}"), value.to_string()),
        (COUNT.to_string(), (count + 1).to_string()),
    ]
}

/// The hooks directory for a spawn whose own env is `env` (falling back to
/// `process` for anything it doesn't set), if `gh-agent` has written it.
pub fn hooks_dir_for(env: &HashMap<String, String>, process: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let get = |k: &str| env.get(k).cloned().or_else(|| process(k));
    let home = get(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
        .unwrap_or_default();
    let dir = gh_agent_hooks_dir(&get, &home, cfg!(windows));
    is_guard_hooks_dir(&dir).then_some(dir)
}

/// Point `core.hooksPath` at `hooks_dir` in `env`, an overlay on `process`.
pub fn apply_publish_guard_in(env: &mut HashMap<String, String>, hooks_dir: &Path, process: impl Fn(&str) -> Option<String>) {
    let entries = git_config_entries(|k| env.get(k).cloned().or_else(|| process(k)), HOOKS_PATH_KEY, &hooks_dir.to_string_lossy());
    env.extend(entries);
}

/// [`apply_publish_guard_in`] with the hooks directory this spawn's env and
/// this host give. Not called under test (see `gh_guard::apply_gh_guard`), so
/// tests never read the host's real hooks.
#[cfg_attr(test, allow(dead_code))]
pub fn apply_publish_guard(env: &mut HashMap<String, String>) {
    let process = |k: &str| std::env::var(k).ok();
    match hooks_dir_for(env, process) {
        Some(dir) => apply_publish_guard_in(env, &dir, process),
        None => {
            static WARN: Once = Once::new();
            WARN.call_once(|| {
                tracing::warn!(
                    "publish guard: gh-agent's hooks are not written, so agents' pushes are not scanned; \
                     install gh-agent 0.5 or later and run `gh-agent setup-git`, then restart agents"
                );
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn no_process(_: &str) -> Option<String> {
        None
    }

    fn guard_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pre-push"), format!("#!/bin/sh\n# {MARKER}: test\n")).unwrap();
        dir
    }

    #[test]
    fn hooks_dir_follows_gh_agent() {
        let home = Path::new("/home/a");
        let none = |_: &str| None;
        assert_eq!(gh_agent_hooks_dir(none, home, false), PathBuf::from("/home/a/.local/share/gh-agent/hooks"));
        assert_eq!(
            gh_agent_hooks_dir(|k| (k == "XDG_DATA_HOME").then(|| "/x".to_string()), home, false),
            PathBuf::from("/x/gh-agent/hooks")
        );
        assert_eq!(
            gh_agent_hooks_dir(|k| (k == "LOCALAPPDATA").then(|| "C:/L".to_string()), home, true),
            PathBuf::from("C:/L/gh-agent-hooks")
        );
        assert_eq!(gh_agent_hooks_dir(none, home, true), PathBuf::from("/home/a/AppData/Local/gh-agent-hooks"));
        assert_eq!(
            gh_agent_hooks_dir(|k| (k == "GH_AGENT_HOOKS_DIR").then(|| "/o".to_string()), home, true),
            PathBuf::from("/o")
        );
    }

    #[test]
    fn the_agents_own_env_decides_where_the_hooks_are() {
        let dir = guard_dir();
        let agent = env(&[("GH_AGENT_HOOKS_DIR", &dir.path().to_string_lossy())]);
        assert_eq!(hooks_dir_for(&agent, no_process).as_deref(), Some(dir.path()));
        // the process env is the fallback
        let p = dir.path().to_string_lossy().into_owned();
        assert_eq!(hooks_dir_for(&HashMap::new(), |k| (k == "GH_AGENT_HOOKS_DIR").then(|| p.clone())).as_deref(), Some(dir.path()));
        // not written yet, or written by something else: nothing
        let empty = tempfile::tempdir().unwrap();
        let agent = env(&[("GH_AGENT_HOOKS_DIR", &empty.path().to_string_lossy())]);
        assert_eq!(hooks_dir_for(&agent, no_process), None);
        std::fs::write(empty.path().join("pre-push"), "#!/bin/sh\n# someone else's hook\n").unwrap();
        assert_eq!(hooks_dir_for(&agent, no_process), None);
    }

    #[test]
    fn sets_core_hooks_path_through_the_environment() {
        let dir = guard_dir();
        let mut e = env(&[("AGENTMUX_AGENT_ID", "AgentY")]);
        apply_publish_guard_in(&mut e, dir.path(), no_process);
        assert_eq!(e[COUNT], "1");
        assert_eq!(e["GIT_CONFIG_KEY_0"], HOOKS_PATH_KEY);
        assert_eq!(PathBuf::from(&e["GIT_CONFIG_VALUE_0"]), dir.path());
    }

    #[test]
    fn appends_after_entries_already_there_and_replaces_its_own() {
        let dir = guard_dir();
        let mut e = env(&[(COUNT, "1"), ("GIT_CONFIG_KEY_0", "user.name"), ("GIT_CONFIG_VALUE_0", "x")]);
        apply_publish_guard_in(&mut e, dir.path(), no_process);
        assert_eq!(e[COUNT], "2");
        assert_eq!(e["GIT_CONFIG_KEY_0"], "user.name");
        assert_eq!(e["GIT_CONFIG_KEY_1"], HOOKS_PATH_KEY);
        // applied again (a persisted cmd:env carrying an old value): same slot, no growth
        e.insert("GIT_CONFIG_VALUE_1".into(), "/somewhere/else".into());
        apply_publish_guard_in(&mut e, dir.path(), no_process);
        assert_eq!(e[COUNT], "2");
        assert_eq!(PathBuf::from(&e["GIT_CONFIG_VALUE_1"]), dir.path());
    }

    #[test]
    fn a_hooks_path_entry_from_elsewhere_is_replaced_not_kept() {
        let dir = guard_dir();
        let mut e = env(&[(COUNT, "1"), ("GIT_CONFIG_KEY_0", "core.hookspath"), ("GIT_CONFIG_VALUE_0", "/dev/null")]);
        apply_publish_guard_in(&mut e, dir.path(), no_process);
        assert_eq!(e[COUNT], "1");
        assert_eq!(PathBuf::from(&e["GIT_CONFIG_VALUE_0"]), dir.path());
    }

    #[test]
    fn builds_on_entries_the_process_env_already_has() {
        let dir = guard_dir();
        let mut e = HashMap::new();
        let process = |k: &str| match k {
            "GIT_CONFIG_COUNT" => Some("2".to_string()),
            "GIT_CONFIG_KEY_1" => Some("core.hooksPath".to_string()),
            _ => None,
        };
        apply_publish_guard_in(&mut e, dir.path(), process);
        assert_eq!(e[COUNT], "2", "reuses the process env's own hooksPath slot");
        assert_eq!(PathBuf::from(&e["GIT_CONFIG_VALUE_1"]), dir.path());
        assert!(!e.contains_key("GIT_CONFIG_KEY_0"), "the other inherited slot is left alone");
    }

    #[test]
    fn an_absurd_count_is_not_trusted_or_looped_over() {
        let dir = guard_dir();
        let mut e = env(&[(COUNT, "999999999999")]);
        apply_publish_guard_in(&mut e, dir.path(), no_process);
        assert_eq!(e[COUNT], "1");
        assert_eq!(e["GIT_CONFIG_KEY_0"], HOOKS_PATH_KEY);
    }

    #[test]
    fn entries_for_a_pty_spawn_keep_the_slots_its_env_already_uses() {
        // what a PTY's CommandBuilder already holds from cmd:env
        let existing = env(&[(COUNT, "1"), ("GIT_CONFIG_KEY_0", "url.x.insteadOf"), ("GIT_CONFIG_VALUE_0", "y")]);
        let pairs = git_config_entries(|k| existing.get(k).cloned(), HOOKS_PATH_KEY, "/h");
        assert!(pairs.contains(&("GIT_CONFIG_KEY_1".to_string(), HOOKS_PATH_KEY.to_string())));
        assert!(pairs.contains(&("GIT_CONFIG_VALUE_1".to_string(), "/h".to_string())));
        assert!(pairs.contains(&(COUNT.to_string(), "2".to_string())));
    }
}
