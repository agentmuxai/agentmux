// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Plain `gh` inside an agent process must not act as a human's `gh` login, when
//! the agent has a GitHub identity of its own.
//!
//! An agent can reach GitHub through an identity of its own: `gh-agent`, which
//! hands `gh` a per-call token, or the AgentMux App's broker, which does the same
//! for `git` and `gh`. The `gh` CLI, run directly, instead reads whatever login a
//! human made on the host — so an agent that types `gh` where it meant its own
//! identity silently acts as that human, with nothing failing. `gh` finds its
//! accounts through `$GH_CONFIG_DIR/hosts.yml`; pointing that variable at a
//! directory with no `hosts.yml` leaves `gh` with no accounts, so it reports
//! "not logged in" instead. The agent's own identity is unaffected: it passes its
//! own token.
//!
//! An agent with no such identity has nothing to be protected into using: the
//! person's own `gh login` is the only GitHub access it can have, so it is left
//! alone ([`has_other_github_identity`]). A guard directory an earlier start set
//! is dropped then, so a person who logged in with `gh auth login` and runs agents
//! gets agents that can use `gh`.
//!
//! [`apply_gh_guard`] is called on every path that spawns an agent process or
//! runs a command on an agent's behalf. Where it applies, `GH_CONFIG_DIR` is a
//! RESERVED variable: it always overwrites whatever the env already carries (a
//! persisted `cmd:env`, a caller's override), because a single user-supplied
//! value would re-open the hole. Containers keep it on `CONTAINER_ENV_DENYLIST`
//! deliberately — the host path means nothing inside the image.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const GH_CONFIG_DIR: &str = "GH_CONFIG_DIR";

/// Block meta on an agent's PtyShell: the agent's guard directory, which the
/// PTY spawn sets as `GH_CONFIG_DIR` (`blockcontroller::shell::lifecycle`). A
/// PTY shell's env is built from its own block, not from the agent's, so the
/// guard has to travel on that block.
pub const META_KEY_PTYSHELL_GH_CONFIG_DIR: &str = "ptyshell:ghconfigdir";

/// The guard directory for an agent whose stored `cmd:env` is `agent_env`
/// (the slug is read from it, as for the agent's own spawns), prepared the
/// same way: created, with any login in it cleared. Empty when the agent is not
/// guarded (it has no other GitHub identity), so the shell sets nothing.
pub fn guard_dir_for(agent_env: &HashMap<String, String>) -> String {
    let home = guard_config_home();
    let mut env = agent_env.clone();
    apply_gh_guard_for(&mut env, &home, has_other_github_identity(agent_env));
    match env.remove(GH_CONFIG_DIR) {
        Some(dir) if is_own_guard_dir(&dir, &home) => dir,
        _ => String::new(),
    }
}

/// Whether `dir` is one of the guard directories [`agent_gh_config_dir`] makes
/// under `config_home`, as opposed to a `GH_CONFIG_DIR` the person or their
/// agent's definition chose.
fn is_own_guard_dir(dir: &str, config_home: &Path) -> bool {
    let dir = Path::new(dir);
    dir.parent() == Some(config_home) && dir.file_name().is_some_and(|n| n.to_string_lossy().starts_with("gh-"))
}

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

/// Guard the agent when it has another GitHub identity; otherwise leave `gh` to
/// the person's own login, and drop a guard directory an earlier start put in the
/// env (a `GH_CONFIG_DIR` of the person's own choosing stays).
pub fn apply_gh_guard_for(env_vars: &mut HashMap<String, String>, config_home: &Path, other_identity: bool) {
    if other_identity {
        apply_gh_guard_in(env_vars, config_home);
    } else if env_vars.get(GH_CONFIG_DIR).is_some_and(|dir| is_own_guard_dir(dir, config_home)) {
        env_vars.remove(GH_CONFIG_DIR);
    }
}

/// Whether the agent whose spawn env is `env` has a GitHub identity besides the
/// person's `gh` login: `gh-agent` on this machine (its hooks written, or the
/// command on the PATH), or the AgentMux App's broker. Only then does
/// [`apply_gh_guard`] keep the person's login from it.
pub fn has_other_github_identity(env: &HashMap<String, String>) -> bool {
    // Under test every spawn is guarded, as every test of a spawn env expects.
    #[cfg(test)]
    {
        let _ = env;
        true
    }
    #[cfg(not(test))]
    {
        crate::backend::publish_guard::hooks_dir_for(env, |k| std::env::var(k).ok()).is_some()
            || gh_agent_on_path(env)
            || crate::identity::github_tokens::agents_use_broker(env)
    }
}

/// Whether a `gh-agent` command is on the PATH the spawn gets (its own, else this process's).
#[cfg_attr(test, allow(dead_code))]
fn gh_agent_on_path(env: &HashMap<String, String>) -> bool {
    let path = env
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("PATH"))
        .map(|(_, v)| std::ffi::OsString::from(v))
        .or_else(|| std::env::var_os("PATH"))
        .unwrap_or_default();
    let names: &[&str] = if cfg!(windows) { &["gh-agent.cmd", "gh-agent.exe", "gh-agent.bat"] } else { &["gh-agent"] };
    std::env::split_paths(&path).any(|dir| names.iter().any(|name| dir.join(name).is_file()))
}

/// [`apply_gh_guard_for`] under this instance's config dir, plus the publish
/// guard's `core.hooksPath` ([`crate::backend::publish_guard`]): every path that
/// spawns an agent process or runs a command for one comes through here.
pub fn apply_gh_guard(env_vars: &mut HashMap<String, String>) {
    let other_identity = has_other_github_identity(env_vars);
    apply_gh_guard_for(env_vars, &guard_config_home(), other_identity);
    #[cfg(not(test))]
    {
        crate::backend::publish_guard::apply_publish_guard(env_vars);
        crate::identity::github_tokens::apply_agent_git(env_vars);
    }
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

    /// A PtyShell's guard is the agent's own directory, the one its Shell and
    /// its CLI get, read from the agent's stored env.
    #[test]
    fn a_ptyshell_gets_the_agents_own_guard_dir() {
        let dir = guard_dir_for(&env(&[("AGENTMUX_AGENT_ID", "PtyGuardTest")]));
        assert_eq!(
            PathBuf::from(&dir),
            agent_gh_config_dir(&guard_config_home(), Some("ptyguardtest"))
        );
        let mut shell_env = env(&[("AGENTMUX_AGENT_ID", "PtyGuardTest")]);
        apply_gh_guard(&mut shell_env);
        assert_eq!(shell_env.get(GH_CONFIG_DIR), Some(&dir), "same as the agent's Shell");
        assert!(!guard_dir_for(&HashMap::new()).is_empty(), "unattributed is still guarded");
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
    fn an_agent_with_no_other_github_identity_keeps_the_persons_gh_login() {
        // The person ran `gh auth login` and has no gh-agent and no connected account: the guard
        // must neither point gh at an empty folder nor delete a login in it.
        let home = tempfile::tempdir().unwrap();
        let dir = agent_gh_config_dir(home.path(), Some("agenty"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("hosts.yml"), "github.com:\n  user: someone\n").unwrap();
        let mut e = env(&[("AGENTMUX_AGENT_SLUG", "agenty")]);
        apply_gh_guard_for(&mut e, home.path(), false);
        assert!(!e.contains_key(GH_CONFIG_DIR), "gh finds the person's own login");
        assert!(dir.join("hosts.yml").exists(), "nothing is cleared for an unguarded agent");
    }

    #[test]
    fn an_unguarded_agent_drops_a_guard_dir_an_earlier_start_left_but_not_a_dir_of_the_persons_choosing() {
        let home = tempfile::tempdir().unwrap();
        let ours = agent_gh_config_dir(home.path(), Some("agenty")).to_string_lossy().into_owned();
        let mut e = env(&[("AGENTMUX_AGENT_SLUG", "agenty"), (GH_CONFIG_DIR, &ours)]);
        apply_gh_guard_for(&mut e, home.path(), false);
        assert!(!e.contains_key(GH_CONFIG_DIR));

        let mut e = env(&[("AGENTMUX_AGENT_SLUG", "agenty"), (GH_CONFIG_DIR, "/home/person/.config/gh")]);
        apply_gh_guard_for(&mut e, home.path(), false);
        assert_eq!(e[GH_CONFIG_DIR], "/home/person/.config/gh");
    }

    #[test]
    fn a_guarded_agent_is_set_up_as_before() {
        let home = tempfile::tempdir().unwrap();
        let mut e = env(&[("AGENTMUX_AGENT_SLUG", "agenty")]);
        apply_gh_guard_for(&mut e, home.path(), true);
        assert_eq!(PathBuf::from(&e[GH_CONFIG_DIR]), home.path().join("gh-agenty"));
    }

    #[test]
    fn gh_agent_on_the_path_counts_as_another_identity() {
        let bin = tempfile::tempdir().unwrap();
        let name = if cfg!(windows) { "gh-agent.cmd" } else { "gh-agent" };
        let path = |dirs: &[&Path]| std::env::join_paths(dirs.iter().copied()).unwrap().to_string_lossy().into_owned();
        let empty = tempfile::tempdir().unwrap();
        assert!(!gh_agent_on_path(&env(&[("PATH", &path(&[empty.path()]))])));
        std::fs::write(bin.path().join(name), "").unwrap();
        assert!(gh_agent_on_path(&env(&[("PATH", &path(&[empty.path(), bin.path()]))])));
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
