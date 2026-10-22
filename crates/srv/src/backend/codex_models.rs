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
/// its curated list.
pub fn models_every_account_lists(dirs: impl IntoIterator<Item = PathBuf>) -> Vec<CatalogModel> {
    let mut lists: Vec<(String, Vec<CatalogModel>)> = dirs
        .into_iter()
        .filter_map(|d| std::fs::read_to_string(d.join(MODELS_CACHE_FILE)).ok())
        .filter_map(|s| parse(&s))
        .filter(|(_, models)| !models.is_empty())
        .collect();
    // RFC 3339 timestamps from the same CLI compare correctly as text.
    lists.sort_by(|a, b| b.0.cmp(&a.0));
    let Some(((_, newest), others)) = lists.split_first() else {
        return Vec::new();
    };
    newest
        .iter()
        .filter(|m| others.iter().all(|(_, list)| list.iter().any(|o| o.id == m.id)))
        .cloned()
        .collect()
}

/// Every Codex config dir AgentMux manages: each account's
/// `<identities>/<account>/<auth_dir_name>` and the shared provider dir.
pub fn codex_dirs(identities_dir: &Path, shared_provider_dir: PathBuf, auth_dir_name: &str) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(identities_dir)
        .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path().join(auth_dir_name)).collect())
        .unwrap_or_default();
    dirs.push(shared_provider_dir);
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache(fetched_at: &str, models: &[(&str, &str, i64)]) -> String {
        let models: Vec<serde_json::Value> = models
            .iter()
            .map(|(slug, vis, prio)| {
                serde_json::json!({ "slug": slug, "display_name": slug.to_uppercase(), "visibility": vis, "priority": prio, "context_window": 1 })
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
    fn finds_every_accounts_codex_dir_and_the_shared_one() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("acct-1").join("codex")).unwrap();
        std::fs::create_dir_all(root.path().join("acct-2")).unwrap();
        let shared = root.path().join("providers").join("codex");
        let dirs = codex_dirs(root.path(), shared.clone(), "codex");
        assert!(dirs.contains(&root.path().join("acct-1").join("codex")));
        assert!(dirs.contains(&shared));
    }
}
