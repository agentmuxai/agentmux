// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `AGENTMUX_AGENT_FORMER_NAMES` and `AGENTMUX_AGENT_FORMER_SLUGS`: the names
//! an agent had before its renames, for `gh-agent` (0.10+). It signs a PR as
//! the agent that opened it but leaves a title, an `Agent:` line or a PR-body
//! tag that already names that agent as it is, and it learns the agent's own
//! names from the environment. Without these two, a renamed agent's old name
//! (still on a PR it had opened, or in a file it reads) read as another
//! agent's and was rewritten.
//!
//! Server-set, like `AGENTMUX_AGENT_UID` (`input::carry_agent_uid_env` calls
//! [`carry`]): the server's rename history replaces a `cmd:env` copy (a
//! launch-time snapshot, or the list a spawned worker copied from its leader),
//! and an agent with no history, or no row, has the variables removed. Names
//! are addresses only, never credentials.
//!
//! Comma-separated, so a value never holds a comma: a display name is written
//! in route form (`Loap #3` is `Loap-3`, `agent_route_name`), a slug as it was
//! stored. Blank names and names with an `@` (an address, not a name) are left
//! out, a display name with no ASCII letter or digit too (its route form is the
//! placeholder `agent`, which is nobody's name), duplicates are folded without
//! regard to case, and the list is sorted.

use std::collections::HashMap;

use crate::backend::storage::store::Store;

/// What the server holds of an agent's rename history.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct FormerNames {
    /// Display names it had (`db_agent_former_names`), as stored.
    names: Vec<String>,
    /// Slugs it answered to (`db_agent_former_slugs`).
    slugs: Vec<String>,
}

/// The history of the agent `uid` (the row `block_id` runs). A store error is
/// logged and reads as no history, like the token mint beside it: the agent
/// still has an identity, and a spawn never fails for this.
pub(super) fn read(mstore: &Store, block_id: &str, uid: &str) -> FormerNames {
    let names = mstore.agent_former_names(uid).unwrap_or_else(|e| {
        tracing::warn!(block_id, uid, error = %e, "spawn env: former-name lookup failed — no AGENTMUX_AGENT_FORMER_NAMES");
        Vec::new()
    });
    let slugs = mstore.agent_former_slugs_local(uid).map(|fs| fs.into_iter().map(|f| f.slug).collect()).unwrap_or_else(|e| {
        tracing::warn!(block_id, uid, error = %e, "spawn env: former-slug lookup failed — no AGENTMUX_AGENT_FORMER_SLUGS");
        Vec::new()
    });
    FormerNames { names, slugs }
}

/// Set both variables from `former`, or remove them: with no history, and
/// with no row (`None`).
pub(super) fn carry(env_vars: &mut HashMap<String, String>, former: Option<&FormerNames>) {
    let routes: Option<Vec<String>> = former.map(|f| f.names.iter().filter(|n| n.chars().any(|c| c.is_ascii_alphanumeric())).map(|n| agentmux_common::slug::agent_route_name(n)).collect());
    let slugs = former.map(|f| f.slugs.clone());
    for (key, names) in [("AGENTMUX_AGENT_FORMER_NAMES", routes), ("AGENTMUX_AGENT_FORMER_SLUGS", slugs)] {
        match names.and_then(|n| join(&n)) {
            Some(value) => env_vars.insert(key.to_string(), value),
            None => env_vars.remove(key),
        };
    }
}

/// `names` as one comma-separated value: trimmed, without blanks, addresses
/// (an `@`), commas or spaces, folded for duplicates, sorted. `None` if none
/// is left.
fn join(names: &[String]) -> Option<String> {
    let mut kept: Vec<&str> = Vec::new();
    for name in names.iter().map(|n| n.trim()) {
        if name.is_empty() || name.contains(['@', ',']) || name.contains(char::is_whitespace) {
            continue;
        }
        if !kept.iter().any(|k| k.to_lowercase() == name.to_lowercase()) {
            kept.push(name);
        }
    }
    kept.sort_by_key(|k| k.to_lowercase());
    (!kept.is_empty()).then(|| kept.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::agents::test_agent_def;
    use crate::server::agent_handlers::input::{carry_agent_uid_env, persisted_agent_identity};

    fn former(names: &[&str], slugs: &[&str]) -> FormerNames {
        let own = |v: &[&str]| v.iter().map(|s| s.to_string()).collect();
        FormerNames { names: own(names), slugs: own(slugs) }
    }

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn carried(f: &FormerNames) -> (Option<String>, Option<String>) {
        let mut e = HashMap::new();
        carry(&mut e, Some(f));
        (e.get("AGENTMUX_AGENT_FORMER_NAMES").cloned(), e.get("AGENTMUX_AGENT_FORMER_SLUGS").cloned())
    }

    fn block_running(store: &Store, block_id: &str, agent_id: &str) {
        let mut block = crate::backend::obj::Block {
            oid: block_id.to_string(),
            parentoref: String::new(),
            version: 0,
            runtimeopts: None,
            stickers: None,
            meta: crate::backend::obj::MetaMapType::from_iter([
                ("view".to_string(), serde_json::json!("agent")),
                ("agentId".to_string(), serde_json::json!(agent_id)),
            ]),
            subblockids: None,
        };
        store.insert(&mut block).unwrap();
    }

    /// A display name is written in route form, in order, once.
    #[test]
    fn display_names_are_route_form_unique_and_sorted() {
        let (names, _) = carried(&former(&["Zed", "Loap #3", "loap-3", " ", "agent2@narko"], &[]));
        assert_eq!(names.as_deref(), Some("Loap-3,Zed"));
        assert_eq!(carried(&former(&["Smith, Jr"], &[])).0.as_deref(), Some("Smith-Jr"), "no comma survives");
        assert_eq!(carried(&former(&["", "a@b"], &[])), (None, None));
    }

    /// A name with no ASCII letter or digit routes as the placeholder `agent`,
    /// which is nobody's name, so it is left out; and a slug is carried as it
    /// was stored, never route-formed (#5441).
    #[test]
    fn a_non_ascii_name_is_left_out_and_a_slug_is_carried_as_it_was() {
        assert_eq!(carried(&former(&["名前", "Real"], &[])).0.as_deref(), Some("Real"));
        assert_eq!(carried(&former(&["名前"], &[])).0, None, "not exported as `agent`");
        assert_eq!(carried(&former(&[], &["ä-bot", "名前", "plain", "Plain", "has space", "a,b", "x@y"])).1.as_deref(), Some("plain,ä-bot,名前"));
    }

    /// Reserved: the server's history replaces a `cmd:env` copy, and no
    /// history, or no row, removes it.
    #[test]
    fn the_variables_are_server_set_and_removed_when_unknown() {
        let stale = || {
            env(&[
                ("AGENTMUX_AGENT_FORMER_NAMES", "Camper"),
                ("AGENTMUX_AGENT_FORMER_SLUGS", "camper"),
                ("AGENTMUX_AGENT_ID", "qtimer"),
            ])
        };
        let mut e = stale();
        carry(&mut e, Some(&former(&["Qt Old"], &[])));
        assert_eq!(e["AGENTMUX_AGENT_FORMER_NAMES"], "Qt-Old");
        assert!(!e.contains_key("AGENTMUX_AGENT_FORMER_SLUGS"), "no former slugs: the stale one goes");

        for none in [None, Some(&FormerNames::default())] {
            let mut e = stale();
            carry(&mut e, none);
            assert!(!e.contains_key("AGENTMUX_AGENT_FORMER_NAMES"));
            assert!(!e.contains_key("AGENTMUX_AGENT_FORMER_SLUGS"));
            assert_eq!(e["AGENTMUX_AGENT_ID"], "qtimer", "other identity untouched");
        }
    }

    /// End to end through a real store and the call the spawn paths make: an
    /// agent never renamed carries nothing; after two display renames and a
    /// slug rename it carries the old display names (route form) and the slug
    /// it had.
    #[test]
    fn a_renamed_agents_history_reaches_the_env() {
        let store = Store::open_in_memory().unwrap();
        let mut def = test_agent_def("uid-loap", "Loap", "claude", "host", 1, "");
        def.slug = "loap".to_string();
        store.agent_def_insert(&mut def).unwrap();
        block_running(&store, "block-loap", "uid-loap");

        let spawn_env = |stale: &[(&str, &str)]| {
            let mut e = env(stale);
            carry_agent_uid_env(&mut e, persisted_agent_identity(&store, "block-loap").as_ref());
            e
        };
        let e = spawn_env(&[]);
        assert!(!e.contains_key("AGENTMUX_AGENT_FORMER_NAMES"), "never renamed");
        assert!(!e.contains_key("AGENTMUX_AGENT_FORMER_SLUGS"), "never renamed");

        for name in ["Loap #3", "Loap Prime"] {
            let mut d = store.agent_def_get("uid-loap").unwrap().unwrap();
            d.name = name.to_string();
            assert!(store.agent_def_update(&mut d).unwrap());
        }
        store.test_rename_slug("uid-loap", "loap-prime", 7);

        let e = spawn_env(&[("AGENTMUX_AGENT_FORMER_NAMES", "Camper")]);
        assert_eq!(e["AGENTMUX_AGENT_FORMER_NAMES"], "Loap,Loap-3", "its old display names, route form");
        assert_eq!(e["AGENTMUX_AGENT_FORMER_SLUGS"], "loap", "the slug it had");
    }
}
