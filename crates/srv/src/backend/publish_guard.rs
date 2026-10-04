// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Put `gh-agent`'s publish guard in front of every `git push` an agent makes.
//!
//! `gh-agent` writes a git hooks directory whose `pre-push` scans what a push
//! to a public repository would publish and refuses it on a hit; every other
//! hook in it runs the repository's own hook of that name. Git only uses it if
//! `core.hooksPath` points there, and a repository's own `core.hooksPath` (this
//! repo sets `.githooks`) beats a global one. Configuration from the
//! environment (`GIT_CONFIG_COUNT`, `GIT_CONFIG_KEY_<n>`, `GIT_CONFIG_VALUE_<n>`)
//! beats both, so that is how agents get it.
//!
//! The directory is computed the way `gh-agent hooks-path` computes it rather
//! than by running `gh-agent` on every spawn, and it is only used once
//! `gh-agent` has written it (`gh-agent setup-git` does). Without it the spawn
//! goes ahead unchanged: the guard is an extra check, never a reason to fail.
//!
//! Like `GH_CONFIG_DIR` from `backend::gh_guard`, the entry is reserved: a value
//! for `core.hooksPath` already in the env is replaced, not kept.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Once;

const COUNT: &str = "GIT_CONFIG_COUNT";
const HOOKS_PATH_KEY: &str = "core.hooksPath";
/// The line `gh-agent` writes into every hook it generates.
const MARKER: &str = "gh-agent publish guard";

/// Where `gh-agent` writes its hooks: `GH_AGENT_HOOKS_DIR`, else
/// `%LOCALAPPDATA%\gh-agent-hooks` on Windows and
/// `$XDG_DATA_HOME/gh-agent/hooks` (default `~/.local/share`) elsewhere.
pub fn gh_agent_hooks_dir(get: impl Fn(&str) -> Option<String>, home: &Path, windows: bool) -> PathBuf {
    if let Some(dir) = get("GH_AGENT_HOOKS_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    if windows {
        let base = get("LOCALAPPDATA")
            .filter(|d| !d.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData").join("Local"));
        return base.join("gh-agent-hooks");
    }
    let base = get("XDG_DATA_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local").join("share"));
    base.join("gh-agent").join("hooks")
}

/// Whether `dir` holds hooks `gh-agent` wrote.
pub fn is_guard_hooks_dir(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join("pre-push")).map(|s| s.contains(MARKER)).unwrap_or(false)
}

/// Set `key = value` through the `GIT_CONFIG_*` variables in `env`, reusing the
/// slot that already names `key` (case-insensitively, as git does for the
/// section and name) and otherwise appending after the entries already there.
/// `inherited` supplies the count when `env` overlays a process env that has one.
pub fn set_git_config_env(env: &mut HashMap<String, String>, inherited: Option<String>, key: &str, value: &str) {
    let count: usize = env
        .get(COUNT)
        .cloned()
        .or(inherited)
        .and_then(|c| c.trim().parse().ok())
        .unwrap_or(0);
    for i in 0..count {
        if env.get(&format!("GIT_CONFIG_KEY_{i}")).is_some_and(|k| k.eq_ignore_ascii_case(key)) {
            env.insert(format!("GIT_CONFIG_VALUE_{i}"), value.to_string());
            env.insert(COUNT.to_string(), count.to_string());
            return;
        }
    }
    env.insert(format!("GIT_CONFIG_KEY_{count}"), key.to_string());
    env.insert(format!("GIT_CONFIG_VALUE_{count}"), value.to_string());
    env.insert(COUNT.to_string(), (count + 1).to_string());
}

/// Point `core.hooksPath` at `hooks_dir` if it holds `gh-agent`'s hooks.
/// Returns whether it did.
pub fn apply_publish_guard_in(env: &mut HashMap<String, String>, hooks_dir: &Path, inherited_count: Option<String>) -> bool {
    if !is_guard_hooks_dir(hooks_dir) {
        return false;
    }
    set_git_config_env(env, inherited_count, HOOKS_PATH_KEY, &hooks_dir.to_string_lossy());
    true
}

/// [`apply_publish_guard_in`] with this host's `gh-agent` hooks directory.
/// Not called under test (see `gh_guard::apply_gh_guard`), so tests never read the host's real hooks.
#[cfg_attr(test, allow(dead_code))]
pub fn apply_publish_guard(env: &mut HashMap<String, String>) {
    let dir = gh_agent_hooks_dir(|k| std::env::var(k).ok(), &dirs::home_dir().unwrap_or_default(), cfg!(windows));
    if !apply_publish_guard_in(env, &dir, std::env::var(COUNT).ok()) {
        static WARN: Once = Once::new();
        WARN.call_once(|| {
            tracing::warn!(
                dir = %dir.display(),
                "publish guard: gh-agent's hooks are not written, so agents' pushes are not scanned; run `gh-agent setup-git`"
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
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
    fn sets_core_hooks_path_through_the_environment() {
        let dir = guard_dir();
        let mut e = env(&[("AGENTMUX_AGENT_ID", "AgentY")]);
        assert!(apply_publish_guard_in(&mut e, dir.path(), None));
        assert_eq!(e[COUNT], "1");
        assert_eq!(e["GIT_CONFIG_KEY_0"], HOOKS_PATH_KEY);
        assert_eq!(PathBuf::from(&e["GIT_CONFIG_VALUE_0"]), dir.path());
    }

    #[test]
    fn appends_after_entries_already_there_and_replaces_its_own() {
        let dir = guard_dir();
        let mut e = env(&[(COUNT, "1"), ("GIT_CONFIG_KEY_0", "user.name"), ("GIT_CONFIG_VALUE_0", "x")]);
        apply_publish_guard_in(&mut e, dir.path(), None);
        assert_eq!(e[COUNT], "2");
        assert_eq!(e["GIT_CONFIG_KEY_0"], "user.name");
        assert_eq!(e["GIT_CONFIG_KEY_1"], HOOKS_PATH_KEY);
        // applied again (a persisted cmd:env carrying an old value): same slot, no growth
        e.insert("GIT_CONFIG_VALUE_1".into(), "/somewhere/else".into());
        apply_publish_guard_in(&mut e, dir.path(), None);
        assert_eq!(e[COUNT], "2");
        assert_eq!(PathBuf::from(&e["GIT_CONFIG_VALUE_1"]), dir.path());
    }

    #[test]
    fn a_hooks_path_entry_from_elsewhere_is_replaced_not_kept() {
        let dir = guard_dir();
        let mut e = env(&[(COUNT, "1"), ("GIT_CONFIG_KEY_0", "core.hookspath"), ("GIT_CONFIG_VALUE_0", "/dev/null")]);
        apply_publish_guard_in(&mut e, dir.path(), None);
        assert_eq!(e[COUNT], "1");
        assert_eq!(PathBuf::from(&e["GIT_CONFIG_VALUE_0"]), dir.path());
    }

    #[test]
    fn counts_from_the_inherited_env_when_the_overlay_has_none() {
        let dir = guard_dir();
        let mut e = HashMap::new();
        apply_publish_guard_in(&mut e, dir.path(), Some("2".into()));
        assert_eq!(e[COUNT], "3");
        assert_eq!(e["GIT_CONFIG_KEY_2"], HOOKS_PATH_KEY);
    }

    #[test]
    fn leaves_the_env_alone_until_gh_agent_has_written_its_hooks() {
        let empty = tempfile::tempdir().unwrap();
        let mut e = env(&[("AGENTMUX_AGENT_ID", "AgentY")]);
        assert!(!apply_publish_guard_in(&mut e, empty.path(), None));
        std::fs::write(empty.path().join("pre-push"), "#!/bin/sh\n# someone else's hook\n").unwrap();
        assert!(!apply_publish_guard_in(&mut e, empty.path(), None));
        assert!(!e.contains_key(COUNT));
    }
}
