// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One-shot startup migration that seeds a "Default" identity bundle from
//! ambient OAuth credentials living in the user's home dir
//! (`<HOME>/.<auth_dir_name>/.credentials.json` for each oauth-class
//! provider).
//!
//! Per `SPEC_OAUTH_IDENTITY_BUNDLES_2026_05_22.md` §5 (OAuth Bundles PR E):
//! when a user upgrades from a pre-bundle build that wrote OAuth tokens
//! to the legacy ambient location (e.g. Claude Code's `~/.claude/`),
//! AgentMux should auto-detect those credentials and bind them into a
//! "Default" identity bundle so launches keep working without forcing
//! the user to re-OAuth. Empty / `"blank"` `identity_id` rows on
//! `db_agent_instances` are back-filled to point at the new Default
//! bundle so the resolver picks them up at next spawn.
//!
//! ## Idempotency contract
//!
//! The migration is safe to call on every srv startup:
//!
//! 1. For every oauth-class provider in the registry we check whether
//!    ANY identity bundle already has a binding for that provider. If
//!    yes → that provider is already covered (either by a prior run of
//!    this migration, or by a user-driven `auth.start` flow), skip it.
//! 2. The "Default" bundle is upserted (not unconditionally
//!    `INSERT`'d) — running the migration twice produces no extra row.
//! 3. The IdentityAccount uuid is reused on the re-bind path
//!    (`bundle_identity_bindings` lookup), matching the
//!    `persist_oauth_binding_or_synthetic` pattern from PR C, so a
//!    second run doesn't orphan rows in `db_accounts`.
//! 4. Back-fill only runs when the Default bundle exists OR was created
//!    this run — so a fresh install with no ambient creds never
//!    rewrites `identity_id` to a non-existent FK.
//!
//! Failure modes are warn-don't-block (same as `inject_identity_env`):
//! account upsert / bind / publish errors are logged and skipped, the
//! srv keeps coming up.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::backend::providers::{get_provider, get_provider_list};
use crate::backend::storage::store::{
    Identity, IdentityAccount, SecretRef, Store,
};
use crate::backend::wps::{Broker, WaveEvent};
use crate::identity::resolver::{
    oauth_status, probe_oauth_status, provider_class, OAuthProbeStatus, ProviderClass,
};

/// Id used for the seeded Default bundle. Fixed so the migration is
/// idempotent across restarts (a second run looks up by id, sees the
/// row, reuses it instead of minting a new uuid). Not the same as the
/// `"blank"` singleton — the blank bundle has no bindings by contract.
pub const DEFAULT_BUNDLE_ID: &str = "default";

/// Human-readable name for the seeded bundle. Per spec §5.1.
pub const DEFAULT_BUNDLE_NAME: &str = "Default";

/// Summary of what the migration did. Returned for testability + log
/// observability; the production caller only inspects field counts
/// (e.g. logging "0 providers seeded" at info level for visibility).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MigrationStats {
    /// Number of oauth-class providers in the registry we examined.
    pub providers_examined: usize,
    /// Number of providers for which a binding ALREADY existed (in any
    /// bundle) — skipped without touching the ambient creds.
    pub providers_skipped_existing: usize,
    /// Number of providers with no ambient creds on disk — nothing to
    /// seed.
    pub providers_skipped_no_ambient: usize,
    /// Number of providers we successfully bound into the Default bundle.
    pub providers_seeded: usize,
    /// Number of existing Default accounts repointed off the user's ambient
    /// `~/.<provider>` dir to the AgentMux-owned dir (isolation sweep, §4.2).
    pub providers_repointed: usize,
    /// Whether the Default bundle was created (vs. reused) this run.
    pub default_bundle_created: bool,
    /// Count of `db_agent_instances` rows whose `identity_id` was
    /// updated from empty / `"blank"` → Default bundle id.
    pub instances_backfilled: usize,
}

/// Entry point — call once on srv startup, after Store is open and
/// before the srv begins accepting requests. `home_dir_override` is
/// `None` in production (resolves `dirs::home_dir()`); tests use
/// `Some(tempdir)` so they can plant fake `~/.<auth_dir_name>/.credentials.json`
/// files without touching the user's real home.
///
/// Returns the [`MigrationStats`] for logging. Never panics; every
/// internal failure path logs at `warn` and continues.
pub fn run_default_bundle_migration(
    wstore: &Arc<Store>,
    broker: Option<&Arc<Broker>>,
    home_dir_override: Option<PathBuf>,
) -> MigrationStats {
    let mut stats = MigrationStats::default();

    // Resolve `<HOME>`. Skipping when `dirs::home_dir()` fails is the
    // only way the migration becomes a no-op without surfacing an error
    // to the user — without a home dir we couldn't probe ambient creds
    // anyway. Tests pass `Some(tempdir)` to bypass.
    let home = match home_dir_override.or_else(dirs::home_dir) {
        Some(h) => h,
        None => {
            tracing::debug!(
                target: "identity",
                "oauth-bundles migration: no home_dir resolvable — skipping"
            );
            return stats;
        }
    };

    // Enumerate every oauth-class provider in the registry. The match
    // arm `provider_class("claude" | "codex" | "openclaw")` returns
    // `Some(ProviderClass::OAuth { .. })`; everything else returns
    // either `None` (unknown / new) or `ApiKey { .. }`. Iterating the
    // registry (not a hardcoded list) means a new oauth-class provider
    // added to `providers.rs` + `resolver.rs::provider_class` is
    // automatically picked up by this migration on the next release.
    let oauth_providers: Vec<&str> = get_provider_list()
        .filter_map(|p| match provider_class(p.id) {
            Some(ProviderClass::OAuth { .. }) => Some(p.id),
            _ => None,
        })
        .collect();

    stats.providers_examined = oauth_providers.len();

    // Snapshot the existing bindings table ONCE per migration run. We
    // walk every bundle's bindings and tally which providers are
    // already covered — that's the "this provider is already managed,
    // skip" gate. Doing it once (rather than per-provider) keeps the
    // migration O(bundles) instead of O(providers × bundles), matters
    // little in practice (single-digit counts on both axes) but is the
    // clearer shape.
    let covered_providers: std::collections::HashSet<String> = bound_providers(wstore);

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    // Track whether we created the Default bundle this run. Lazy: only
    // do the upsert on the FIRST seedable provider, so a no-op
    // migration (everything covered, or no ambient creds) doesn't even
    // create the Default row.
    let mut default_ready: Option<()> = None;

    for provider_id in oauth_providers {
        // Already-bound check — any bundle that has a binding for this
        // provider counts as covered. The user has already done the
        // OAuth flow once through `auth.start`, OR a prior run of this
        // migration seeded it. Either way, don't touch the ambient
        // creds again.
        if covered_providers.contains(provider_id) {
            stats.providers_skipped_existing += 1;
            tracing::debug!(
                target: "identity",
                provider_id,
                "oauth-bundles migration: provider already bound — skipping"
            );
            continue;
        }

        // Ambient-creds check — `<HOME>/.<auth_dir_name>/.credentials.json`.
        // Use the provider registry's `auth_dir_name` (never hardcode
        // `claude` / `.claude` — that's what makes the migration
        // extensible to codex / openclaw without code changes).
        let provider_cfg = match get_provider(provider_id) {
            Some(p) => p,
            None => {
                // Should be impossible — we enumerated FROM the
                // registry. Belt-and-braces.
                tracing::warn!(
                    target: "identity",
                    provider_id,
                    "oauth-bundles migration: provider missing from registry mid-iteration — skipping"
                );
                continue;
            }
        };
        let ambient_dir = home.join(format!(".{}", provider_cfg.auth_dir_name));
        let creds_file = ambient_dir.join(".credentials.json");
        if !creds_file.exists() {
            stats.providers_skipped_no_ambient += 1;
            tracing::debug!(
                target: "identity",
                provider_id,
                path = %creds_file.display(),
                "oauth-bundles migration: no ambient credentials file — skipping"
            );
            continue;
        }

        // Ambient creds exist — ensure the Default bundle row exists.
        // Lazy upsert: only the first seedable provider triggers it.
        if default_ready.is_none() {
            match ensure_default_bundle(wstore, now_ms) {
                Ok(created) => {
                    stats.default_bundle_created = created;
                    default_ready = Some(());
                }
                Err(e) => {
                    tracing::warn!(
                        target: "identity",
                        error = %e,
                        "oauth-bundles migration: failed to upsert Default bundle — aborting migration this run"
                    );
                    // Bail entirely — without the bundle row we can't
                    // bind anything, and the back-fill below would
                    // point rows at a non-existent FK.
                    return stats;
                }
            }
        }

        // SPEC_PROVIDER_ISOLATION (INV-A/INV-R): the live auth dir must be
        // AgentMux-owned, never the user's ambient `~/.<provider>`. Import the
        // ambient credential into the AgentMux dir ONCE (read-only copy,
        // sentinel-gated), then point the account at the AgentMux dir so the
        // CLI reads + refreshes THERE — `~/.<provider>` is never written and
        // never the live target. (Reverses the #983 pointer-to-ambient.)
        let dest_dir = provider_auth_dir(&home, &provider_cfg.auth_dir_name);
        let dest_str = dest_dir.to_string_lossy().to_string();
        if let Err(e) = import_ambient_once(&creds_file, &dest_dir) {
            tracing::warn!(
                target: "identity",
                provider_id,
                error = %e,
                "oauth-bundles migration: failed to import ambient creds into the AgentMux dir — skipping provider"
            );
            continue;
        }

        // Probe the AgentMux dir (the dir the agent will actually run in), so
        // the seeded binding lands with an accurate status.
        let probed_status = probe_oauth_status(provider_id, &dest_str, now_ms)
            .map(|s| s.as_str())
            .unwrap_or(oauth_status::UNKNOWN);

        // Mirror `persist_oauth_binding_or_synthetic` (PR C): reuse the
        // account_id if a binding already exists, mint a fresh uuid otherwise.
        let account_id = wstore
            .bundle_identity_bindings(DEFAULT_BUNDLE_ID)
            .ok()
            .into_iter()
            .flatten()
            .find(|b| b.provider == provider_id)
            .map(|b| b.account_id)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let account = IdentityAccount {
            id: account_id.clone(),
            name: format!("{provider_id}-oauth"),
            provider: provider_id.to_string(),
            kind: "oauth".to_string(),
            display_name: String::new(),
            // INV-A: AgentMux-owned dir, NOT the user's `~/.<provider>`.
            secret_ref: SecretRef::OAuthConfigDir { dir: dest_str.clone() },
            context: serde_json::json!({}),
            status: probed_status.to_string(),
            created_at: now_ms,
            updated_at: now_ms,
        };

        if let Err(e) = wstore.identity_upsert(&account) {
            tracing::warn!(
                target: "identity",
                provider_id,
                error = %e,
                "oauth-bundles migration: identity_upsert failed — skipping provider"
            );
            continue;
        }
        if let Err(e) = wstore.bundle_identity_bind(DEFAULT_BUNDLE_ID, provider_id, &account_id) {
            tracing::warn!(
                target: "identity",
                provider_id,
                account_id,
                error = %e,
                "oauth-bundles migration: bundle_identity_bind failed — account row persisted but no binding"
            );
            continue;
        }

        stats.providers_seeded += 1;
        tracing::info!(
            target: "identity",
            provider_id,
            account_id,
            dir = %dest_str,
            status = probed_status,
            "oauth-bundles migration: imported ambient credentials into the AgentMux dir + bound into Default bundle"
        );

        // Best-effort broker publish so any open UI refreshes. None in
        // tests; Some(broker) in production. Per spec §5 + the same
        // pattern as `persist_oauth_binding_or_synthetic`.
        if let Some(b) = broker {
            b.publish(WaveEvent {
                event: format!("identitybundlebindings:changed:{DEFAULT_BUNDLE_ID}"),
                scopes: vec![],
                sender: String::new(),
                persist: 0,
                data: None,
            });
        }

        // The probe returned `Valid` / `Expired` / `NeedsReauth` /
        // `None`. Logging here keeps the diagnostic trail intact for
        // the "I just upgraded and my agent has the wrong status"
        // support thread.
        if let Some(probed) = OAuthProbeStatus::from_str(probed_status) {
            tracing::info!(
                target: "identity",
                provider_id,
                ?probed,
                "oauth-bundles migration: probe status"
            );
        }
    }

    // Isolation sweep (SPEC_PROVIDER_ISOLATION §4.2): repoint any pre-existing
    // Default account still pointing at the user's ambient `~/.<provider>`
    // (seeded by the old #983 pointer-to-ambient behaviour) to the
    // AgentMux-owned dir, so existing agents stop reading/refreshing in the
    // user's personal login. Idempotent + sentinel-gated.
    sweep_default_accounts_off_ambient(wstore, broker, &home, now_ms, &mut stats);

    // Back-fill the empty / "blank" identity_id rows on
    // db_agent_instances → Default bundle id. Per spec §5 step 4.
    //
    // Gate: the Default bundle must exist (FK target). It exists if
    // EITHER we seeded a provider in THIS run (`default_ready`) OR a
    // previous run already seeded it. The latter case matters because
    // a legacy row (`identity_id == ""` / `"blank"`) created between
    // restarts — by a code path that still produces them — would
    // otherwise never get repaired: subsequent startups would
    // `providers_skipped_existing` and skip the back-fill too. codex
    // P2 on #983.
    let default_bundle_exists = default_ready.is_some()
        || wstore
            .bundle_identity_list()
            .ok()
            .map(|bs| bs.iter().any(|b| b.id == DEFAULT_BUNDLE_ID))
            .unwrap_or(false);
    if default_bundle_exists {
        match wstore.instance_backfill_identity_id(DEFAULT_BUNDLE_ID) {
            Ok(rows) => {
                stats.instances_backfilled = rows;
                if rows > 0 {
                    tracing::info!(
                        target: "identity",
                        rows,
                        bundle_id = DEFAULT_BUNDLE_ID,
                        "oauth-bundles migration: back-filled empty/blank identity_id rows"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    target: "identity",
                    error = %e,
                    "oauth-bundles migration: instance_backfill_identity_id failed — rows unchanged"
                );
            }
        }
    }

    tracing::info!(
        target: "identity",
        ?stats,
        "oauth-bundles migration: complete"
    );
    stats
}

/// `<home>/.agentmux/shared/providers/<auth_dir_name>` — the AgentMux-owned
/// live auth dir (SPEC_PROVIDER_ISOLATION INV-A). Rooted on the migration's
/// resolved `home` (not a globally-resolved `DataPaths`) so tests using
/// `home_dir_override` stay hermetic. Mirrors `DataPaths::provider_auth_dir`.
fn provider_auth_dir(home: &Path, auth_dir_name: &str) -> PathBuf {
    home.join(".agentmux")
        .join("shared")
        .join("providers")
        .join(auth_dir_name)
}

/// Import the user's ambient credential into the AgentMux dir ONCE — a
/// read-only **copy** of `~/.<provider>/.credentials.json` (never a move,
/// never a write-back to the user's dir). Sentinel-gated
/// (`<dest>/.agentmux-cred-seeded`) so a later logout in the AgentMux dir
/// sticks and we never silently re-import. Returns `Ok(true)` when a copy
/// happened this call. (SPEC_PROVIDER_ISOLATION INV-R.)
fn import_ambient_once(ambient_creds: &Path, dest_dir: &Path) -> std::io::Result<bool> {
    let sentinel = dest_dir.join(".agentmux-cred-seeded");
    if sentinel.exists() {
        return Ok(false);
    }
    std::fs::create_dir_all(dest_dir)?;
    let mut copied = false;
    if ambient_creds.exists() {
        std::fs::copy(ambient_creds, dest_dir.join(".credentials.json"))?;
        copied = true;
    }
    // Mark the AgentMux dir as "taken over" even if there was nothing to copy,
    // so a later fresh login here isn't clobbered by a future ambient import.
    std::fs::write(&sentinel, b"")?;
    Ok(copied)
}

/// Repoint any Default-bundle account still pointing at the user's ambient
/// `~/.<provider>` dir to the AgentMux-owned dir (SPEC_PROVIDER_ISOLATION
/// INV-A / §4.2). Fixes installs seeded by the old #983 pointer-to-ambient
/// behaviour so existing agents stop reading/refreshing in the user's personal
/// login. Idempotent: accounts already pointing at the AgentMux dir (or a
/// custom dir) are untouched; the credential is imported once (sentinel-gated).
fn sweep_default_accounts_off_ambient(
    wstore: &Arc<Store>,
    broker: Option<&Arc<Broker>>,
    home: &Path,
    now_ms: i64,
    stats: &mut MigrationStats,
) {
    let bindings = match wstore.bundle_identity_bindings(DEFAULT_BUNDLE_ID) {
        Ok(b) => b,
        Err(_) => return,
    };
    for b in bindings {
        let provider_cfg = match get_provider(&b.provider) {
            Some(p) => p,
            None => continue,
        };
        let ambient_dir = home.join(format!(".{}", provider_cfg.auth_dir_name));
        let ambient_str = ambient_dir.to_string_lossy().to_string();
        let dest_dir = provider_auth_dir(home, &provider_cfg.auth_dir_name);
        let dest_str = dest_dir.to_string_lossy().to_string();

        let mut acct = match wstore.identity_get(&b.account_id) {
            Ok(Some(a)) => a,
            _ => continue,
        };
        let points_at_ambient = matches!(
            &acct.secret_ref,
            SecretRef::OAuthConfigDir { dir } if *dir == ambient_str
        );
        if !points_at_ambient {
            continue; // already AgentMux-owned (or a custom dir) — leave it
        }

        if let Err(e) = import_ambient_once(&ambient_dir.join(".credentials.json"), &dest_dir) {
            tracing::warn!(
                target: "identity",
                provider = %b.provider,
                error = %e,
                "isolation sweep: import failed — leaving account pointed at ambient"
            );
            continue;
        }
        acct.secret_ref = SecretRef::OAuthConfigDir {
            dir: dest_str.clone(),
        };
        acct.status = probe_oauth_status(&b.provider, &dest_str, now_ms)
            .map(|s| s.as_str())
            .unwrap_or(oauth_status::UNKNOWN)
            .to_string();
        acct.updated_at = now_ms;
        if let Err(e) = wstore.identity_upsert(&acct) {
            tracing::warn!(
                target: "identity",
                provider = %b.provider,
                error = %e,
                "isolation sweep: identity_upsert failed"
            );
            continue;
        }
        stats.providers_repointed += 1;
        tracing::info!(
            target: "identity",
            provider = %b.provider,
            from = %ambient_str,
            to = %dest_str,
            "isolation sweep: repointed Default account off the user's ambient dir to the AgentMux dir"
        );
        if let Some(br) = broker {
            br.publish(WaveEvent {
                event: format!("identitybundlebindings:changed:{DEFAULT_BUNDLE_ID}"),
                scopes: vec![],
                sender: String::new(),
                persist: 0,
                data: None,
            });
        }
    }
}

/// Collect the set of providers that are bound in ANY identity bundle
/// today. The migration's idempotency gate — if a provider is already
/// in here, the ambient-creds seed is a no-op for that provider.
fn bound_providers(wstore: &Arc<Store>) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    let bundles = match wstore.bundle_identity_list() {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(
                target: "identity",
                error = %e,
                "oauth-bundles migration: bundle_identity_list failed — treating all providers as uncovered"
            );
            return out;
        }
    };
    for bundle in bundles {
        match wstore.bundle_identity_bindings(&bundle.id) {
            Ok(bindings) => {
                for b in bindings {
                    out.insert(b.provider);
                }
            }
            Err(e) => {
                tracing::warn!(
                    target: "identity",
                    bundle_id = %bundle.id,
                    error = %e,
                    "oauth-bundles migration: bundle_identity_bindings failed for bundle — skipping"
                );
            }
        }
    }
    out
}

/// Look up the Default bundle by id; create it (via
/// `bundle_identity_upsert`) if missing. Returns `true` when a new row
/// was inserted, `false` when an existing row was reused.
fn ensure_default_bundle(
    wstore: &Arc<Store>,
    now_ms: i64,
) -> Result<bool, crate::backend::storage::error::StoreError> {
    if let Some(existing) = wstore.bundle_identity_get(DEFAULT_BUNDLE_ID)? {
        // Already exists. Don't churn `updated_at` — the bundle is
        // unchanged by this run.
        let _ = existing;
        return Ok(false);
    }
    let identity = Identity {
        id: DEFAULT_BUNDLE_ID.to_string(),
        name: DEFAULT_BUNDLE_NAME.to_string(),
        description: "Seeded from ambient OAuth credentials on first launch.".to_string(),
        is_blank: false,
        created_at: now_ms,
        updated_at: now_ms,
    };
    wstore.bundle_identity_upsert(&identity)?;
    Ok(true)
}

// Small helper to round-trip the probe status string back to the enum,
// purely for the `tracing::info!` line at the seed site. Keeping the
// resolver's enum + constants single-source.
impl OAuthProbeStatus {
    fn from_str(s: &str) -> Option<Self> {
        match s {
            oauth_status::VALID => Some(Self::Valid),
            oauth_status::EXPIRED => Some(Self::Expired),
            oauth_status::NEEDS_REAUTH => Some(Self::NeedsReauth),
            _ => None,
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::store::{IdentityAccount, SecretRef};

    /// Create a fresh in-memory store. The seeded blank-singleton +
    /// schema are already wired up by `Store::open_in_memory`.
    fn make_store() -> Arc<Store> {
        Arc::new(Store::open_in_memory().unwrap())
    }

    /// Plant a Claude-shape ambient credentials file at
    /// `<home>/.claude/.credentials.json` with a future expiry so the
    /// probe reports `Valid`.
    fn plant_ambient_claude_creds(home: &std::path::Path) {
        let dir = home.join(".claude");
        std::fs::create_dir_all(&dir).unwrap();
        // Far-future expiry so the probe doesn't false-positive
        // expired when the test machine's clock has drifted.
        let body = serde_json::json!({
            "claudeAiOauth": {
                "accessToken": "test-access",
                "refreshToken": "test-refresh",
                "expiresAt": 99_999_999_999_999_i64,
            }
        });
        std::fs::write(
            dir.join(".credentials.json"),
            serde_json::to_string(&body).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn no_home_dir_skips_silently() {
        // `home_dir_override = None` here is impossible to set up
        // without affecting the user's real home, so we exercise the
        // empty path via the no-ambient-creds branch (tempdir as
        // home, no files planted). The "no home_dir resolvable" branch
        // is unreachable via the public API short of unsetting HOME on
        // every supported OS — covered by visual inspection.
        let store = make_store();
        let tmp = tempfile::tempdir().unwrap();
        let stats = run_default_bundle_migration(&store, None, Some(tmp.path().to_path_buf()));
        assert!(stats.providers_examined > 0); // claude + codex + openclaw
        assert_eq!(stats.providers_seeded, 0);
        assert_eq!(stats.providers_skipped_no_ambient, stats.providers_examined);
        assert_eq!(stats.providers_skipped_existing, 0);
        assert!(!stats.default_bundle_created);
        assert_eq!(stats.instances_backfilled, 0);
    }

    #[test]
    fn ambient_claude_creds_create_default_bundle_and_bind() {
        let store = make_store();
        let tmp = tempfile::tempdir().unwrap();
        plant_ambient_claude_creds(tmp.path());

        let stats = run_default_bundle_migration(&store, None, Some(tmp.path().to_path_buf()));

        assert!(stats.default_bundle_created);
        assert_eq!(stats.providers_seeded, 1);

        // Default bundle row exists.
        let default = store.bundle_identity_get(DEFAULT_BUNDLE_ID).unwrap();
        assert!(default.is_some(), "Default bundle should be created");
        let default = default.unwrap();
        assert_eq!(default.name, DEFAULT_BUNDLE_NAME);
        assert!(!default.is_blank);

        // Binding for claude.
        let bindings = store.bundle_identity_bindings(DEFAULT_BUNDLE_ID).unwrap();
        assert_eq!(bindings.len(), 1);
        let claude_binding = &bindings[0];
        assert_eq!(claude_binding.provider, "claude");

        // SPEC_PROVIDER_ISOLATION INV-A: the account points at the AgentMux
        // dir, NOT the user's ambient `~/.claude` (T-A1).
        let account = store.identity_get(&claude_binding.account_id).unwrap().unwrap();
        assert_eq!(account.provider, "claude");
        assert_eq!(account.kind, "oauth");
        assert_eq!(account.status, oauth_status::VALID);
        let agentmux_dir = tmp
            .path()
            .join(".agentmux")
            .join("shared")
            .join("providers")
            .join("claude");
        match account.secret_ref {
            SecretRef::OAuthConfigDir { dir } => {
                assert_eq!(dir, agentmux_dir.to_string_lossy());
            }
            other => panic!("expected OAuthConfigDir, got {:?}", other),
        }

        // INV-R: the credential was COPIED into the AgentMux dir (with the
        // import sentinel), and the user's ambient `~/.claude` is untouched.
        let dest_creds = agentmux_dir.join(".credentials.json");
        assert!(dest_creds.exists(), "credential should be copied into the AgentMux dir");
        assert!(
            agentmux_dir.join(".agentmux-cred-seeded").exists(),
            "import sentinel should be written"
        );
        let ambient_creds = tmp.path().join(".claude").join(".credentials.json");
        assert!(ambient_creds.exists(), "ambient ~/.claude creds must remain (read-only import)");
        assert_eq!(
            std::fs::read(&ambient_creds).unwrap(),
            std::fs::read(&dest_creds).unwrap(),
            "copy must be byte-identical to the ambient source"
        );
    }

    #[test]
    fn sweep_repoints_existing_ambient_account_to_agentmux_dir() {
        // Simulate the old #983 state: a Default 'claude' account pointing AT
        // the user's ambient `~/.claude`. The migration's isolation sweep
        // (§4.2) must repoint it to the AgentMux dir (copy once) without a
        // manual re-login — the fix for already-bound agents (Nark/Poal).
        let store = make_store();
        let tmp = tempfile::tempdir().unwrap();
        plant_ambient_claude_creds(tmp.path());
        let ambient_dir = tmp.path().join(".claude");

        // Pre-seed an old-style account + binding pointing AT the ambient dir.
        let legacy = IdentityAccount {
            id: "acct-legacy".to_string(),
            name: "claude-oauth".to_string(),
            provider: "claude".to_string(),
            kind: "oauth".to_string(),
            display_name: String::new(),
            secret_ref: SecretRef::OAuthConfigDir {
                dir: ambient_dir.to_string_lossy().to_string(),
            },
            context: serde_json::json!({}),
            status: oauth_status::VALID.to_string(),
            created_at: 1,
            updated_at: 1,
        };
        store.identity_upsert(&legacy).unwrap();
        ensure_default_bundle(&store, 1).unwrap();
        store
            .bundle_identity_bind(DEFAULT_BUNDLE_ID, "claude", "acct-legacy")
            .unwrap();

        let stats = run_default_bundle_migration(&store, None, Some(tmp.path().to_path_buf()));
        // claude is already bound → the loop skips it; the sweep repoints it.
        assert_eq!(stats.providers_seeded, 0);
        assert_eq!(stats.providers_repointed, 1);

        let updated = store.identity_get("acct-legacy").unwrap().unwrap();
        let agentmux_dir = tmp
            .path()
            .join(".agentmux")
            .join("shared")
            .join("providers")
            .join("claude");
        match updated.secret_ref {
            SecretRef::OAuthConfigDir { dir } => {
                assert_eq!(dir, agentmux_dir.to_string_lossy());
            }
            other => panic!("expected OAuthConfigDir, got {:?}", other),
        }
        assert!(
            agentmux_dir.join(".credentials.json").exists(),
            "creds copied into the AgentMux dir on sweep"
        );

        // Idempotent: a second run finds it already AgentMux-owned → no repoint.
        let s2 = run_default_bundle_migration(&store, None, Some(tmp.path().to_path_buf()));
        assert_eq!(s2.providers_repointed, 0);
    }

    #[test]
    fn idempotent_second_run_is_noop() {
        // Running the migration twice with ambient creds present
        // must NOT produce a second binding row or a second account
        // row. Per spec §5: idempotent across restarts.
        let store = make_store();
        let tmp = tempfile::tempdir().unwrap();
        plant_ambient_claude_creds(tmp.path());

        let s1 = run_default_bundle_migration(&store, None, Some(tmp.path().to_path_buf()));
        assert_eq!(s1.providers_seeded, 1);
        assert!(s1.default_bundle_created);

        let bindings_after_first = store.bundle_identity_bindings(DEFAULT_BUNDLE_ID).unwrap();
        assert_eq!(bindings_after_first.len(), 1);

        let s2 = run_default_bundle_migration(&store, None, Some(tmp.path().to_path_buf()));
        assert_eq!(s2.providers_seeded, 0);
        // Second run reuses the existing bundle row (no churn) and
        // sees the existing binding → providers_skipped_existing.
        assert!(!s2.default_bundle_created);
        assert_eq!(s2.providers_skipped_existing, 1);

        let bindings_after_second = store.bundle_identity_bindings(DEFAULT_BUNDLE_ID).unwrap();
        assert_eq!(bindings_after_second.len(), 1);
        // Same account_id — no orphan.
        assert_eq!(
            bindings_after_first[0].account_id,
            bindings_after_second[0].account_id,
        );
    }

    #[test]
    fn existing_binding_in_other_bundle_skips_provider() {
        // If the user has ALREADY done the auth.start flow and a
        // user-named bundle already binds claude, the migration must
        // NOT also seed the Default bundle for claude — that would
        // double-bind the same provider across bundles. Per spec §5:
        // "If a binding already exists → skip this provider."
        let store = make_store();
        let tmp = tempfile::tempdir().unwrap();
        plant_ambient_claude_creds(tmp.path());

        // Pre-seed a separate bundle that already binds claude.
        let work_bundle = Identity {
            id: "work-bundle".to_string(),
            name: "Work".to_string(),
            description: String::new(),
            is_blank: false,
            created_at: 0,
            updated_at: 0,
        };
        store.bundle_identity_upsert(&work_bundle).unwrap();
        let work_account = IdentityAccount {
            id: "acct-work-claude".to_string(),
            name: "work-claude".to_string(),
            provider: "claude".to_string(),
            kind: "oauth".to_string(),
            display_name: String::new(),
            secret_ref: SecretRef::OAuthConfigDir {
                dir: "/somewhere/work/claude".to_string(),
            },
            context: serde_json::json!({}),
            status: oauth_status::VALID.to_string(),
            created_at: 0,
            updated_at: 0,
        };
        store.identity_upsert(&work_account).unwrap();
        store
            .bundle_identity_bind("work-bundle", "claude", "acct-work-claude")
            .unwrap();

        let stats = run_default_bundle_migration(&store, None, Some(tmp.path().to_path_buf()));

        // Claude was covered by the work bundle → skipped.
        assert_eq!(stats.providers_seeded, 0);
        assert!(stats.providers_skipped_existing >= 1);

        // Default bundle should NOT be created (we only do that on
        // the first seedable provider; with claude skipped and codex
        // / openclaw lacking ambient creds, there's nothing to seed).
        assert!(!stats.default_bundle_created);
        let default = store.bundle_identity_get(DEFAULT_BUNDLE_ID).unwrap();
        assert!(default.is_none(), "Default bundle must not be auto-created when nothing to seed");
    }

    #[test]
    fn backfills_empty_identity_id_rows_after_seed() {
        // The Default bundle exists after seeding → empty /
        // "blank" identity_id rows on db_agent_instances get
        // back-filled to point at it.
        let store = make_store();
        let tmp = tempfile::tempdir().unwrap();
        plant_ambient_claude_creds(tmp.path());

        // Need an agent definition for the FK on db_agent_instances.
        let mut def = crate::backend::storage::store::AgentDefinition {
            id: "def-1".to_string(),
            slug: String::new(),
            name: "T".to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 0,
            agent_type: String::new(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 0,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
        };
        store.agent_def_insert(&mut def).unwrap();

        // Plant two legacy rows — one with empty identity_id, one
        // with the legacy "blank" sentinel. Both should be
        // back-filled.
        let inst_empty = crate::backend::storage::store::AgentInstance {
            id: "inst-empty".to_string(),
            definition_id: "def-1".to_string(),
            parent_instance_id: String::new(),
            block_id: "block-empty".to_string(),
            session_id: String::new(),
            status: "running".to_string(),
            github_context: String::new(),
            started_at: 0,
            ended_at: 0,
            created_at: 0,
            identity_id: String::new(),
            memory_id: String::new(),
            instance_name: String::new(),
            working_directory: String::new(),
            display_hidden: false,
        };
        store.instance_create(&inst_empty).unwrap();

        let inst_blank = crate::backend::storage::store::AgentInstance {
            id: "inst-blank".to_string(),
            definition_id: "def-1".to_string(),
            parent_instance_id: String::new(),
            block_id: "block-blank".to_string(),
            session_id: String::new(),
            status: "running".to_string(),
            github_context: String::new(),
            started_at: 0,
            ended_at: 0,
            created_at: 0,
            identity_id: "blank".to_string(),
            memory_id: String::new(),
            instance_name: String::new(),
            working_directory: String::new(),
            display_hidden: false,
        };
        store.instance_create(&inst_blank).unwrap();

        // Plant a row that ALREADY has a real identity_id — must NOT
        // be touched by the back-fill.
        let inst_set = crate::backend::storage::store::AgentInstance {
            id: "inst-set".to_string(),
            definition_id: "def-1".to_string(),
            parent_instance_id: String::new(),
            block_id: "block-set".to_string(),
            session_id: String::new(),
            status: "running".to_string(),
            github_context: String::new(),
            started_at: 0,
            ended_at: 0,
            created_at: 0,
            identity_id: "some-existing-bundle".to_string(),
            memory_id: String::new(),
            instance_name: String::new(),
            working_directory: String::new(),
            display_hidden: false,
        };
        store.instance_create(&inst_set).unwrap();

        let stats = run_default_bundle_migration(&store, None, Some(tmp.path().to_path_buf()));

        assert!(stats.default_bundle_created);
        assert_eq!(stats.instances_backfilled, 2);

        // Verify the two legacy rows now point at Default.
        let after_empty = store.instance_get("inst-empty").unwrap().unwrap();
        assert_eq!(after_empty.identity_id, DEFAULT_BUNDLE_ID);
        let after_blank = store.instance_get("inst-blank").unwrap().unwrap();
        assert_eq!(after_blank.identity_id, DEFAULT_BUNDLE_ID);
        // The non-empty row stays put.
        let after_set = store.instance_get("inst-set").unwrap().unwrap();
        assert_eq!(after_set.identity_id, "some-existing-bundle");
    }

    #[test]
    fn backfills_legacy_rows_added_between_runs() {
        // Subsequent-startup self-heal — codex P2 on #983: if a
        // legacy row (`identity_id == ""` / `"blank"`) gets created
        // AFTER the first migration seeded Default, the next startup
        // would normally `providers_skipped_existing` for everything
        // and skip the back-fill too. The back-fill gate must check
        // "Default bundle exists" (whether seeded this run OR a prior
        // run), not "did we seed in this run", so the new legacy row
        // gets repaired.
        let store = make_store();
        let tmp = tempfile::tempdir().unwrap();
        plant_ambient_claude_creds(tmp.path());

        // Agent def + initial run that creates Default.
        let mut def = crate::backend::storage::store::AgentDefinition {
            id: "def-1".to_string(),
            slug: String::new(),
            name: "T".to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 0,
            agent_type: String::new(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 0,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
        };
        store.agent_def_insert(&mut def).unwrap();

        let s1 = run_default_bundle_migration(&store, None, Some(tmp.path().to_path_buf()));
        assert!(s1.default_bundle_created);
        assert_eq!(s1.instances_backfilled, 0); // no legacy rows yet

        // Between runs: a code path adds a legacy row with empty
        // identity_id (simulates an older spawn path lingering).
        let inst_late = crate::backend::storage::store::AgentInstance {
            id: "inst-late".to_string(),
            definition_id: "def-1".to_string(),
            parent_instance_id: String::new(),
            block_id: "block-late".to_string(),
            session_id: String::new(),
            status: "running".to_string(),
            github_context: String::new(),
            started_at: 0,
            ended_at: 0,
            created_at: 0,
            identity_id: String::new(),
            memory_id: String::new(),
            instance_name: String::new(),
            working_directory: String::new(),
            display_hidden: false,
        };
        store.instance_create(&inst_late).unwrap();

        // Second run: claude is `providers_skipped_existing`, so
        // `default_ready` stays None — but Default exists from run 1,
        // so the back-fill MUST still run.
        let s2 = run_default_bundle_migration(&store, None, Some(tmp.path().to_path_buf()));
        assert!(!s2.default_bundle_created); // already there
        assert_eq!(
            s2.instances_backfilled, 1,
            "subsequent-run back-fill must repair newly-added legacy rows"
        );
        let after = store.instance_get("inst-late").unwrap().unwrap();
        assert_eq!(after.identity_id, DEFAULT_BUNDLE_ID);
    }

    #[test]
    fn no_ambient_no_default_bundle_no_backfill() {
        // The no-ambient path must not create the Default bundle
        // (FK target would be missing) and must not back-fill rows
        // to a non-existent id. Per spec §5 step 6.
        let store = make_store();
        let tmp = tempfile::tempdir().unwrap();
        // NO `plant_ambient_claude_creds` — empty home.

        // Plant a row with empty identity_id.
        let mut def = crate::backend::storage::store::AgentDefinition {
            id: "def-1".to_string(),
            slug: String::new(),
            name: "T".to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 0,
            agent_type: String::new(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 0,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
        };
        store.agent_def_insert(&mut def).unwrap();
        let inst = crate::backend::storage::store::AgentInstance {
            id: "inst-empty".to_string(),
            definition_id: "def-1".to_string(),
            parent_instance_id: String::new(),
            block_id: "block-empty".to_string(),
            session_id: String::new(),
            status: "running".to_string(),
            github_context: String::new(),
            started_at: 0,
            ended_at: 0,
            created_at: 0,
            identity_id: String::new(),
            memory_id: String::new(),
            instance_name: String::new(),
            working_directory: String::new(),
            display_hidden: false,
        };
        store.instance_create(&inst).unwrap();

        let stats = run_default_bundle_migration(&store, None, Some(tmp.path().to_path_buf()));

        assert_eq!(stats.providers_seeded, 0);
        assert!(!stats.default_bundle_created);
        assert_eq!(stats.instances_backfilled, 0);

        // Row still has empty identity_id — no spurious FK write.
        let after = store.instance_get("inst-empty").unwrap().unwrap();
        assert_eq!(after.identity_id, "");
    }
}
