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
    fn other_providers_and_missing_dirs_write_nothing() {
        let env = HashMap::new();
        prepare("grok", &env, "C:/x");
        prepare("dsh", &env, "C:/x");
        prepare("claude", &env, "C:/x");
    }
}
