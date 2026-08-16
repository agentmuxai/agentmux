// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Filesystem/directory provisioning for the pre-launch OAuth flow.
//!
//! Split out of `identity_handlers.rs` (module-organization pass, see
//! `docs/specs/PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_2026_07_20.md`) —
//! owns `compute_and_ensure_bundle_dir` (vestigial, bundle mode was
//! retired in Phase 4c of SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md,
//! kept only so the wire shape and its one call site in
//! `identity_handlers::register_identity_handlers` don't need touching)
//! and `compute_and_ensure_account_dir` (the live direct-account path).

use crate::backend::providers::get_provider;
use crate::backend::storage::store::Store;

/// Compute the per-bundle OAuth config dir + ensure it exists +
/// override the provider's `auth_config_dir_env_var` entry in `auth_env`.
///
/// Returns `Some(<absolute path string>)` when:
///   - `into_bundle_id` is `Some` and non-empty AND
///   - the provider is registered in the CLI provider registry AND
///   - the provider declares an `auth_config_dir_env_var` (oauth-class
///     providers per `identity::resolver::provider_class` — claude / codex /
///     openclaw / gemini / copilot as of
///     REPORT_AUTH_ARCHITECTURE_STATE_AND_RETHINK_2026_07_21.md §2.5/§6) AND
///   - `DataPaths::from_env()` resolves AND
///   - `create_dir_all` succeeds.
///
/// Otherwise returns `None` and leaves `auth_env` untouched. Callers
/// must continue without per-bundle isolation (legacy ambient path,
/// or skip the binding-persist step) — never abort the OAuth start.
///
/// Per `specs/archive/SPEC_OAUTH_IDENTITY_BUNDLES_2026_05_22.md` §4.5: the dir
/// (and the env-var key) come from the CLI provider registry
/// (`backend::providers::get_provider(id)`) so the resolver / spawn
/// path / OAuth-start handler never drift on which env var redirects
/// each CLI's config home. The single source of truth lives in
/// `agentmux-srv/src/backend/providers.rs`.
pub(crate) fn compute_and_ensure_bundle_dir(
    into_bundle_id: Option<&str>,
    provider_id: &str,
    auth_env: &mut std::collections::HashMap<String, String>,
) -> Option<String> {
    let bundle_id = into_bundle_id.filter(|s| !s.is_empty())?;
    // Gate on provider_class so api-key-class providers (which have a
    // registry entry with a non-empty `auth_config_dir_env_var` —
    // e.g. kimi's `KIMI_SHARE_DIR`) never go through the per-bundle
    // OAuth-dir path. Only providers `provider_class` classifies as
    // OAuth-class (claude / codex / openclaw / gemini / copilot) get the
    // per-bundle override — see that function's own doc comment for the
    // current, authoritative set.
    match crate::identity::resolver::provider_class(provider_id) {
        Some(crate::identity::resolver::ProviderClass::OAuth { .. }) => {}
        _ => return None,
    }
    let provider = match get_provider(provider_id) {
        Some(p) => p,
        None => {
            // OAuth-class per provider_class but missing from the CLI
            // registry — should be impossible (resolver reads the env
            // var from the registry itself), but treat as a soft fail.
            tracing::warn!(
                target: "identity",
                provider_id,
                "auth.start: oauth-class provider not in registry — skipping per-bundle dir override"
            );
            return None;
        }
    };
    // Empty env-var name → no isolation possible (oauth-class providers
    // should never have this empty per spec, but belt-and-braces).
    if provider.auth_config_dir_env_var.is_empty() {
        return None;
    }
    let paths = match agentmux_common::DataPaths::from_env() {
        Some(p) => p,
        None => {
            tracing::warn!(
                target: "identity",
                provider_id,
                bundle_id,
                "auth.start: DataPaths::from_env() returned None — skipping per-bundle dir override"
            );
            return None;
        }
    };
    // identity_dir rejects unsafe path segments (empty / `.` / `..` /
    // segment with `/`, `\`, drive-letter, …). bundle_id comes from
    // the auth.start request body, so this guard prevents a crafted
    // id from escaping the identities root.
    let bundle_root = match paths.identity_dir(bundle_id) {
        Some(p) => p,
        None => {
            tracing::warn!(
                target: "identity",
                provider_id,
                bundle_id,
                "auth.start: bundle_id is not a safe path segment — skipping per-bundle dir override"
            );
            return None;
        }
    };
    let dir = bundle_root.join(provider.auth_dir_name);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(
            target: "identity",
            provider_id,
            bundle_id,
            error = %e,
            path = %dir.display(),
            "auth.start: failed to create per-bundle config dir — skipping override"
        );
        return None;
    }
    // See the identical block in compute_and_ensure_account_dir (the live
    // sibling of this vestigial function) for why this exists.
    if agentmux_common::isolated_auth_enabled() {
        if let Err(e) = agentmux_common::ensure_history_link(
            &dir.join("projects"),
            &paths.identity_history_dir(bundle_id, provider.auth_dir_name),
        ) {
            tracing::warn!(
                target: "identity",
                provider_id,
                bundle_id,
                error = %e,
                "auth.start: failed to link conversation history to the global location — \
                 this bundle's history may not survive a channel/build change"
            );
        }
    }
    let dir_str = dir.to_string_lossy().to_string();
    // Override (or insert) the provider's config-dir env var. The
    // frontend may have computed the legacy ambient dir via
    // `ensureAuthDir(providerId)` and put it here under the same key;
    // we replace it with the per-bundle dir so the OAuth CLI writes
    // its tokens inside the bundle, not in the ambient version-config
    // dir.
    auth_env.insert(
        provider.auth_config_dir_env_var.to_string(),
        dir_str.clone(),
    );
    tracing::info!(
        target: "identity",
        provider_id,
        bundle_id,
        env_var = provider.auth_config_dir_env_var,
        dir = %dir.display(),
        "auth.start: per-bundle OAuth config dir wired"
    );
    Some(dir_str)
}

/// Direct-account sibling of `compute_and_ensure_bundle_dir` (issue
/// #1624 PR-C Part B) — bypasses the bundle system entirely. Mints a
/// fresh `account_id` when `existing_account_id` is empty (first-time
/// OAuth connect from the launch modal); reuses it when non-empty
/// (Reconnect — refresh tokens in place, same isolation dir, same
/// account row updated in place).
///
/// Every fresh mint (`existing_account_id` empty) is swept for BEFORE it,
/// giving prior abandoned/failed attempts a chance to be cleaned up
/// (reagent finding on #2260 — see `sweep_orphaned_account_dirs`'s own
/// doc comment). 30 minutes comfortably clears every frontend poll
/// timeout in this flow (5 minutes, as of this writing) plus room for a
/// slow user plus one retry, so an in-progress login's dir is never
/// swept out from under it.
const ORPHAN_SWEEP_MIN_AGE_SECS: u64 = 30 * 60;

/// Returns `(account_id, dir)` — `account_id` is always populated (even
/// on a dir-resolution failure, so the caller can still log/track it);
/// `dir` is `None` on the same gate/registry/fs failures
/// `compute_and_ensure_bundle_dir` treats as soft failures — never
/// abort `auth.start` over a config-dir issue.
pub(crate) fn compute_and_ensure_account_dir(
    store: &Store,
    existing_account_id: &str,
    provider_id: &str,
    auth_env: &mut std::collections::HashMap<String, String>,
) -> (String, Option<String>) {
    let account_id = if existing_account_id.is_empty() {
        uuid::Uuid::new_v4().to_string()
    } else {
        existing_account_id.to_string()
    };

    // Same provider_class gate as the bundle path — only providers
    // `provider_class` classifies as OAuth-class (claude / codex /
    // openclaw / gemini / copilot as of
    // REPORT_AUTH_ARCHITECTURE_STATE_AND_RETHINK_2026_07_21.md §2.5/§6)
    // get a per-account isolation dir.
    match crate::identity::resolver::provider_class(provider_id) {
        Some(crate::identity::resolver::ProviderClass::OAuth { .. }) => {}
        _ => return (account_id, None),
    }
    let provider = match get_provider(provider_id) {
        Some(p) => p,
        None => {
            tracing::warn!(
                target: "identity",
                provider_id,
                account_id,
                "auth.start (direct-account): oauth-class provider not in registry — skipping config dir"
            );
            return (account_id, None);
        }
    };
    if provider.auth_config_dir_env_var.is_empty() {
        return (account_id, None);
    }
    let paths = match agentmux_common::DataPaths::from_env() {
        Some(p) => p,
        None => {
            tracing::warn!(
                target: "identity",
                provider_id,
                account_id,
                "auth.start (direct-account): DataPaths::from_env() returned None — skipping config dir"
            );
            return (account_id, None);
        }
    };
    if existing_account_id.is_empty() {
        let removed = crate::identity::cleanup::sweep_orphaned_account_dirs(
            store,
            &paths.identities_dir(),
            ORPHAN_SWEEP_MIN_AGE_SECS,
        );
        if !removed.is_empty() {
            tracing::info!(
                target: "identity",
                provider_id,
                account_id,
                swept = removed.len(),
                "auth.start (direct-account): swept orphaned account dirs before minting a fresh one"
            );
        }
    }
    // identity_dir is already generic (not bundle-specific) — same
    // unsafe-path-segment rejection applies to account_id here.
    let account_root = match paths.identity_dir(&account_id) {
        Some(p) => p,
        None => {
            tracing::warn!(
                target: "identity",
                provider_id,
                account_id,
                "auth.start (direct-account): account_id is not a safe path segment — skipping config dir"
            );
            return (account_id, None);
        }
    };
    let dir = account_root.join(provider.auth_dir_name);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(
            target: "identity",
            provider_id,
            account_id,
            error = %e,
            path = %dir.display(),
            "auth.start (direct-account): failed to create config dir — skipping"
        );
        return (account_id, None);
    }
    // Keep conversation-history transcripts (e.g. Claude Code's own
    // `projects/` subdir) reachable at a stable, always-global location
    // even when this credential dir is per-channel isolated — see
    // docs/specs/SPEC_AGENT_IDENTITY_HISTORY_PERSISTENCE_PROTOCOL_2026_08_16.md
    // §4.1. Best-effort: never abort auth.start over this, same
    // philosophy as every other soft-fail in this function. Only
    // meaningful when isolated_auth_enabled() — when it's off, `dir` is
    // already inside the global identities tree and `dir/projects`
    // already IS the global location, so linking it to itself would be
    // pointless.
    if agentmux_common::isolated_auth_enabled() {
        if let Err(e) = agentmux_common::ensure_history_link(
            &dir.join("projects"),
            &paths.identity_history_dir(&account_id, provider.auth_dir_name),
        ) {
            tracing::warn!(
                target: "identity",
                provider_id,
                account_id,
                error = %e,
                "auth.start (direct-account): failed to link conversation history to the global \
                 location — this account's history may not survive a channel/build change"
            );
        }
    }
    let dir_str = dir.to_string_lossy().to_string();
    auth_env.insert(provider.auth_config_dir_env_var.to_string(), dir_str.clone());
    tracing::info!(
        target: "identity",
        provider_id,
        account_id,
        env_var = provider.auth_config_dir_env_var,
        dir = %dir.display(),
        "auth.start (direct-account): OAuth config dir wired"
    );
    (account_id, Some(dir_str))
}

#[cfg(test)]
mod tests {
    use super::*;

    // `AGENTMUX_HOME_OVERRIDE` is one of the process-global env vars
    // `crate::test_support::ISOLATED_AUTH_ENV_LOCK` exists specifically to
    // guard — a module-local lock here only serializes tests WITHIN this
    // module; `cargo test` runs a crate's tests in one multi-threaded
    // process, so a local-only lock still let this module's tests race
    // against any other module's tests touching the same var (confirmed
    // live: `server::app_api::bundle_self_get_registry_fallback_tests`
    // and `server::native_memory_handlers::tests`, both added for
    // SPEC_AGENT_PANE_HISTORY_ALIGNMENT_2026_08_05.md, intermittently
    // failed this module's `compute_account_dir_mints_fresh_id_when_
    // existing_is_empty` in full-suite runs — passed every time alone).
    // Reusing the crate-wide lock here closes that gap.
    use crate::test_support::ISOLATED_AUTH_ENV_LOCK as ENV_LOCK;

    #[test]
    fn compute_account_dir_mints_fresh_id_when_existing_is_empty() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::TempDir::new().unwrap();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", tmp.path());
        let paths = agentmux_common::DataPaths::resolve(
            "0.0.0-test",
            &agentmux_common::RuntimeMode::Installed,
        )
        .unwrap();
        paths.ensure_dirs().unwrap();
        for (k, v) in paths.to_env_vars() {
            std::env::set_var(k, v);
        }

        let wstore = Store::open_in_memory().unwrap();
        let mut env = std::collections::HashMap::new();
        let (account_id, dir) = compute_and_ensure_account_dir(&wstore, "", "claude", &mut env);
        assert!(!account_id.is_empty(), "must mint a fresh id when none supplied");
        let dir = dir.expect("oauth-class provider must yield a dir");
        let expected = paths.identity_dir(&account_id).unwrap().join("claude");
        assert_eq!(std::path::Path::new(&dir), expected);
        assert!(expected.is_dir());
        assert_eq!(env.get("CLAUDE_CONFIG_DIR").map(String::as_str), Some(dir.as_str()));

        std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
        for (k, _) in paths.to_env_vars() {
            std::env::remove_var(k);
        }
    }

    #[test]
    fn compute_account_dir_reuses_existing_id() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::TempDir::new().unwrap();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", tmp.path());
        let paths = agentmux_common::DataPaths::resolve(
            "0.0.0-test",
            &agentmux_common::RuntimeMode::Installed,
        )
        .unwrap();
        paths.ensure_dirs().unwrap();
        for (k, v) in paths.to_env_vars() {
            std::env::set_var(k, v);
        }

        let wstore = Store::open_in_memory().unwrap();
        let mut env = std::collections::HashMap::new();
        let (account_id, _) = compute_and_ensure_account_dir(&wstore, "acc-reconnect", "claude", &mut env);
        assert_eq!(account_id, "acc-reconnect", "reconnect must reuse the supplied id, not mint a new one");

        std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
        for (k, _) in paths.to_env_vars() {
            std::env::remove_var(k);
        }
    }

    // docs/specs/SPEC_AGENT_IDENTITY_HISTORY_PERSISTENCE_PROTOCOL_2026_08_16.md
    // §4.1: when the credential dir is per-channel isolated, its
    // `projects/` subpath must become a link to the always-global
    // history location, not a real (fresh, empty, orphan-prone)
    // directory. This is the actual end-to-end wiring test — the
    // `ensure_history_link` unit tests in `agentmux-common` only prove
    // the linking primitive works in isolation, not that this call site
    // actually invokes it with the right paths.
    #[test]
    fn compute_account_dir_links_projects_to_the_global_history_location_when_isolated() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::TempDir::new().unwrap();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", tmp.path());
        std::env::set_var("AGENTMUX_ISOLATED_AUTH", "1");
        let paths = agentmux_common::DataPaths::resolve(
            "0.0.0-test",
            &agentmux_common::RuntimeMode::Installed,
        )
        .unwrap();
        paths.ensure_dirs().unwrap();
        for (k, v) in paths.to_env_vars() {
            std::env::set_var(k, v);
        }
        assert!(agentmux_common::isolated_auth_enabled(), "precondition: this test needs isolation actually on");

        let wstore = Store::open_in_memory().unwrap();
        let mut env = std::collections::HashMap::new();
        let (account_id, dir) = compute_and_ensure_account_dir(&wstore, "", "claude", &mut env);
        let dir = std::path::PathBuf::from(dir.expect("oauth-class provider must yield a dir"));
        assert!(
            dir.starts_with(&paths.instance_dir),
            "precondition: the credential dir must actually be the per-channel isolated one, not the global one, or this test isn't exercising the code path it claims to"
        );

        let global_history = paths.identity_history_dir(&account_id, "claude");
        std::fs::write(global_history.join("real-session.jsonl"), b"a real session").unwrap();
        let via_isolated_path = std::fs::read(dir.join("projects").join("real-session.jsonl"))
            .expect("a file written at the global history location must be readable through the isolated credential dir's projects/ subpath");
        assert_eq!(via_isolated_path, b"a real session");

        std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
        std::env::remove_var("AGENTMUX_ISOLATED_AUTH");
        for (k, _) in paths.to_env_vars() {
            std::env::remove_var(k);
        }
    }

    #[test]
    fn compute_account_dir_skipped_for_api_key_provider() {
        // Same provider_class gate as the bundle path — dir is None,
        // but the account id is still returned (unlike bundle mode,
        // there's no "skip entirely" case here — the account always
        // gets minted/reused, only the isolation dir is conditional).
        let wstore = Store::open_in_memory().unwrap();
        let mut env = std::collections::HashMap::new();
        let (account_id, dir) = compute_and_ensure_account_dir(&wstore, "", "kimi", &mut env);
        assert!(!account_id.is_empty());
        assert!(dir.is_none(), "api-key provider class must skip the OAuth dir path");
        assert!(env.get("KIMI_SHARE_DIR").is_none());
    }
}
