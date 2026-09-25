// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Plain `gh` inside an agent process must not act as a human's `gh` login.
//!
//! Agents reach GitHub through `gh-agent`, which hands `gh` a per-call token of
//! the agent's own. The `gh` CLI, run directly, instead reads whatever login a
//! human made on the host — so an agent that types `gh` where it meant
//! `gh-agent` silently acts as that human, with nothing failing. `gh` finds its
//! accounts through `$GH_CONFIG_DIR/hosts.yml`; pointing that variable at a
//! directory with no `hosts.yml` leaves `gh` with no accounts, so it reports
//! "not logged in" instead. `gh-agent` is unaffected: it passes its own token.
//!
//! [`apply_gh_guard`] is called on every path that spawns an agent process or
//! runs a command on an agent's behalf. It is a RESERVED variable: it always
//! overwrites whatever the env already carries (a persisted `cmd:env`, a
//! caller's override), because a single user-supplied value would re-open the
//! hole. Containers keep it on `CONTAINER_ENV_DENYLIST` deliberately — the
//! host path means nothing inside the image.
//!
//! Design: `a5af/shared-infrastructure` `SPEC_AGENT_PLAIN_GH_GUARD_2026_09_25`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const GH_CONFIG_DIR: &str = "GH_CONFIG_DIR";

/// The file `gh` records its accounts in. Its absence is what "logged out"
/// means; nothing else in the directory is touched.
const GH_HOSTS_FILE: &str = "hosts.yml";

/// Directory name for an env with no agent identity on it. Still guarded —
/// an unattributed spawn is not a reason to fall back to the human's login.
const UNATTRIBUTED: &str = "unattributed";

/// `<config_home>/gh-<slug>`, the same location `agent.open` has always used.
///
/// The slug is reduced to `[a-z0-9_-]` so it can never name a path outside
/// `config_home` — this directory is one [`clear_gh_login`] deletes from.
pub fn agent_gh_config_dir(config_home: &Path, agent_slug: Option<&str>) -> PathBuf {
    let slug: String = agent_slug
        .unwrap_or("")
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    let slug = slug.trim_matches('-');
    let slug = if slug.is_empty() { UNATTRIBUTED } else { slug };
    config_home.join(format!("gh-{slug}"))
}

/// Create `dir` if needed and remove a `hosts.yml` from it.
///
/// A login can land here: `gh`'s own "not logged in" error tells the reader to
/// run `gh auth login`, and doing that inside an agent writes exactly this
/// file. Clearing it on every spawn is what keeps the guard from being undone
/// by one well-meant login. Only that one file is removed; `gh`'s preferences
/// (`config.yml`) are left alone.
pub fn clear_gh_login(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let hosts = dir.join(GH_HOSTS_FILE);
    match std::fs::symlink_metadata(&hosts) {
        // A file or a symlink: remove the entry itself, never a link's target.
        Ok(meta) if !meta.is_dir() => std::fs::remove_file(&hosts),
        Ok(_) => Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("{} is a directory, not a gh login file", hosts.display()),
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Point `GH_CONFIG_DIR` at this agent's login-free directory, under
/// `config_home`. The agent is read from the env itself
/// (`AGENTMUX_AGENT_SLUG`, then `AGENTMUX_AGENT_ID`).
///
/// Never fails the spawn: if the directory can't be prepared the variable is
/// still set, and a missing directory is just as logged-out as an empty one.
pub fn apply_gh_guard_in(env_vars: &mut HashMap<String, String>, config_home: &Path) {
    let slug = ["AGENTMUX_AGENT_SLUG", "AGENTMUX_AGENT_ID"]
        .iter()
        .filter_map(|k| env_vars.get(*k))
        .map(|v| v.trim())
        .find(|v| !v.is_empty())
        .map(str::to_string);
    let dir = agent_gh_config_dir(config_home, slug.as_deref());
    if let Err(e) = clear_gh_login(&dir) {
        tracing::warn!(dir = %dir.display(), error = %e, "gh guard: could not clear the agent's gh login dir");
    }
    env_vars.insert(GH_CONFIG_DIR.to_string(), dir.to_string_lossy().into_owned());
}

/// [`apply_gh_guard_in`] under this instance's config dir.
pub fn apply_gh_guard(env_vars: &mut HashMap<String, String>) {
    apply_gh_guard_in(env_vars, &guard_config_home());
}

/// Where guard dirs live. Under test, a per-process temp dir: every test that
/// builds a spawn env reaches this, and none of them should create or clear
/// anything in the developer's real config home.
pub fn guard_config_home() -> PathBuf {
    #[cfg(test)]
    {
        std::env::temp_dir().join(format!("agentmux-gh-guard-test-{}", std::process::id()))
    }
    #[cfg(not(test))]
    {
        crate::backend::base::get_mux_config_dir()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn sets_a_per_agent_dir_under_config_home() {
        let home = tempfile::tempdir().unwrap();
        let mut e = env(&[("AGENTMUX_AGENT_SLUG", "agenty")]);
        apply_gh_guard_in(&mut e, home.path());
        let dir = PathBuf::from(&e[GH_CONFIG_DIR]);
        assert_eq!(dir, home.path().join("gh-agenty"));
        assert!(dir.is_dir(), "the dir is created so gh has somewhere empty to look");
    }

    #[test]
    fn overrides_a_gh_config_dir_already_in_the_env() {
        // The hole this closes: a persisted or caller-supplied value pointing
        // at a real login must not survive into the spawn.
        let home = tempfile::tempdir().unwrap();
        let mut e = env(&[("AGENTMUX_AGENT_ID", "AgentY"), (GH_CONFIG_DIR, "/home/human/.config/gh")]);
        apply_gh_guard_in(&mut e, home.path());
        assert_eq!(PathBuf::from(&e[GH_CONFIG_DIR]), home.path().join("gh-agenty"));
    }

    #[test]
    fn prefers_the_slug_and_falls_back_to_the_agent_id() {
        let home = tempfile::tempdir().unwrap();
        let mut e = env(&[("AGENTMUX_AGENT_ID", "Display Name"), ("AGENTMUX_AGENT_SLUG", "korp")]);
        apply_gh_guard_in(&mut e, home.path());
        assert!(e[GH_CONFIG_DIR].ends_with("gh-korp"));

        let mut e = env(&[("AGENTMUX_AGENT_ID", "Korp"), ("AGENTMUX_AGENT_SLUG", "  ")]);
        apply_gh_guard_in(&mut e, home.path());
        assert!(e[GH_CONFIG_DIR].ends_with("gh-korp"));
    }

    #[test]
    fn guards_an_unattributed_spawn_rather_than_skipping_it() {
        let home = tempfile::tempdir().unwrap();
        let mut e = HashMap::new();
        apply_gh_guard_in(&mut e, home.path());
        assert_eq!(PathBuf::from(&e[GH_CONFIG_DIR]), home.path().join("gh-unattributed"));
    }

    #[test]
    fn a_slug_cannot_escape_config_home() {
        let home = Path::new("/cfg");
        for bad in ["../../etc", "..", "a/b", "a\\b", "C:\\x", "/abs"] {
            let dir = agent_gh_config_dir(home, Some(bad));
            assert_eq!(dir.parent(), Some(home), "{bad:?} -> {}", dir.display());
            let name = dir.file_name().unwrap().to_string_lossy().into_owned();
            assert!(name.starts_with("gh-") && !name.contains(['/', '\\', '.']), "{bad:?} -> {name}");
        }
    }

    #[test]
    fn clears_a_login_left_in_the_dir_and_keeps_preferences() {
        let home = tempfile::tempdir().unwrap();
        let dir = agent_gh_config_dir(home.path(), Some("agenty"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("hosts.yml"), "github.com:\n  user: someone\n").unwrap();
        std::fs::write(dir.join("config.yml"), "editor: vim\n").unwrap();

        let mut e = env(&[("AGENTMUX_AGENT_SLUG", "agenty")]);
        apply_gh_guard_in(&mut e, home.path());

        assert!(!dir.join("hosts.yml").exists(), "a login in the guard dir must not survive a spawn");
        assert!(dir.join("config.yml").exists(), "gh preferences are not a login and are left alone");
    }

    #[test]
    fn a_dir_named_hosts_yml_is_refused_not_recursed_into() {
        let home = tempfile::tempdir().unwrap();
        let dir = agent_gh_config_dir(home.path(), Some("agenty"));
        std::fs::create_dir_all(dir.join("hosts.yml")).unwrap();
        assert!(clear_gh_login(&dir).is_err());
        // The variable is still set: the spawn is not failed over this.
        let mut e = env(&[("AGENTMUX_AGENT_SLUG", "agenty")]);
        apply_gh_guard_in(&mut e, home.path());
        assert_eq!(PathBuf::from(&e[GH_CONFIG_DIR]), dir);
    }
}
