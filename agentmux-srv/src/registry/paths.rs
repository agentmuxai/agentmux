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
///
/// When `agentmux_common::isolated_auth_enabled()` is set, resolves to
/// `<instance_dir>/identity-store.db` instead — a channel-scoped store for
/// destructive Armory testing (delete-account flows) that can never touch
/// the real global store other channels/instances depend on. Falls back
/// to the global path if `AGENTMUX_INSTANCE_DIR` isn't set (e.g. a bare
/// `cargo run` outside the launcher) rather than failing outright — this
/// function's existing `None`-on-unresolvable contract is preserved, just
/// with an extra source to try first. See
/// `docs/specs/SPEC_ISOLATED_AUTH_DEV_TESTING_2026_07_27.md`.
///
/// IMPORTANT: `MigrationContext.home` (`migrations/runner.rs`) must NEVER
/// be derived from this function's return value (e.g. via
/// `.parent().parent()`) — it needs the unconditional global root
/// regardless of isolation, and must call `resolve_global_shared_root()`
/// directly instead. See that module's comments.
pub fn resolve_shared_store_path() -> Option<std::path::PathBuf> {
    if agentmux_common::isolated_auth_enabled() {
        if let Ok(instance_dir) = std::env::var("AGENTMUX_INSTANCE_DIR") {
            if !instance_dir.is_empty() {
                return Some(std::path::PathBuf::from(instance_dir).join("identity-store.db"));
            }
        }
    }
    resolve_global_shared_root().map(|h| h.join("store.db"))
}

/// Resolve the GLOBAL `<home>/shared/agents/reactive/` directory — the
/// cross-channel presence registry for live reactive-injection targets
/// (agent_id -> local_url/block_id/pid/auth_key, one entry per channel),
/// used by MuxBus Tier 2b same-host cross-channel delivery
/// (issue #1916 / `SPEC_MUXBUS_CROSS_CHANNEL_DELIVERY_2026_07_02.md`).
///
/// Sibling of [`resolve_shared_registry_dir`] in *location* (both live
/// under `shared/agents/`), but a different *concern*: that registry
/// mirrors durable agent definitions/session-resume state, while this one
/// tracks live, ephemeral reachability for message forwarding — entries
/// here are written at reactive-register time and removed at unregister,
/// not persisted across restarts.
///
/// Returns `None` only if the global shared root can't be resolved —
/// caller treats this as "cross-channel delivery disabled," falling back
/// to same-channel-only Tier 2 (no behavior regression).
pub fn resolve_shared_reactive_dir() -> Option<PathBuf> {
    resolve_global_shared_root().map(|h| h.join("agents").join("reactive"))
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

/// The global, channel-independent `<home>/shared` root. Deliberately
/// UNAFFECTED by `isolated_auth_enabled()` — callers that need the true
/// `~/.agentmux` home (e.g. `migrations/runner.rs`'s `MigrationContext.home`)
/// must call this directly rather than deriving it from
/// `resolve_shared_store_path()`, which DOES vary with isolation.
pub(crate) fn resolve_global_shared_root() -> Option<PathBuf> {
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

    // Process-global env access — shared with migrations::runner and
    // migrations::m0011_shared_store_backfill's tests, which mutate the SAME
    // AGENTMUX_ISOLATED_AUTH/AGENTMUX_INSTANCE_DIR vars. A module-local lock
    // only serializes tests within this file; Cargo runs a crate's tests in
    // one multi-threaded process, so a local-only lock still let this
    // module's tests race against those two (reagent/codex on PR #2318).
    use crate::test_support::ISOLATED_AUTH_ENV_LOCK as ENV_LOCK;

    fn clear() {
        std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
        std::env::remove_var("AGENTMUX_DATA_DIR");
        std::env::remove_var("AGENTMUX_SHARED_DIR");
        std::env::remove_var("AGENTMUX_ISOLATED_AUTH");
        std::env::remove_var("AGENTMUX_INSTANCE_DIR");
        std::env::remove_var("AGENTMUX_CHANNEL");
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
    fn reactive_dir_uses_shared_dir() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        std::env::set_var("AGENTMUX_SHARED_DIR", "/home/user/.agentmux/shared");
        let r = resolve_shared_reactive_dir().unwrap();
        assert_eq!(
            r,
            PathBuf::from("/home/user/.agentmux/shared/agents/reactive")
        );
        clear();
    }

    #[test]
    fn reactive_dir_override_wins() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", "/tmp/test-home");
        let r = resolve_shared_reactive_dir().unwrap();
        assert_eq!(
            r,
            PathBuf::from("/tmp/test-home/shared/agents/reactive")
        );
        clear();
    }

    #[test]
    fn reactive_dir_is_sibling_of_registry_dir() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        std::env::set_var("AGENTMUX_SHARED_DIR", "/home/user/.agentmux/shared");
        let reactive = resolve_shared_reactive_dir().unwrap();
        let registry = resolve_shared_registry_dir().unwrap();
        assert_eq!(reactive.parent(), registry.parent());
        clear();
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

    #[test]
    fn shared_store_path_default_is_global_on_stable_channel() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", "/tmp/test-home");
        // Isolation flag unset, channel is the real release channel —
        // this is the one default SPEC_ISOLATED_AUTH_DEFAULT_BY_CHANNEL_
        // 2026_08_06.md deliberately leaves unchanged, even with an
        // instance dir present.
        std::env::set_var("AGENTMUX_CHANNEL", "stable");
        std::env::set_var("AGENTMUX_INSTANCE_DIR", "/tmp/test-home/dev/some-branch");
        let r = resolve_shared_store_path().unwrap();
        assert_eq!(r, PathBuf::from("/tmp/test-home/shared/store.db"));
        clear();
    }

    #[test]
    fn shared_store_path_default_is_global_when_channel_unset() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", "/tmp/test-home");
        // No AGENTMUX_CHANNEL at all (e.g. bare `cargo test`) — stay
        // global rather than guess. Isolation flag also unset.
        std::env::set_var("AGENTMUX_INSTANCE_DIR", "/tmp/test-home/dev/some-branch");
        let r = resolve_shared_store_path().unwrap();
        assert_eq!(r, PathBuf::from("/tmp/test-home/shared/store.db"));
        clear();
    }

    #[test]
    fn shared_store_path_isolated_by_default_on_non_stable_channel() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", "/tmp/test-home");
        // The behavior change: no AGENTMUX_ISOLATED_AUTH set at all — a
        // dev/local-build channel isolates by default now.
        std::env::set_var("AGENTMUX_CHANNEL", "dev-some-branch");
        std::env::set_var("AGENTMUX_INSTANCE_DIR", "/tmp/test-home/dev/some-branch");
        let r = resolve_shared_store_path().unwrap();
        assert_eq!(
            r,
            PathBuf::from("/tmp/test-home/dev/some-branch/identity-store.db"),
            "non-stable channels must isolate by default with no explicit flag set"
        );
        clear();
    }

    #[test]
    fn shared_store_path_explicit_opt_out_stays_global_on_non_stable_channel() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", "/tmp/test-home");
        std::env::set_var("AGENTMUX_CHANNEL", "dev-some-branch");
        std::env::set_var("AGENTMUX_ISOLATED_AUTH", "0");
        std::env::set_var("AGENTMUX_INSTANCE_DIR", "/tmp/test-home/dev/some-branch");
        let r = resolve_shared_store_path().unwrap();
        assert_eq!(
            r,
            PathBuf::from("/tmp/test-home/shared/store.db"),
            "AGENTMUX_ISOLATED_AUTH=0 must override the non-stable-channel default"
        );
        clear();
    }

    #[test]
    fn shared_store_path_isolated_uses_instance_dir() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", "/tmp/test-home");
        std::env::set_var("AGENTMUX_ISOLATED_AUTH", "1");
        std::env::set_var("AGENTMUX_INSTANCE_DIR", "/tmp/test-home/dev/some-branch");
        let r = resolve_shared_store_path().unwrap();
        assert_eq!(
            r,
            PathBuf::from("/tmp/test-home/dev/some-branch/identity-store.db"),
            "isolated shared store must live under the channel's own instance dir"
        );
        clear();
    }

    #[test]
    fn shared_store_path_isolated_without_instance_dir_falls_back() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", "/tmp/test-home");
        std::env::set_var("AGENTMUX_ISOLATED_AUTH", "1");
        // AGENTMUX_INSTANCE_DIR deliberately left unset — e.g. a bare
        // `cargo run` outside the launcher. Must degrade to the global
        // path rather than returning None unnecessarily.
        let r = resolve_shared_store_path().unwrap();
        assert_eq!(r, PathBuf::from("/tmp/test-home/shared/store.db"));
        clear();
    }
}
