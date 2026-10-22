// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The agent's identity variables in its spawn environment, beyond what
//! `input.rs`'s `split_agent_identity_env` and `carry_agent_uid_env` set.
//!
//! **The server wins on identity** (`SPEC_FORK_SLUG_MIGRATION_2026_10_22.md`
//! P0.3): when the agent's row has a slug, `AGENTMUX_AGENT_ID`,
//! `MUXBUS_AGENT_ID` and the git identity AgentMux derived from the id come
//! from that slug, whatever a persisted `cmd:env` says. `cmd:env` is a
//! launch-time snapshot, so after a rename it names the old id, and the agent
//! would keep answering to it. `AGENTMUX_AGENT_SLUG` is set the same way
//! (`split_agent_identity_env`), and `GH_CONFIG_DIR` follows the slug
//! (`gh_guard`). An agent with no row or no slug keeps its `cmd:env` values.

use std::collections::HashMap;

/// Per-agent git commit identity env vars for a spawned agent process
/// (2026-08-22, docs/retro/retro-shared-git-identity-committer-misattribution-2026-08-22.md).
///
/// Without this, `git commit` falls through to whatever user.name/
/// user.email happens to be sitting in a shared multi-agent host's
/// `~/.gitconfig` -- one agent's real identity, silently baked into every
/// OTHER agent's commits too. `agentmux-cloud`'s review-notification
/// consumer resolves "who committed this" via GitHub's own commit->account
/// auto-linking (`commit.author.login`), not the raw git commit metadata
/// directly -- so whichever agent's *real, GitHub-verified* email is
/// sitting in the shared config silently receives every other agent's
/// committer notifications too (confirmed live: every commit across 9+
/// PRs from 4 different agents on one host resolved to a single agent's
/// account this way).
///
/// `GIT_AUTHOR_*`/`GIT_COMMITTER_*` env vars take precedence over
/// config-file `user.name`/`user.email` at commit time (git's own
/// behavior) -- scoped to just the spawned process tree, same pattern as
/// `AGENTMUX_AGENT_ID`. The email intentionally uses a non-existent
/// `.local` domain: GitHub can't link it to any real account, so an agent
/// with no dedicated PAT registered (the common case -- see this repo's
/// `CLAUDE.md`, "Which GitHub account am I acting as?") simply drops out
/// of `commit.author.login` lookups entirely instead of resolving to
/// someone else's real, verified identity.
pub(crate) fn git_identity_env_vars(agent_id: &str) -> [(&'static str, String); 4] {
    let git_email = format!("{}@agentmux.local", agent_id.to_lowercase());
    [
        ("GIT_AUTHOR_NAME", agent_id.to_string()),
        ("GIT_COMMITTER_NAME", agent_id.to_string()),
        ("GIT_AUTHOR_EMAIL", git_email.clone()),
        ("GIT_COMMITTER_EMAIL", git_email),
    ]
}

/// The slug's whole identity on an env built outside
/// `build_persistent_spawn_env` (an ACP or App Server spawn, the agent's Shell
/// and PTY shell): `AGENTMUX_AGENT_SLUG`, then [`carry_agent_identity_env`].
pub(crate) fn carry_slug_identity_env(env_vars: &mut HashMap<String, String>, slug: &str) {
    super::input::split_agent_identity_env(env_vars, "", Some(slug));
    carry_agent_identity_env(env_vars, "", Some(slug));
}

/// The definition's slug for the agent on `block_id`, if its row has one.
/// For an env built from the agent's stored `cmd:env` outside a spawn, so
/// it skips the token mint and key ensure a spawn's lookup does. A store
/// error reads as no slug.
pub(crate) fn block_agent_slug(mstore: &crate::backend::storage::store::Store, block_id: &str) -> Option<String> {
    let instance = mstore.instance_get_active_for_block(block_id).ok().flatten()?;
    let def = mstore.agent_def_get(&instance.definition_id).ok().flatten()?;
    Some(def.slug).filter(|s| !s.trim().is_empty())
}

/// [`carry_slug_identity_env`] for the agent on `block_id`; nothing for a
/// block with no row slug.
pub(crate) fn carry_block_slug_identity_env(
    mstore: &crate::backend::storage::store::Store,
    block_id: &str,
    env_vars: &mut HashMap<String, String>,
) {
    if let Some(slug) = block_agent_slug(mstore, block_id) {
        carry_slug_identity_env(env_vars, &slug);
    }
}

/// The agent's id and git identity: [`carry_agent_id_env`], then
/// [`carry_git_identity_env`] with the id it replaced.
pub(crate) fn carry_agent_identity_env(
    env_vars: &mut HashMap<String, String>,
    display_name: &str,
    persisted_slug: Option<&str>,
) {
    let replaced = carry_agent_id_env(env_vars, display_name, persisted_slug);
    carry_git_identity_env(env_vars, replaced.as_deref());
}

/// `AGENTMUX_AGENT_ID` and its `MUXBUS_AGENT_ID` mirror. It is the
/// canonical, app-wide agent-identity variable (MCP tool routing, native
/// memory, shell OSC titling, jekt auto-registration, gh-agent; see this
/// repo's CLAUDE.md Naming Conventions table). With a row slug it is that
/// slug, overriding `cmd:env` (P0.3, see the module doc): the slug is what
/// every launch path sets and what the name-keyed signing keys are filed
/// under (identity M4d, `agent_self_keys.rs`). Without one, a `cmd:env` value
/// is kept, and a pane with no row (quick launch) falls back to its display
/// name as a route name, since a name that isn't a valid agent id (a fork's
/// "Loap #3") fails every registration
/// (REPORT_FORK_AGENT_ISOLATION_2026_10_12.md F3). `MUXBUS_AGENT_ID` mirrors
/// it so muxbus-client picks it up under the prefix it checks first; scoped to
/// that one hand-off point, not a second source of truth (ARCH-002).
///
/// Returns the `cmd:env` `AGENTMUX_AGENT_ID` the slug replaced, if it did.
pub(crate) fn carry_agent_id_env(
    env_vars: &mut HashMap<String, String>,
    display_name: &str,
    persisted_slug: Option<&str>,
) -> Option<String> {
    if let Some(slug) = persisted_slug.map(str::trim).filter(|s| !s.is_empty()) {
        let replaced = set_server_value(env_vars, "AGENTMUX_AGENT_ID", slug);
        set_server_value(env_vars, "MUXBUS_AGENT_ID", slug);
        return replaced;
    }
    if !env_vars.contains_key("AGENTMUX_AGENT_ID") && !display_name.trim().is_empty() {
        env_vars.insert("AGENTMUX_AGENT_ID".to_string(), agentmux_common::slug::agent_route_name(display_name));
    }
    if !env_vars.contains_key("MUXBUS_AGENT_ID") {
        if let Some(agent_id) = env_vars.get("AGENTMUX_AGENT_ID").cloned() {
            env_vars.insert("MUXBUS_AGENT_ID".to_string(), agent_id);
        }
    }
    None
}

/// Set `key` to the server's `value`, logging when that changes what
/// `cmd:env` carried. Returns the replaced value.
fn set_server_value(env_vars: &mut HashMap<String, String>, key: &str, value: &str) -> Option<String> {
    let previous = env_vars.insert(key.to_string(), value.to_string());
    let replaced = previous.filter(|p| !p.trim().is_empty() && p != value)?;
    tracing::info!(key, cmd_env = %replaced, server = %value, "spawn env: persisted identity differs from the definition; the definition's wins");
    Some(replaced)
}

/// The git identity: each of [`git_identity_env_vars`] that `cmd:env` lacks
/// is set from `AGENTMUX_AGENT_ID`. One it carries is the user's to keep,
/// unless it names `replaced`, the stale id [`carry_agent_id_env`] overrode:
/// a name equal to it, or an email whose local part is it (the launch writes
/// `<slug>@agents.local`), is rewritten to the current id, keeping the email's
/// domain.
pub(crate) fn carry_git_identity_env(env_vars: &mut HashMap<String, String>, replaced: Option<&str>) {
    let Some(agent_id) = env_vars.get("AGENTMUX_AGENT_ID").cloned() else {
        return;
    };
    for (key, value) in git_identity_env_vars(&agent_id) {
        let Some(existing) = env_vars.get(key) else {
            env_vars.insert(key.to_string(), value);
            continue;
        };
        let Some(stale) = replaced else { continue };
        let rewritten = match existing.split_once('@') {
            Some((local, domain)) if key.ends_with("_EMAIL") && local.eq_ignore_ascii_case(stale) => {
                format!("{}@{domain}", agent_id.to_lowercase())
            }
            _ if existing == stale => agent_id.clone(),
            _ => continue,
        };
        set_server_value(env_vars, key, &rewritten);
    }
}

#[cfg(test)]
mod tests {
    use super::{carry_agent_id_env, carry_agent_identity_env, git_identity_env_vars};
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    /// With no `AGENTMUX_AGENT_ID` in `cmd:env`, the row's slug: what the
    /// launch sets and the signing keys are filed under. A fork's display
    /// name is never used when the row is known.
    #[test]
    fn a_missing_agent_id_is_the_rows_slug() {
        let mut e = env(&[]);
        carry_agent_id_env(&mut e, "Loap #3", Some("loap-loap-2-loap-3"));
        assert_eq!(e["AGENTMUX_AGENT_ID"], "loap-loap-2-loap-3");
        assert_eq!(e["MUXBUS_AGENT_ID"], "loap-loap-2-loap-3");
    }

    /// A pane with no row (quick launch) falls back to its display name, as
    /// a valid id: "Loap #3" used to fail every registration.
    #[test]
    fn a_rowless_panes_agent_id_is_its_route_name() {
        let mut e = env(&[]);
        carry_agent_id_env(&mut e, "Loap #3", None);
        assert_eq!(e["AGENTMUX_AGENT_ID"], "Loap-3");
        let mut plain = env(&[]);
        carry_agent_id_env(&mut plain, "AgentY", None);
        assert_eq!(plain["AGENTMUX_AGENT_ID"], "AgentY", "a valid name keeps its casing");
        let mut none = env(&[]);
        carry_agent_id_env(&mut none, "  ", None);
        assert!(!none.contains_key("AGENTMUX_AGENT_ID"));
    }

    /// With no row slug, a launch's own value wins, and its mirror is kept.
    #[test]
    fn without_a_slug_a_carried_agent_id_is_kept() {
        let mut e = env(&[("AGENTMUX_AGENT_ID", "agenty-2"), ("MUXBUS_AGENT_ID", "bus-y")]);
        assert_eq!(carry_agent_id_env(&mut e, "AgentY", None), None);
        assert_eq!(e["AGENTMUX_AGENT_ID"], "agenty-2");
        assert_eq!(e["MUXBUS_AGENT_ID"], "bus-y");
        let mut blank = env(&[("AGENTMUX_AGENT_ID", "agenty-2")]);
        carry_agent_id_env(&mut blank, "AgentY", Some("  "));
        assert_eq!(blank["AGENTMUX_AGENT_ID"], "agenty-2", "a blank slug is no slug");
    }

    /// P0.3: a persisted `cmd:env` from before a rename loses to the
    /// definition's slug: the id, its mirror, and the git identity the launch
    /// derived from the old id. A git identity of the user's own is kept.
    #[test]
    fn a_persisted_identity_loses_to_the_definition() {
        let mut e = env(&[
            ("AGENTMUX_AGENT_ID", "loap-loap-2-loap-3"),
            ("MUXBUS_AGENT_ID", "loap-loap-2-loap-3"),
            ("GIT_AUTHOR_NAME", "Loap #3"),
            ("GIT_AUTHOR_EMAIL", "loap-loap-2-loap-3@agents.local"),
            ("GIT_COMMITTER_NAME", "loap-loap-2-loap-3"),
            ("GIT_COMMITTER_EMAIL", "Loap-Loap-2-Loap-3@agents.local"),
        ]);
        carry_agent_identity_env(&mut e, "Loap #3", Some("loap-3"));
        assert_eq!(e["AGENTMUX_AGENT_ID"], "loap-3");
        assert_eq!(e["MUXBUS_AGENT_ID"], "loap-3");
        assert_eq!(e["GIT_AUTHOR_NAME"], "Loap #3", "the display name is not the id");
        assert_eq!(e["GIT_AUTHOR_EMAIL"], "loap-3@agents.local");
        assert_eq!(e["GIT_COMMITTER_NAME"], "loap-3");
        assert_eq!(e["GIT_COMMITTER_EMAIL"], "loap-3@agents.local");

        let mut own = env(&[
            ("AGENTMUX_AGENT_ID", "old-id"),
            ("GIT_AUTHOR_NAME", "Jane Doe"),
            ("GIT_AUTHOR_EMAIL", "jane@example.com"),
        ]);
        carry_agent_identity_env(&mut own, "Jane", Some("new-id"));
        assert_eq!(own["GIT_AUTHOR_NAME"], "Jane Doe");
        assert_eq!(own["GIT_AUTHOR_EMAIL"], "jane@example.com");
        assert_eq!(own["GIT_COMMITTER_EMAIL"], "new-id@agentmux.local", "a missing one is filled");
    }

    /// No row: everything `cmd:env` carries stays, and the gaps are filled
    /// from its own id, as before.
    #[test]
    fn an_agent_with_no_slug_keeps_its_identity() {
        let mut e = env(&[("AGENTMUX_AGENT_ID", "Korp"), ("GIT_AUTHOR_EMAIL", "korp@agents.local")]);
        carry_agent_identity_env(&mut e, "Korp", None);
        assert_eq!(e["AGENTMUX_AGENT_ID"], "Korp");
        assert_eq!(e["MUXBUS_AGENT_ID"], "Korp");
        assert_eq!(e["GIT_AUTHOR_EMAIL"], "korp@agents.local");
        assert_eq!(e["GIT_COMMITTER_EMAIL"], "korp@agentmux.local");
        assert_eq!(e["GIT_AUTHOR_NAME"], "Korp");
    }

    /// P0.3 through the store, as a spawn and the agent's Shell build their
    /// env: the agent's shared definition moved on to `loap-3` while this
    /// channel's row and the block's persisted `cmd:env` still say the old
    /// slug. The id, slug, MuxBus id, git email and `GH_CONFIG_DIR` all follow
    /// the definition. A block with no row keeps its `cmd:env`.
    #[test]
    fn a_block_takes_the_definitions_identity_over_its_persisted_env() {
        use crate::backend::storage::store::Store;
        let store = Store::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let defs = std::sync::Arc::new(crate::registry::DefinitionStore::open(tmp.path().join("definitions")).unwrap());
        store.set_def_registry(defs.clone());
        let mut def = crate::backend::storage::agents::test_agent_def("uid-p03", "Loap #3", "claude", "host", 1, "");
        def.slug = "loap-loap-2-loap-3".to_string();
        store.agent_def_insert(&mut def).unwrap();
        let mut rec = defs.get("uid-p03").unwrap().unwrap();
        rec.data.slug = "loap-3".to_string();
        std::fs::write(defs.root().join("uid-p03.json"), serde_json::to_vec(&rec).unwrap()).unwrap();
        for (block_id, agent_id) in [("block-p03", "uid-p03"), ("block-p03-none", "claude")] {
            let mut meta = crate::backend::obj::MetaMapType::new();
            meta.insert("view".to_string(), serde_json::json!("agent"));
            meta.insert("agentId".to_string(), serde_json::json!(agent_id));
            let mut block = crate::backend::obj::Block { oid: block_id.to_string(), meta, ..Default::default() };
            store.insert(&mut block).unwrap();
        }
        let old = "loap-loap-2-loap-3";
        let stale = env(&[
            ("AGENTMUX_AGENT_ID", old),
            ("AGENTMUX_AGENT_SLUG", old),
            ("MUXBUS_AGENT_ID", old),
            ("GIT_AUTHOR_EMAIL", "loap-loap-2-loap-3@agents.local"),
            ("GH_CONFIG_DIR", "/somewhere/gh-loap-loap-2-loap-3"),
        ]);
        let home = tempfile::tempdir().unwrap();

        let mut spawn = stale.clone();
        crate::server::agent_handlers::input::carry_block_identity_env(&store, "block-p03", &mut spawn);
        let mut shell = stale.clone();
        super::carry_block_slug_identity_env(&store, "block-p03", &mut shell);
        for e in [&mut spawn, &mut shell] {
            crate::backend::gh_guard::apply_gh_guard_in(e, home.path());
            for key in ["AGENTMUX_AGENT_ID", "AGENTMUX_AGENT_SLUG", "MUXBUS_AGENT_ID"] {
                assert_eq!(e[key], "loap-3", "{key}");
            }
            assert_eq!(e["GIT_AUTHOR_EMAIL"], "loap-3@agents.local");
            assert_eq!(std::path::PathBuf::from(&e["GH_CONFIG_DIR"]), home.path().join("gh-loap-3"));
        }

        let mut rowless = stale.clone();
        super::carry_block_slug_identity_env(&store, "block-p03-none", &mut rowless);
        assert_eq!(rowless, stale, "no row, no slug: cmd:env stays");
    }

    #[test]
    fn maps_agent_id_to_name_and_placeholder_email() {
        let vars = git_identity_env_vars("korp");
        let map: HashMap<&str, String> = vars.into_iter().collect();
        assert_eq!(map["GIT_AUTHOR_NAME"], "korp");
        assert_eq!(map["GIT_COMMITTER_NAME"], "korp");
        assert_eq!(map["GIT_AUTHOR_EMAIL"], "korp@agentmux.local");
        assert_eq!(map["GIT_COMMITTER_EMAIL"], "korp@agentmux.local");
    }

    #[test]
    fn lowercases_email_but_preserves_display_name_casing() {
        // AGENTMUX_AGENT_ID is natural display casing (e.g. "Korp", per
        // SPEC_PR_TITLE_AGENT_HOST_PREFIX_2026_08_22.md's distinction
        // between the tag's lowercase machine-key and the title's natural
        // casing) -- the git *name* field should read naturally too, but
        // the email's local-part must stay lowercase so it can never
        // collide with a differently-cased but same agent (git/GitHub
        // treat email local-parts as effectively case-sensitive strings
        // for linking purposes; we want exactly one canonical email per
        // agent regardless of what casing happened to be in the block's
        // agentName metadata at spawn time).
        let vars = git_identity_env_vars("Korp");
        let map: HashMap<&str, String> = vars.into_iter().collect();
        assert_eq!(map["GIT_AUTHOR_NAME"], "Korp");
        assert_eq!(map["GIT_AUTHOR_EMAIL"], "korp@agentmux.local");
    }

    #[test]
    fn distinct_agents_get_distinct_non_colliding_identities() {
        let a = git_identity_env_vars("agenty");
        let b = git_identity_env_vars("smike");
        let a_map: HashMap<&str, String> = a.into_iter().collect();
        let b_map: HashMap<&str, String> = b.into_iter().collect();
        assert_ne!(a_map["GIT_AUTHOR_EMAIL"], b_map["GIT_AUTHOR_EMAIL"]);
        assert_ne!(a_map["GIT_AUTHOR_NAME"], b_map["GIT_AUTHOR_NAME"]);
    }

    #[test]
    fn email_domain_is_not_a_real_github_verifiable_domain() {
        // Load-bearing for the fix's safety property: this must NOT be a
        // domain GitHub could ever link to a real account (see the
        // git_identity_env_vars doc comment) -- asserting the exact
        // domain here so a future edit can't accidentally change it to
        // something real (e.g. "users.noreply.github.com") without this
        // test catching it.
        let vars = git_identity_env_vars("camper");
        let map: HashMap<&str, String> = vars.into_iter().collect();
        assert!(map["GIT_AUTHOR_EMAIL"].ends_with("@agentmux.local"));
        assert!(map["GIT_COMMITTER_EMAIL"].ends_with("@agentmux.local"));
    }
}
