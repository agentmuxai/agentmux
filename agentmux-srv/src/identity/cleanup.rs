// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! On-disk / keychain credential cleanup for a deleted identity account.
//!
//! Layer 1 of the account-delete auth-lifecycle remediation
//! (`docs/analysis/ANALYSIS_ACCOUNT_DELETE_AUTH_LIFECYCLE_GAP_2026_07_14.md`
//! §4 option 1): deleting an account row must not leave its credential
//! material behind. Two secret backends carry on-host state:
//!
//! - `SecretRef::Keychain` — delete the OS-keychain entry (moved here
//!   from the inline block in the `deleteidentityaccount` handler).
//! - `SecretRef::OAuthConfigDir { dir }` — the CLI's live access +
//!   refresh tokens sit in that directory; remove the tree. This is
//!   best-effort and **containment-guarded**: the dir is only removed
//!   when it resolves INSIDE the agentmux identities root
//!   (`~/.agentmux/shared/identities/`). The legacy `~/.claude`
//!   migration case (and any other ambient/global CLI dir) is the
//!   user's own login — log and skip, never delete.
//!
//! Other variants (`Env`, `SecretsManager`, `PlaintextDev`) hold no
//! agentmux-owned on-host state → no-op.
//!
//! All log lines use the `identity.delete:` prefix so they land in the
//! `muxlog auth` vocabulary (regex `identity\.(unlink|delete|self\.|account)`),
//! and use `info!`/`warn!` — the production filter is
//! "agentmuxsrv=info,info", so `debug!` would be invisible (reagent P1,
//! PR #2143). Provider-side token revocation (running the CLI's own
//! `logout` against the dir before deleting) is an explicit follow-up —
//! it needs per-provider subprocess plumbing this module doesn't have.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::backend::storage::store::{IdentityAccount, SecretRef, Store};

/// What `cleanup_account_secrets` did, for callers/tests to assert on.
/// Logging already happened inside the function.
#[derive(Debug, PartialEq, Eq)]
pub enum SecretCleanup {
    /// Keychain entry removed (or was already absent — idempotent).
    KeychainRemoved,
    /// Keychain delete failed (logged as warn; account delete proceeds).
    KeychainFailed(String),
    /// OAuth config dir tree removed.
    OAuthDirRemoved(PathBuf),
    /// OAuth config dir was already gone — nothing to remove.
    OAuthDirAbsent(PathBuf),
    /// OAuth config dir NOT removed: it does not resolve inside the
    /// agentmux identities root (e.g. the legacy `~/.claude` migration
    /// account), or the root itself could not be resolved/canonicalized.
    OAuthDirSkipped { dir: PathBuf, reason: String },
    /// OAuth config dir removal failed (fs error; logged as warn).
    OAuthDirFailed { dir: PathBuf, error: String },
    /// Secret backend holds no agentmux-owned on-host state.
    NoOp,
}

/// Best-effort removal of the on-host credential material behind
/// `acct.secret_ref`. Never returns `Err` — account deletion must not be
/// blocked by cleanup trouble; every outcome is logged.
///
/// `identities_root` is the agentmux identities root
/// (`DataPaths::identities_dir()`), `None` when `DataPaths::from_env()`
/// could not resolve (CI / unusual envs) — OAuth dirs are then skipped,
/// never guessed. Blocking (keyring + fs) — call via `spawn_blocking`
/// from async contexts.
pub fn cleanup_account_secrets(
    acct: &IdentityAccount,
    identities_root: Option<&Path>,
) -> SecretCleanup {
    match &acct.secret_ref {
        SecretRef::Keychain { .. } => match crate::identity::secret_store::delete(&acct.id) {
            Ok(()) => {
                tracing::info!(
                    account_id = %acct.id,
                    provider = %acct.provider,
                    "identity.delete: keychain secret removed"
                );
                SecretCleanup::KeychainRemoved
            }
            Err(e) => {
                tracing::warn!(
                    account_id = %acct.id,
                    provider = %acct.provider,
                    error = %e,
                    "identity.delete: keychain secret delete failed"
                );
                SecretCleanup::KeychainFailed(e)
            }
        },
        SecretRef::OAuthConfigDir { dir } => cleanup_oauth_dir(acct, Path::new(dir), identities_root),
        SecretRef::Env { .. } | SecretRef::SecretsManager { .. } | SecretRef::PlaintextDev { .. } => {
            SecretCleanup::NoOp
        }
    }
}

fn cleanup_oauth_dir(
    acct: &IdentityAccount,
    dir: &Path,
    identities_root: Option<&Path>,
) -> SecretCleanup {
    let skip = |reason: String| -> SecretCleanup {
        tracing::warn!(
            account_id = %acct.id,
            provider = %acct.provider,
            dir = %dir.display(),
            reason = %reason,
            "identity.delete: oauth config dir outside data root — skipped"
        );
        SecretCleanup::OAuthDirSkipped { dir: dir.to_path_buf(), reason }
    };

    let root = match identities_root {
        Some(r) => r,
        None => return skip("identities root unresolved (DataPaths::from_env() = None)".into()),
    };
    if !dir.exists() {
        // Nothing on disk — already clean (e.g. the CLI never wrote tokens).
        tracing::info!(
            account_id = %acct.id,
            provider = %acct.provider,
            dir = %dir.display(),
            "identity.delete: oauth config dir already absent"
        );
        return SecretCleanup::OAuthDirAbsent(dir.to_path_buf());
    }
    // Canonicalize BOTH sides so `..` segments, symlinks, and Windows
    // `\\?\` prefixes can't defeat the containment check.
    let canon_root = match std::fs::canonicalize(root) {
        Ok(p) => p,
        Err(e) => return skip(format!("identities root not canonicalizable: {e}")),
    };
    let canon_dir = match std::fs::canonicalize(dir) {
        Ok(p) => p,
        Err(e) => return skip(format!("dir not canonicalizable: {e}")),
    };
    // Strictly inside the root — refuse the root itself and anything
    // outside it (the legacy `~/.claude` migration dir lands here).
    if canon_dir == canon_root || !canon_dir.starts_with(&canon_root) {
        return skip(format!(
            "resolved path {} is not strictly inside identities root {}",
            canon_dir.display(),
            canon_root.display()
        ));
    }
    match std::fs::remove_dir_all(&canon_dir) {
        Ok(()) => {
            // Verify the removal actually took — a "removed" log that leaves
            // the credential on disk is the exact login/logout-round bug this
            // diagnostic exists to catch (e.g. a racing re-seed, a bind-mount,
            // or a handle keeping the tree alive). Escalate to WARN if so.
            if canon_dir.exists() {
                tracing::warn!(
                    account_id = %acct.id,
                    provider = %acct.provider,
                    dir = %dir.display(),
                    "identity.delete: oauth config dir STILL PRESENT after remove_dir_all — credential not cleared"
                );
            } else {
                tracing::info!(
                    account_id = %acct.id,
                    provider = %acct.provider,
                    dir = %dir.display(),
                    "identity.delete: oauth config dir removed (verified absent)"
                );
            }
            SecretCleanup::OAuthDirRemoved(dir.to_path_buf())
        }
        Err(e) => {
            tracing::warn!(
                account_id = %acct.id,
                provider = %acct.provider,
                dir = %dir.display(),
                error = %e,
                "identity.delete: oauth config dir removal failed"
            );
            SecretCleanup::OAuthDirFailed { dir: dir.to_path_buf(), error: e.to_string() }
        }
    }
}

/// Canonicalizes both `dir` and `root`, then removes `dir`'s tree only if
/// it resolves strictly inside `root` (never the root itself) — the same
/// escape guard `cleanup_oauth_dir` uses for a single account's credential
/// dir, generalized for `sweep_orphaned_account_dirs` below, which removes
/// a whole per-account dir rather than one provider's subdir within it.
fn remove_tree_if_contained(dir: &Path, root: &Path) -> Result<(), String> {
    let canon_root =
        std::fs::canonicalize(root).map_err(|e| format!("root not canonicalizable: {e}"))?;
    let canon_dir =
        std::fs::canonicalize(dir).map_err(|e| format!("dir not canonicalizable: {e}"))?;
    if canon_dir == canon_root || !canon_dir.starts_with(&canon_root) {
        return Err(format!(
            "{} is not strictly inside {}",
            canon_dir.display(),
            canon_root.display()
        ));
    }
    std::fs::remove_dir_all(&canon_dir).map_err(|e| e.to_string())
}

/// Sweep `identities_root`'s direct children for orphaned per-account
/// dirs: `compute_and_ensure_account_dir` mints a fresh `<account_id>/`
/// dir and writes real credential files into it BEFORE the login that
/// dir belongs to ever completes, so no `IdentityAccount` row exists yet
/// to reference it. A login that's abandoned, times out, or crashes the
/// app mid-flow leaves that directory behind forever — reagent's finding
/// on #2260: "abandoned/failed attempts leave orphaned directories with
/// no cleanup path."
///
/// Called opportunistically from `compute_and_ensure_account_dir` right
/// before every FRESH mint (never on a reconnect — reusing an existing
/// account's dir needs no sweep). Age-gated via `min_age_secs` so a
/// login that's still genuinely in progress — which legitimately has no
/// DB row yet either — is never swept out from under it; callers should
/// pick a margin comfortably above every frontend poll timeout (5
/// minutes as of this writing) to leave room for a slow user plus retry.
///
/// Best-effort and silent-safe: `identity_get` failures or an unreadable
/// root skip that candidate (or the whole sweep) rather than risk
/// deleting a real, in-use account's dir on a false read. Returns the
/// dirs actually removed, mainly for tests to assert on.
pub fn sweep_orphaned_account_dirs(
    store: &Store,
    identities_root: &Path,
    min_age_secs: u64,
) -> Vec<PathBuf> {
    let mut removed = Vec::new();
    let entries = match std::fs::read_dir(identities_root) {
        Ok(e) => e,
        Err(_) => return removed, // root doesn't exist yet — nothing to sweep
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(account_id) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        match store.identity_get(account_id) {
            // A real, persisted account — never touch it.
            Ok(Some(_)) => continue,
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(
                    account_id,
                    error = %e,
                    "identity.sweep: identity_get failed — skipping candidate rather than risk a false orphan"
                );
                continue;
            }
        }
        let age_secs = match entry.metadata().and_then(|m| m.modified()) {
            Ok(mtime) => SystemTime::now()
                .duration_since(mtime)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            Err(_) => continue, // can't determine age — skip, don't guess
        };
        if age_secs < min_age_secs {
            continue; // could still be a login in progress
        }
        match remove_tree_if_contained(&path, identities_root) {
            Ok(()) => {
                tracing::info!(
                    account_id,
                    dir = %path.display(),
                    age_secs,
                    "identity.sweep: orphaned account dir removed (no matching account row, past age threshold)"
                );
                removed.push(path);
            }
            Err(e) => {
                tracing::debug!(account_id, dir = %path.display(), error = %e, "identity.sweep: candidate not removed");
            }
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acct_with(secret_ref: SecretRef) -> IdentityAccount {
        IdentityAccount {
            id: "acct-test".to_string(),
            name: "asaf-anthropic".to_string(),
            provider: "anthropic".to_string(),
            kind: "oauth".to_string(),
            display_name: String::new(),
            secret_ref,
            context: serde_json::json!({}),
            status: "ok".to_string(),
            created_at: 0,
            updated_at: 0,
        }
    }

    /// OAuthConfigDir inside the identities root → tree removed.
    #[test]
    fn oauth_dir_inside_data_root_is_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("identities");
        let dir = root.join("acct-test").join("claude");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".credentials.json"), "{}").unwrap();

        let acct = acct_with(SecretRef::OAuthConfigDir {
            dir: dir.to_string_lossy().to_string(),
        });
        let out = cleanup_account_secrets(&acct, Some(&root));

        assert!(matches!(out, SecretCleanup::OAuthDirRemoved(_)), "got {out:?}");
        assert!(!dir.exists(), "token dir must be gone");
        // The account's parent folder under the root may remain; only the
        // configured dir tree is removed.
        assert!(root.exists(), "identities root itself must survive");
    }

    /// OAuthConfigDir OUTSIDE the identities root (the legacy `~/.claude`
    /// migration case) → NOT removed, skip outcome.
    #[test]
    fn oauth_dir_outside_data_root_is_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("identities");
        std::fs::create_dir_all(&root).unwrap();
        // A stand-in for the user's global ~/.claude dir.
        let ambient = tmp.path().join("dot-claude");
        std::fs::create_dir_all(&ambient).unwrap();
        std::fs::write(ambient.join(".credentials.json"), "{}").unwrap();

        let acct = acct_with(SecretRef::OAuthConfigDir {
            dir: ambient.to_string_lossy().to_string(),
        });
        let out = cleanup_account_secrets(&acct, Some(&root));

        assert!(
            matches!(out, SecretCleanup::OAuthDirSkipped { .. }),
            "must refuse to delete outside the identities root, got {out:?}"
        );
        assert!(ambient.exists(), "ambient CLI dir must be untouched");
        assert!(ambient.join(".credentials.json").exists());
    }

    /// A `..`-laden path that textually starts under the root but resolves
    /// outside it must also be skipped (canonicalization guard).
    #[test]
    fn oauth_dir_dotdot_escape_is_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("identities");
        std::fs::create_dir_all(&root).unwrap();
        let ambient = tmp.path().join("dot-claude");
        std::fs::create_dir_all(&ambient).unwrap();

        let sneaky = root.join("..").join("dot-claude");
        let acct = acct_with(SecretRef::OAuthConfigDir {
            dir: sneaky.to_string_lossy().to_string(),
        });
        let out = cleanup_account_secrets(&acct, Some(&root));

        assert!(matches!(out, SecretCleanup::OAuthDirSkipped { .. }), "got {out:?}");
        assert!(ambient.exists());
    }

    /// Unresolvable identities root (DataPaths::from_env() = None) → skip,
    /// never guess.
    #[test]
    fn oauth_dir_with_no_root_is_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("identities").join("acct-test").join("claude");
        std::fs::create_dir_all(&dir).unwrap();

        let acct = acct_with(SecretRef::OAuthConfigDir {
            dir: dir.to_string_lossy().to_string(),
        });
        let out = cleanup_account_secrets(&acct, None);

        assert!(matches!(out, SecretCleanup::OAuthDirSkipped { .. }), "got {out:?}");
        assert!(dir.exists(), "dir must be untouched when the root is unknown");
    }

    /// Already-absent dir → absent outcome, nothing created.
    #[test]
    fn oauth_dir_absent_is_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("identities");
        std::fs::create_dir_all(&root).unwrap();
        let dir = root.join("acct-test").join("claude");

        let acct = acct_with(SecretRef::OAuthConfigDir {
            dir: dir.to_string_lossy().to_string(),
        });
        let out = cleanup_account_secrets(&acct, Some(&root));

        assert!(matches!(out, SecretCleanup::OAuthDirAbsent(_)), "got {out:?}");
        assert!(!dir.exists());
    }

    /// Env / dev variants hold no on-host state → no-op, filesystem untouched.
    #[test]
    fn env_and_dev_variants_touch_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("identities");
        let canary = root.join("acct-test").join("claude");
        std::fs::create_dir_all(&canary).unwrap();

        for secret_ref in [
            SecretRef::Env { env_var: "ANTHROPIC_API_KEY".to_string() },
            SecretRef::PlaintextDev { plaintext_dev: "sk-dev".to_string() },
            SecretRef::SecretsManager { sm_path: "path/x".to_string(), sm_json_path: None },
        ] {
            let out = cleanup_account_secrets(&acct_with(secret_ref), Some(&root));
            assert_eq!(out, SecretCleanup::NoOp);
        }
        assert!(canary.exists(), "no filesystem side effects for non-oauth variants");
    }

    /// Keychain variant must never touch the filesystem (outcome depends on
    /// the host's keychain — absent entry deletes idempotently, headless CI
    /// may fail — but both are non-filesystem outcomes).
    #[test]
    fn keychain_variant_touches_no_filesystem() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("identities");
        let canary = root.join("acct-test").join("claude");
        std::fs::create_dir_all(&canary).unwrap();

        // Deliberately unresolvable id so the delete is a guaranteed
        // no-entry (idempotent Ok) on a dev machine's real keychain.
        let mut acct = acct_with(SecretRef::Keychain {
            service: "agentmux".to_string(),
            account: "acct:zz-unit-test-never-provisioned".to_string(),
        });
        acct.id = "zz-unit-test-never-provisioned".to_string();
        let out = cleanup_account_secrets(&acct, Some(&root));

        assert!(
            matches!(out, SecretCleanup::KeychainRemoved | SecretCleanup::KeychainFailed(_)),
            "got {out:?}"
        );
        assert!(canary.exists(), "keychain cleanup must not touch the filesystem");
    }

    fn make_store() -> Store {
        Store::open_in_memory().unwrap()
    }

    /// An orphaned dir (no matching `IdentityAccount` row) past the age
    /// threshold is removed and reported.
    #[test]
    fn sweep_removes_orphaned_dir_past_age_threshold() {
        let store = make_store();
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("identities");
        let orphan = root.join("orphan-uuid").join("claude");
        std::fs::create_dir_all(&orphan).unwrap();
        std::fs::write(orphan.join(".credentials.json"), "{}").unwrap();

        let removed = sweep_orphaned_account_dirs(&store, &root, 0);

        assert_eq!(removed, vec![root.join("orphan-uuid")]);
        assert!(!root.join("orphan-uuid").exists(), "orphaned dir must be gone");
    }

    /// A dir with a real, persisted `IdentityAccount` row is never swept,
    /// no matter the age threshold — this is the guard that keeps an
    /// in-use account's credentials safe.
    #[test]
    fn sweep_never_removes_a_dir_with_a_real_account_row() {
        let store = make_store();
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("identities");
        let real = root.join("real-account-id").join("claude");
        std::fs::create_dir_all(&real).unwrap();

        store
            .identity_upsert(&IdentityAccount {
                id: "real-account-id".to_string(),
                name: "asaf-claude".to_string(),
                provider: "claude".to_string(),
                kind: "oauth".to_string(),
                display_name: String::new(),
                secret_ref: SecretRef::OAuthConfigDir {
                    dir: real.to_string_lossy().to_string(),
                },
                context: serde_json::json!({}),
                status: "valid".to_string(),
                created_at: 0,
                updated_at: 0,
            })
            .unwrap();

        let removed = sweep_orphaned_account_dirs(&store, &root, 0);

        assert!(removed.is_empty());
        assert!(real.exists(), "a real account's dir must survive the sweep");
    }

    /// An orphaned dir younger than `min_age_secs` is left alone — it
    /// could be a login that's still genuinely in progress.
    #[test]
    fn sweep_never_removes_a_dir_younger_than_the_age_threshold() {
        let store = make_store();
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("identities");
        let fresh = root.join("fresh-uuid").join("claude");
        std::fs::create_dir_all(&fresh).unwrap();

        // Effectively "just created" vs. an implausibly high age floor.
        let removed = sweep_orphaned_account_dirs(&store, &root, 999_999);

        assert!(removed.is_empty());
        assert!(root.join("fresh-uuid").exists(), "a fresh in-progress dir must survive");
    }

    /// The identities root not existing yet (fresh install, nothing ever
    /// minted) is a normal state, not an error — empty result, no panic.
    #[test]
    fn sweep_on_missing_root_returns_empty_without_panicking() {
        let store = make_store();
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("identities-never-created");

        let removed = sweep_orphaned_account_dirs(&store, &root, 0);

        assert!(removed.is_empty());
    }
}
