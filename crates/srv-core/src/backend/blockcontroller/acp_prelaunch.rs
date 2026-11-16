// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Files two ACP agents need in their own config dirs before they start.
//! SPEC_NINE_NEW_HARNESSES_2026_10_14.md §7.5 (Grok Build), §7.6 (DeepSeek Harness).

use std::collections::HashMap;
use std::path::Path;

/// Run before an ACP agent spawns, with its env and absolute session cwd. A
/// failure is logged and the launch goes on: the agent still runs, only
/// without what the file gives it.
pub(super) fn prepare(provider: &str, env: &HashMap<String, String>, cwd: &str) {
    let result = match provider {
        "grok" => env.get("GROK_HOME").map(|home| trust_grok_folder(Path::new(home), cwd)),
        "dsh" => env.get("DSH_HOME").map(|home| seed_dsh_patch(Path::new(home))),
        _ => None,
    };
    if let Some(Err(e)) = result {
        tracing::warn!(provider, error = %e, "[acp] pre-launch file not written");
    }
}

/// Run on an ACP agent's spawn env before it spawns. DeepSeek Harness drops every variable whose name
/// holds `KEY`, `TOKEN`, `SECRET` or `PASSWORD` from the shells it starts, which would split git's
/// `GIT_CONFIG_*` list, so its git config is handed over as `GIT_CONFIG_PARAMETERS`
/// (`publish_guard::git_config_as_parameters`).
pub(super) fn prepare_env(provider: &str, env: &mut HashMap<String, String>) {
    if provider == "dsh" {
        crate::backend::publish_guard::git_config_as_parameters(env, |k| std::env::var(k).ok());
    }
}

/// The Node.js DeepSeek Harness needs (its package's `engines`). The install flow checks the same
/// minimum before the first launch (`systemPrereqs` in `frontend/app/view/agent/providers/catalog.ts`);
/// an agent that is already set up is not run through that, so a restart on an older Node would end in
/// dsh's own startup error. Keep the two in step.
const DSH_MIN_NODE: [u64; 3] = [24, 9, 0];

/// `major.minor.patch` of a `node --version` answer (`v24.19.0`, `24.19.0`); a missing part is 0.
fn version_parts(text: &str) -> Option<[u64; 3]> {
    let mut parts = [0u64; 3];
    let mut numbers = text.trim().trim_start_matches('v').split('.');
    parts[0] = numbers.next()?.trim().parse().ok()?;
    for part in parts.iter_mut().skip(1) {
        match numbers.next() {
            Some(n) => *part = n.split(|c: char| !c.is_ascii_digit()).next().unwrap_or("").parse().ok()?,
            None => break,
        }
    }
    Some(parts)
}

/// What to tell the person when `version` is older than DeepSeek Harness needs; `None` when it is
/// new enough or cannot be read (dsh then says what it makes of it).
fn node_too_old(version: &str) -> Option<String> {
    let found = version_parts(version)?;
    (found < DSH_MIN_NODE).then(|| {
        let [major, minor, patch] = DSH_MIN_NODE;
        format!(
            "DeepSeek Harness needs Node.js {major}.{minor}.{patch} or later, and the Node.js on this agent's PATH is {}. Update Node.js (https://nodejs.org/), then start the agent again.",
            version.trim().trim_start_matches('v')
        )
    })
}

/// Refuse to start DeepSeek Harness on a Node.js older than it needs, with the reason, instead of
/// letting it fail with its own startup error. Every other provider, and a machine with no `node` on
/// the agent's PATH (the spawn then says so), goes on.
pub(super) fn check_runtime(provider: &str, env: &HashMap<String, String>) -> Result<(), String> {
    if provider != "dsh" {
        return Ok(());
    }
    let path = env.iter().find(|(k, _)| k.eq_ignore_ascii_case("PATH")).map(|(_, v)| std::ffi::OsString::from(v)).or_else(|| std::env::var_os("PATH"));
    let Ok(cwd) = std::env::current_dir() else { return Ok(()) };
    let Ok(node) = which::which_in("node", path, cwd) else { return Ok(()) };
    let version = crate::backend::tool_store::probe_version(&node.to_string_lossy(), &Some("--version".to_string()));
    match version.as_deref().and_then(node_too_old) {
        Some(reason) => Err(reason),
        None => Ok(()),
    }
}

/// Grok reads project rules (the agent's AGENTS.md) only in a trusted folder,
/// and its stdio agent takes no `--trust`. So the agent's own working folder
/// is trusted in `$GROK_HOME/trusted_folders.toml`, the file `grok --trust`
/// writes. Only that folder: `GROK_FOLDER_TRUST=0` would also load any repo's
/// own MCP servers and hooks. An entry already there, trusted or not, is left
/// as the user decided.
fn trust_grok_folder(grok_home: &Path, cwd: &str) -> std::io::Result<()> {
    let folder = if cfg!(windows) { cwd.replace('/', "\\") } else { cwd.to_string() };
    // A TOML literal key can't hold a quote or a line break.
    if folder.is_empty() || folder.contains('\'') || folder.contains('\n') {
        return Ok(());
    }
    let file = grok_home.join("trusted_folders.toml");
    let existing = std::fs::read_to_string(&file).unwrap_or_default();
    let header = format!("[folders.'{folder}']");
    if existing.lines().any(|l| l.trim() == header) {
        return Ok(());
    }
    std::fs::create_dir_all(grok_home)?;
    let sep = if existing.is_empty() || existing.ends_with('\n') { "" } else { "\n" };
    let now = agentmux_common::time::now_secs();
    std::fs::write(&file, format!("{existing}{sep}{header}\ntrusted = true\ndecided_at = {now}\n"))
}

/// DeepSeek Harness uploads its session logs with requests to DeepSeek's API
/// unless a profile patch turns that off, and no env var can. Written only
/// when there is no `$DSH_HOME/cordis.patch.yml` yet, so a user's own patch
/// is never replaced.
fn seed_dsh_patch(dsh_home: &Path) -> std::io::Result<()> {
    let file = dsh_home.join("cordis.patch.yml");
    if file.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(dsh_home)?;
    std::fs::write(file, DSH_NO_SESSION_LOG)
}

const DSH_NO_SESSION_LOG: &str = "# Written by AgentMux: no session-log upload to DeepSeek. Delete or edit to change.\n\
- id: session-log-deepseek\n  config:\n    enabled: false\n";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grok_trusts_the_agents_folder_once_and_keeps_other_entries() {
        let home = tempfile::tempdir().unwrap();
        let file = home.path().join("trusted_folders.toml");
        std::fs::write(&file, "[folders.'C:\\other']\ntrusted = false\ndecided_at = 1").unwrap();
        trust_grok_folder(home.path(), "C:/agents/a").unwrap();
        trust_grok_folder(home.path(), "C:/agents/a").unwrap();
        let s = std::fs::read_to_string(&file).unwrap();
        let folder = if cfg!(windows) { "C:\\agents\\a" } else { "C:/agents/a" };
        assert_eq!(s.matches(&format!("[folders.'{folder}']")).count(), 1);
        assert!(s.starts_with("[folders.'C:\\other']\ntrusted = false\ndecided_at = 1\n"));
        assert!(s.contains(&format!("[folders.'{folder}']\ntrusted = true\n")));
    }

    #[test]
    fn grok_leaves_a_folder_the_user_decided_on() {
        let home = tempfile::tempdir().unwrap();
        let file = home.path().join("trusted_folders.toml");
        let folder = if cfg!(windows) { "C:\\agents\\b" } else { "/agents/b" };
        let before = format!("[folders.'{folder}']\ntrusted = false\n");
        std::fs::write(&file, &before).unwrap();
        trust_grok_folder(home.path(), folder).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), before);
    }

    #[test]
    fn grok_skips_a_folder_a_toml_literal_key_cannot_hold() {
        let home = tempfile::tempdir().unwrap();
        trust_grok_folder(home.path(), "C:/it's").unwrap();
        assert!(!home.path().join("trusted_folders.toml").exists());
    }

    #[test]
    fn dsh_gets_the_no_upload_patch_only_when_it_has_none() {
        let home = tempfile::tempdir().unwrap();
        seed_dsh_patch(home.path()).unwrap();
        let patch = std::fs::read_to_string(home.path().join("cordis.patch.yml")).unwrap();
        assert!(patch.contains("- id: session-log-deepseek\n  config:\n    enabled: false\n"));
        std::fs::write(home.path().join("cordis.patch.yml"), "mine").unwrap();
        seed_dsh_patch(home.path()).unwrap();
        assert_eq!(std::fs::read_to_string(home.path().join("cordis.patch.yml")).unwrap(), "mine");
    }

    #[test]
    fn a_node_older_than_dsh_needs_is_refused_with_both_versions_and_the_rest_go_on() {
        let old = node_too_old("v22.1.0
").expect("too old");
        assert!(old.contains("24.9.0 or later") && old.contains("is 22.1.0.") && old.contains("start the agent again"), "{old}");
        assert!(node_too_old("24.8.9").is_some(), "one patch short");
        assert!(node_too_old("v24.9.0").is_none(), "exactly the minimum");
        assert!(node_too_old("v24.19.0").is_none() && node_too_old("25.0.0").is_none() && node_too_old("v26").is_none());
        assert!(node_too_old("v24.10.0-nightly2025").is_none(), "a suffix after the numbers is ignored");
        assert_eq!(node_too_old("v24").map(|_| ()), Some(()), "24 alone reads as 24.0.0, which is short");
        for unreadable in ["", "node", "vx.y"] {
            assert!(node_too_old(unreadable).is_none(), "{unreadable:?}: dsh decides");
        }
    }

    #[test]
    fn only_dsh_is_checked_and_a_path_with_no_node_goes_on() {
        let empty = tempfile::tempdir().unwrap();
        let env = HashMap::from([("PATH".to_string(), empty.path().to_string_lossy().into_owned())]);
        assert!(check_runtime("dsh", &env).is_ok(), "no node on the PATH: the spawn says so");
        assert!(check_runtime("claude", &env).is_ok());
    }

    #[test]
    fn other_providers_and_missing_dirs_write_nothing() {
        let env = HashMap::new();
        prepare("grok", &env, "C:/x");
        prepare("dsh", &env, "C:/x");
        prepare("claude", &env, "C:/x");
    }
}
