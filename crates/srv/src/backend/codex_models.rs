// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The models a Codex account can use, from the list the Codex CLI fetches
//! from OpenAI and caches in `$CODEX_HOME/models_cache.json`. AgentMux's own
//! list went stale: on 2026-10-22 OpenAI no longer accepted any model it
//! offered for a ChatGPT account. Read locally, no network call.
//! SPEC_CODEX_OAUTH_IN_CONNECTORS_AND_EMAIL_LABELS_2026_10_22.md §4.

use std::path::{Path, PathBuf};

use crate::backend::model_catalog::CatalogModel;

/// The cache file's name in a Codex config dir.
pub const MODELS_CACHE_FILE: &str = "models_cache.json";
/// What `codex login` writes in a Codex config dir.
const AUTH_FILE: &str = "auth.json";

#[derive(serde::Deserialize)]
struct Cache {
    #[serde(default)]
    fetched_at: String,
    #[serde(default)]
    models: Vec<CachedModel>,
}

#[derive(serde::Deserialize)]
struct CachedModel {
    #[serde(default)]
    slug: String,
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    visibility: String,
    #[serde(default)]
    priority: i64,
    #[serde(default)]
    default_reasoning_level: Option<String>,
    #[serde(default)]
    supported_reasoning_levels: Vec<ReasoningLevel>,
}

#[derive(serde::Deserialize)]
struct ReasoningLevel {
    #[serde(default)]
    effort: String,
}

/// The listed models of one cache, in the CLI's own order (`priority`), and
/// when it was fetched. Hidden entries (internal review models) are left out.
fn parse(contents: &str) -> Option<(String, Vec<CatalogModel>)> {
    let cache: Cache = serde_json::from_str(contents).ok()?;
    let mut listed: Vec<CachedModel> = cache
        .models
        .into_iter()
        .filter(|m| m.visibility == "list" && !m.slug.is_empty())
        .collect();
    listed.sort_by_key(|m| m.priority);
    let models = listed
        .into_iter()
        .map(|m| CatalogModel {
            display_name: if m.display_name.is_empty() { m.slug.clone() } else { m.display_name },
            id: m.slug,
            efforts: m.supported_reasoning_levels.into_iter().map(|l| l.effort).filter(|e| !e.is_empty()).collect(),
            default_effort: m.default_reasoning_level.filter(|e| !e.is_empty()),
        })
        .collect();
    Some((cache.fetched_at, models))
}

/// The models every Codex account here can run: those listed in each account's
/// cache, in the order of the most recently fetched one. The menu and a new
/// pane's default are shared by all Codex panes, so offering one account's
/// list would let a pane bound to another account (a different plan) pick a
/// model it can't run. Empty when no account has a list yet (a fresh sign-in
/// before its first turn) or the lists share nothing; the frontend then keeps
/// its curated list. A signed-in account (an `auth.json`) without a list yet
/// makes the common list unknown too: the others' list might not suit it.
/// Directories with neither (another provider's account) are ignored.
pub fn models_every_account_lists(dirs: impl IntoIterator<Item = PathBuf>) -> Vec<CatalogModel> {
    let mut lists: Vec<(String, Vec<CatalogModel>)> = Vec::new();
    for d in dirs {
        let list = std::fs::read_to_string(d.join(MODELS_CACHE_FILE))
            .ok()
            .and_then(|s| parse(&s))
            .filter(|(_, models)| !models.is_empty());
        match list {
            Some(l) => lists.push(l),
            None if d.join(AUTH_FILE).is_file() => return Vec::new(),
            None => {}
        }
    }
    // RFC 3339 timestamps from the same CLI compare correctly as text.
    lists.sort_by(|a, b| b.0.cmp(&a.0));
    let Some(((_, newest), others)) = lists.split_first() else {
        return Vec::new();
    };
    newest
        .iter()
        .filter_map(|m| {
            let same: Vec<&CatalogModel> =
                others.iter().map(|(_, list)| list.iter().find(|o| o.id == m.id)).collect::<Option<_>>()?;
            // Effort is passed to whichever account a pane is bound to, so only
            // the levels every account takes for this model, and a default only
            // when they all have the same one.
            let mut model = m.clone();
            model.efforts.retain(|e| same.iter().all(|o| o.efforts.contains(e)));
            if same.iter().any(|o| o.default_effort != model.default_effort)
                || model.default_effort.as_ref().is_some_and(|d| !model.efforts.contains(d))
            {
                model.default_effort = None;
            }
            Some(model)
        })
        .collect()
}

/// A sign-in is starting in this Codex config dir: drop its cached model list.
/// A reconnect may be another ChatGPT account or plan, and the old list would
/// otherwise stand for it until the CLI fetches a new one; without it the
/// account's models are unknown, so a pane passes no `--model` and the CLI
/// picks one the account offers.
pub fn forget_models_on_sign_in(dir: &Path) {
    match std::fs::remove_file(dir.join(MODELS_CACHE_FILE)) {
        Ok(()) => tracing::info!(target: "identity", dir = %dir.display(), "codex sign-in: dropped the old model list"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!(target: "identity", dir = %dir.display(), error = %e, "codex sign-in: could not drop the old model list"),
    }
}

/// Every account's Codex config dir: `<identities>/<account>/<auth_dir_name>`.
/// Not the old shared provider dir (`shared/providers/codex`): no account uses
/// it, and its `auth.json` without a model list made the common list unknown
/// for good, so Codex panes got no `--model` and the curated menu (AgentA,
/// found testing #5323).
pub fn codex_dirs(identities_dir: &Path, auth_dir_name: &str) -> Vec<PathBuf> {
    std::fs::read_dir(identities_dir)
        .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path().join(auth_dir_name)).collect())
        .unwrap_or_default()
}

/// The ids every Codex account on this install lists, in order: what a pane
/// srv opens itself (`agent.open`) may start on. Empty when no account has a
/// list yet, when data paths can't be resolved, and always in tests, so a test
/// never reads this machine's real accounts.
pub fn installed_common_model_ids() -> Vec<String> {
    #[cfg(test)]
    {
        Vec::new()
    }
    #[cfg(not(test))]
    {
        let Some(paths) = agentmux_common::DataPaths::from_env() else {
            return Vec::new();
        };
        let Some(provider) = crate::backend::providers::get_provider("codex") else {
            return Vec::new();
        };
        let dirs = codex_dirs(&paths.identities_dir(), provider.auth_dir_name);
        models_every_account_lists(dirs).into_iter().map(|m| m.id).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache(fetched_at: &str, models: &[(&str, &str, i64)]) -> String {
        let models: Vec<serde_json::Value> = models
            .iter()
            .map(|(slug, vis, prio)| {
                serde_json::json!({
                    "slug": slug, "display_name": slug.to_uppercase(), "visibility": vis, "priority": prio, "context_window": 1,
                    "default_reasoning_level": "medium",
                    "supported_reasoning_levels": [
                        { "effort": "low", "description": "Fast" },
                        { "effort": "medium", "description": "Balanced" },
                        { "effort": "ultra", "description": "Deepest" }
                    ]
                })
            })
            .collect();
        serde_json::json!({ "fetched_at": fetched_at, "client_version": "0.154.0", "models": models }).to_string()
    }

    #[test]
    fn lists_the_visible_models_in_the_clis_order() {
        let (_, models) = parse(&cache(
            "2026-10-22T06:47:36Z",
            &[("gpt-5.6-sol", "list", 6), ("hidden-review", "hide", 1), ("gpt-6-astra", "list", 2)],
        ))
        .unwrap();
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["gpt-6-astra", "gpt-5.6-sol"]);
        assert_eq!(models[0].display_name, "GPT-6-ASTRA");
    }

    /// Each model's own efforts and default, for the composer's Effort menu.
    #[test]
    fn keeps_each_models_efforts_and_default() {
        let (_, models) = parse(&cache("2026-10-22T06:47:36Z", &[("gpt-6-astra", "list", 2)])).unwrap();
        assert_eq!(models[0].efforts, ["low", "medium", "ultra"]);
        assert_eq!(models[0].default_effort.as_deref(), Some("medium"));
        // A cache from before the CLI listed levels: none, and no default.
        let bare = serde_json::json!({ "fetched_at": "x", "models": [{ "slug": "m", "visibility": "list" }] }).to_string();
        let (_, models) = parse(&bare).unwrap();
        assert!(models[0].efforts.is_empty());
        assert_eq!(models[0].default_effort, None);
    }

    #[test]
    fn offers_only_what_every_account_lists_in_the_newest_lists_order_and_skips_dirs_without_a_cache() {
        let root = tempfile::tempdir().unwrap();
        let (plus, pro, none) = (root.path().join("a"), root.path().join("b"), root.path().join("c"));
        for d in [&plus, &pro, &none] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(
            plus.join(MODELS_CACHE_FILE),
            cache("2026-10-21T00:00:00Z", &[("gpt-5.6-sol", "list", 6), ("gpt-6-astra", "list", 2)]),
        )
        .unwrap();
        std::fs::write(
            pro.join(MODELS_CACHE_FILE),
            cache("2026-10-22T00:00:00Z", &[("gpt-6-astra", "list", 2), ("gpt-6-pro", "list", 1), ("gpt-5.6-sol", "list", 6)]),
        )
        .unwrap();
        let ids = |models: Vec<CatalogModel>| models.into_iter().map(|m| m.id).collect::<Vec<_>>();
        // gpt-6-pro is the Pro account's alone: a pane on the Plus account can't run it.
        assert_eq!(ids(models_every_account_lists([plus.clone(), none, pro.clone()])), ["gpt-6-astra", "gpt-5.6-sol"]);
        // One account: its own list.
        assert_eq!(ids(models_every_account_lists([pro])), ["gpt-6-pro", "gpt-6-astra", "gpt-5.6-sol"]);
    }

    /// One shared list serves every Codex pane, whichever account it is bound
    /// to, so a level is offered only when every account takes it.
    #[test]
    fn offers_only_the_effort_levels_every_account_takes() {
        let root = tempfile::tempdir().unwrap();
        let (a, b) = (root.path().join("a"), root.path().join("b"));
        for d in [&a, &b] {
            std::fs::create_dir_all(d).unwrap();
        }
        let with_levels = |fetched_at: &str, levels: &[&str], default: &str| {
            let levels: Vec<serde_json::Value> = levels.iter().map(|l| serde_json::json!({ "effort": l })).collect();
            serde_json::json!({ "fetched_at": fetched_at, "models": [{
                "slug": "gpt-6-astra", "visibility": "list", "priority": 1,
                "default_reasoning_level": default, "supported_reasoning_levels": levels
            }] })
            .to_string()
        };
        std::fs::write(a.join(MODELS_CACHE_FILE), with_levels("2026-10-22T00:00:00Z", &["low", "medium", "high", "ultra"], "low")).unwrap();
        std::fs::write(b.join(MODELS_CACHE_FILE), with_levels("2026-10-21T00:00:00Z", &["low", "medium", "high"], "low")).unwrap();
        let models = models_every_account_lists([a.clone(), b.clone()]);
        assert_eq!(models[0].efforts, ["low", "medium", "high"], "no ultra: account b's model doesn't take it");
        assert_eq!(models[0].default_effort.as_deref(), Some("low"), "both default to low");
        // Defaults that differ: none is claimed, and the CLI uses each account's own.
        std::fs::write(b.join(MODELS_CACHE_FILE), with_levels("2026-10-21T00:00:00Z", &["low", "medium", "high"], "medium")).unwrap();
        assert_eq!(models_every_account_lists([a, b])[0].default_effort, None);
    }

    #[test]
    fn a_signed_in_account_without_a_list_yet_makes_the_common_list_unknown() {
        let root = tempfile::tempdir().unwrap();
        let (old, fresh, other) = (root.path().join("a"), root.path().join("b"), root.path().join("c"));
        for d in [&old, &fresh, &other] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(old.join(MODELS_CACHE_FILE), cache("2026-10-22T00:00:00Z", &[("gpt-6-pro", "list", 1)])).unwrap();
        // `other` has neither file: not a Codex sign-in, ignored.
        assert_eq!(models_every_account_lists([old.clone(), fresh.clone(), other]).len(), 1);
        // `fresh` signed in but hasn't run a turn: its models are unknown.
        std::fs::write(fresh.join(AUTH_FILE), "{}").unwrap();
        assert!(models_every_account_lists([old, fresh]).is_empty());
    }

    #[test]
    fn a_sign_in_forgets_the_old_list() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(MODELS_CACHE_FILE), cache("2026-10-22T00:00:00Z", &[("gpt-6-pro", "list", 1)])).unwrap();
        std::fs::write(root.path().join(AUTH_FILE), "{}").unwrap();
        forget_models_on_sign_in(root.path());
        assert!(!root.path().join(MODELS_CACHE_FILE).exists());
        assert!(root.path().join(AUTH_FILE).exists(), "only the list goes");
        forget_models_on_sign_in(root.path()); // nothing left: no error
        assert!(models_every_account_lists([root.path().to_path_buf()]).is_empty(), "unknown until the CLI fetches it again");
    }

    #[test]
    fn lists_that_share_nothing_offer_nothing() {
        let root = tempfile::tempdir().unwrap();
        let (a, b) = (root.path().join("a"), root.path().join("b"));
        for (d, id) in [(&a, "gpt-5.5"), (&b, "gpt-6-astra")] {
            std::fs::create_dir_all(d).unwrap();
            std::fs::write(d.join(MODELS_CACHE_FILE), cache("2026-10-22T00:00:00Z", &[(id, "list", 1)])).unwrap();
        }
        assert!(models_every_account_lists([a, b]).is_empty());
    }

    #[test]
    fn nothing_cached_or_unreadable_is_an_empty_list() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(MODELS_CACHE_FILE), "not json").unwrap();
        assert!(models_every_account_lists([root.path().to_path_buf(), root.path().join("missing")]).is_empty());
    }

    #[test]
    fn finds_every_accounts_codex_dir_but_not_the_old_shared_one() {
        let root = tempfile::tempdir().unwrap();
        let identities = root.path().join("identities");
        std::fs::create_dir_all(identities.join("acct-1").join("codex")).unwrap();
        std::fs::create_dir_all(identities.join("acct-2")).unwrap();
        let dirs = codex_dirs(&identities, "codex");
        assert!(dirs.contains(&identities.join("acct-1").join("codex")));
        assert!(!dirs.iter().any(|d| d.starts_with(root.path().join("providers"))), "the shared provider dir is no account's");
    }
}
