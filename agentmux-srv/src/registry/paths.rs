// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Resolve the shared registry directory (`<shared_home>/agents/registry/`).

use std::path::PathBuf;

/// Resolve the GLOBAL `<home>/shared/agents/registry/` directory.
///
/// P0.3 re-roots the named-**instance** registry from the old
/// channel-scoped `channels/<ch>/agents/registry/` (which walked
/// `AGENTMUX_DATA_DIR` up three levels and so landed inside the current
/// channel) to the channel-independent `~/.agentmux/shared/`. An agent
/// named in one channel is then visible in every channel, exactly like the
/// definition store ([`resolve_shared_definitions_dir`]) — the two are now
/// siblings under `shared/agents/`.
///
/// The base directory that instance `working_directory` values are stored
/// *relative to* is tracked separately (the Store's `registry_agents_base`,
/// fed from `AGENTMUX_AGENTS_DIR`) — it must stay the current channel's
/// agents dir even though the registry files are now global.
///
/// Returns `None` only if the global shared root can't be resolved — caller
/// treats this as "registry disabled," no write attempts.
pub fn resolve_shared_registry_dir() -> Option<PathBuf> {
    resolve_global_shared_root().map(|h| h.join("agents").join("registry"))
}

/// Resolve the GLOBAL `<home>/shared/agents/definitions/` directory.
///
/// Sibling of [`resolve_shared_registry_dir`]: since P0.3 both the definition
/// store and the instance registry are **channel-independent**, resolved via
/// the same [`resolve_global_shared_root`] so an agent created/named in one
/// channel is visible in every channel (cross-channel agent persistence,
/// P0.2/P0.3). Resolves via the launcher-exported `AGENTMUX_SHARED_DIR`, with
/// a test override and a `~/.agentmux/shared` fallback.
pub fn resolve_shared_definitions_dir() -> Option<PathBuf> {
    resolve_global_shared_root().map(|h| h.join("agents").join("definitions"))
}

/// Resolve `~/.agentmux/shared/store.db` — the global store for identity
/// accounts, memory bundles, drone definitions, and MuxBus credentials.
///
/// Uses the same root as the agent registry/definitions so
/// `AGENTMUX_HOME_OVERRIDE` and `AGENTMUX_SHARED_DIR` work consistently.
/// Returns `None` only when the shared root itself can't be resolved.
pub fn resolve_shared_store_path() -> Option<std::path::PathBuf> {
    resolve_global_shared_root().map(|h| h.join("store.db"))
}

/// Resolve the GLOBAL `<home>/shared/agents/transcripts/` directory.
///
/// Sibling of [`resolve_shared_registry_dir`] / [`resolve_shared_definitions_dir`]:
/// the agent's *conversation transcript* is the last per-channel agent surface
/// (definitions, instances, workspaces, and auth all became global in
/// #1387–#1396). Backing the `agent:<defId>:current` FileStore zone with a store
/// rooted here makes a conversation load when you open the agent from *any*
/// build/channel — the open path reads the same zone regardless of which channel
/// wrote it. See `docs/analysis/ANALYSIS_CROSS_CHANNEL_CONVERSATION_HISTORY_2026_06_14.md`.
///
/// Returns `None` only if the global shared root can't be resolved — caller
/// treats this as "global transcripts disabled" and falls back to the
/// per-channel store.
pub fn resolve_shared_transcripts_dir() -> Option<PathBuf> {
    resolve_global_shared_root().map(|h| h.join("agents").join("transcripts"))
}

/// The global, channel-independent `<home>/shared` root.
fn resolve_global_shared_root() -> Option<PathBuf> {
    if let Ok(s) = std::env::var("AGENTMUX_HOME_OVERRIDE") {
        if !s.is_empty() {
            // Consistent with data_paths everywhere else: the override is the
            // ~/.agentmux root, with the shared dir at root/shared. (reagent
            // P2 on #1385.)
            return Some(PathBuf::from(s).join("shared"));
        }
    }
    if let Ok(s) = std::env::var("AGENTMUX_SHARED_DIR") {
        if !s.is_empty() {
            return Some(PathBuf::from(s));
        }
    }
    dirs::home_dir().map(|h| h.join(".agentmux").join("shared"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Process-global env access — serialize so parallel tests don't race.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn clear() {
        std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
        std::env::remove_var("AGENTMUX_DATA_DIR");
        std::env::remove_var("AGENTMUX_SHARED_DIR");
    }

    #[test]
    fn override_wins() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        // The override is the ~/.agentmux root; the global registry lives at
        // root/shared/agents/registry (P0.3 re-root).
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", "/tmp/test-home");
        let r = resolve_shared_registry_dir().unwrap();
        assert_eq!(
            r,
            PathBuf::from("/tmp/test-home/shared/agents/registry")
        );
        clear();
    }

    #[test]
    fn registry_uses_shared_dir_and_ignores_data_dir() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        // The instance registry is GLOBAL now: it resolves from the shared
        // root, NOT by walking up the per-version AGENTMUX_DATA_DIR (which
        // would land in the current channel). Set both and confirm the
        // channel-scoped data dir is ignored.
        std::env::set_var("AGENTMUX_SHARED_DIR", "/home/user/.agentmux/shared");
        std::env::set_var(
            "AGENTMUX_DATA_DIR",
            "/home/user/.agentmux/channels/stable/versions/0.44.2/data",
        );
        let r = resolve_shared_registry_dir().unwrap();
        assert_eq!(
            r,
            PathBuf::from("/home/user/.agentmux/shared/agents/registry")
        );
        // Sibling of the definition store — both under shared/agents/.
        let d = resolve_shared_definitions_dir().unwrap();
        assert_eq!(r.parent(), d.parent());
        clear();
    }

    #[test]
    fn falls_back_to_home() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        // No env set — uses dirs::home_dir()/.agentmux/shared. Non-None.
        assert!(resolve_shared_registry_dir().is_some());
    }

    #[test]
    fn definitions_dir_uses_shared_dir() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        std::env::set_var("AGENTMUX_SHARED_DIR", "/home/user/.agentmux/shared");
        let r = resolve_shared_definitions_dir().unwrap();
        assert_eq!(
            r,
            PathBuf::from("/home/user/.agentmux/shared/agents/definitions")
        );
        clear();
    }

    #[test]
    fn definitions_dir_override_wins() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", "/tmp/test-home");
        let r = resolve_shared_definitions_dir().unwrap();
        // Override is the ~/.agentmux root; shared lives at root/shared.
        assert_eq!(
            r,
            PathBuf::from("/tmp/test-home/shared/agents/definitions")
        );
        clear();
    }
}
