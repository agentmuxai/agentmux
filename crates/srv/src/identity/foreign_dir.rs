// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A config dir inside another channel's `identities/` folder.
//!
//! Auth is shared across channels (`<home>/shared/identities/<account>/…`) unless a build opts into
//! `AGENTMUX_ISOLATED_AUTH=1`, which keeps its accounts' dirs under `<home>/channels/<channel>/identities/`.
//! A dir recorded by one of those builds (an account row, or a pane's `cmd:env`) is no place for
//! another channel's agent to run: it holds no sign-in this channel can see, and a session recorded
//! there can't be resumed from the shared dir. Two readings of the same agent that disagree on it
//! made its watched config dir (and, with it, its spawn) flip between the two.

use std::collections::HashMap;
use std::path::{Component, Path};

use crate::backend::obj::{Block, MetaMapType};
use crate::backend::storage::store::Store;

/// The variable Claude Code reads its config dir from.
const VAR: &str = "CLAUDE_CONFIG_DIR";

/// Whether `dir` lies in `<…>/channels/<name>/identities/…` for a `<name>` other than `own_channel`.
/// Lexical, case-insensitive (Windows paths), and false for any dir not in that shape.
pub fn is_other_channel_identity_dir(dir: &Path, own_channel: &str) -> bool {
    let parts: Vec<String> = dir
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().to_ascii_lowercase()),
            _ => None,
        })
        .collect();
    let own = own_channel.to_ascii_lowercase();
    parts.windows(3).any(|w| w[0] == "channels" && w[2] == "identities" && w[1] != own)
}

/// [`is_other_channel_identity_dir`] for the channel this srv runs in. `own_channel` is
/// `AGENTMUX_CHANNEL` and `own_instance_dir` is `AGENTMUX_INSTANCE_DIR` (what the launcher gives srv).
/// A srv that doesn't know its channel treats no dir as foreign, and a dir under its own instance
/// dir never is.
fn is_foreign_in(dir: &Path, own_channel: Option<&str>, own_instance_dir: Option<&Path>) -> bool {
    let Some(own) = own_channel.map(str::trim).filter(|c| !c.is_empty()) else { return false };
    if own_instance_dir.is_some_and(|own_dir| dir.starts_with(own_dir)) {
        return false;
    }
    is_other_channel_identity_dir(dir, own)
}

/// [`is_foreign_in`] for this process's environment.
pub fn is_foreign_to_this_channel(dir: &Path) -> bool {
    let channel = std::env::var("AGENTMUX_CHANNEL").ok();
    let instance_dir = std::env::var_os("AGENTMUX_INSTANCE_DIR").map(std::path::PathBuf::from);
    is_foreign_in(dir, channel.as_deref(), instance_dir.as_deref())
}

/// The pane's `cmd:env` with its `CLAUDE_CONFIG_DIR` pointed at `bound`, when the value it holds is
/// a dir of another channel's (`None` when there is nothing to heal).
fn env_with_bound_config_dir(
    env: &HashMap<String, String>,
    bound: &Path,
    is_foreign: impl Fn(&Path) -> bool,
) -> Option<HashMap<String, String>> {
    let held = env.get(VAR)?;
    if !is_foreign(Path::new(held)) || is_foreign(bound) {
        return None;
    }
    let mut healed = env.clone();
    healed.insert(VAR.to_string(), bound.to_string_lossy().into_owned());
    Some(healed)
}

/// Heals a pane whose `cmd:env` names another channel's config dir: rewritten to `bound`, the dir its
/// account really spawns in. The window's auth re-check and login recovery read `cmd:env` as it is.
pub fn heal_block_config_dir_env(mstore: &Store, block: Option<&Block>, bound: Option<&Path>) {
    let (Some(block), Some(bound)) = (block, bound) else { return };
    let Some(healed) = env_with_bound_config_dir(&crate::backend::blockcontroller::cmd_env_of(&block.meta), bound, is_foreign_to_this_channel) else {
        return;
    };
    let mut update = MetaMapType::new();
    update.insert("cmd:env".to_string(), serde_json::json!(healed));
    match crate::server::service::update_object_meta(mstore, &format!("block:{}", block.oid), &update) {
        Ok(()) => tracing::info!(block_id = %block.oid, dir = %bound.display(), "healed a pane's config dir that named another channel's identities folder"),
        Err(e) => tracing::warn!(block_id = %block.oid, error = %e, "could not heal a pane's config dir"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pane_env_naming_another_channels_dir_is_pointed_at_the_bound_dir() {
        let mut env = HashMap::from([("CLAUDE_CONFIG_DIR".to_string(), OTHER.to_string()), ("X".to_string(), "1".to_string())]);
        let bound = Path::new(SHARED);
        let foreign = |dir: &Path| is_other_channel_identity_dir(dir, "this-channel");
        let healed = env_with_bound_config_dir(&env, bound, foreign).expect("foreign dir is healed");
        assert_eq!(healed["CLAUDE_CONFIG_DIR"], SHARED);
        assert_eq!(healed["X"], "1", "the rest of the env is kept");
        // Nothing to heal: a shared dir, no variable, or a bound dir that is itself foreign.
        env.insert("CLAUDE_CONFIG_DIR".to_string(), SHARED.to_string());
        assert_eq!(env_with_bound_config_dir(&env, bound, foreign), None);
        assert_eq!(env_with_bound_config_dir(&HashMap::new(), bound, foreign), None);
        env.insert("CLAUDE_CONFIG_DIR".to_string(), OTHER.to_string());
        assert_eq!(env_with_bound_config_dir(&env, Path::new(OTHER), foreign), None);
    }

    const OTHER: &str = "C:/Users/u/.agentmux/channels/local-main-b28b7a-051fbf53/identities/322f5f07-40a2/claude";
    const SHARED: &str = "C:/Users/u/.agentmux/shared/identities/b860809b-8bc2/claude";

    #[test]
    fn a_dir_in_another_channels_identities_folder_is_foreign() {
        assert!(is_other_channel_identity_dir(Path::new(OTHER), "local-loap-3-memory-lab-f5abd5-76d6ee20"));
        assert!(is_other_channel_identity_dir(Path::new("/h/.agentmux/channels/stable/identities/a/claude"), "dev"));
    }

    #[test]
    fn this_channels_own_dir_and_the_shared_dir_are_not() {
        assert!(!is_other_channel_identity_dir(Path::new(OTHER), "local-main-b28b7a-051fbf53"));
        assert!(!is_other_channel_identity_dir(Path::new(OTHER), "LOCAL-MAIN-B28B7A-051FBF53"));
        assert!(!is_other_channel_identity_dir(Path::new(SHARED), "stable"));
    }

    #[test]
    fn a_srv_that_does_not_know_its_channel_calls_nothing_foreign() {
        assert!(!is_foreign_in(Path::new(OTHER), None, None));
        assert!(!is_foreign_in(Path::new(OTHER), Some("  "), None));
        assert!(is_foreign_in(Path::new(OTHER), Some("stable"), None));
    }

    #[test]
    fn a_dir_under_this_srvs_own_instance_dir_is_never_foreign() {
        let own = Path::new("C:/Users/u/.agentmux/channels/local-main-b28b7a-051fbf53");
        assert!(!is_foreign_in(Path::new(OTHER), Some("renamed"), Some(own)));
        assert!(is_foreign_in(Path::new(OTHER), Some("renamed"), Some(Path::new("C:/Users/u/.agentmux/channels/stable"))));
    }

    #[test]
    fn only_the_identities_folder_of_a_channel_counts() {
        // A channel's other folders (agents, config) and unrelated paths named alike.
        assert!(!is_other_channel_identity_dir(Path::new("/h/.agentmux/channels/stable/agents/a1"), "dev"));
        assert!(!is_other_channel_identity_dir(Path::new("/h/channels/identities"), "dev"));
        assert!(!is_other_channel_identity_dir(Path::new("/h/.config/claude-agent2"), "dev"));
        assert!(!is_other_channel_identity_dir(Path::new(""), "dev"));
    }
}
