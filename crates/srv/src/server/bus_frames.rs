// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Who a WebSocket `bus:send`, `bus:inject` or `bus:broadcast` frame may name
//! as its sender. A frame carries no token, so the only identity a connection
//! has is the one it registered with (`bus:register`); the frame's `from` has
//! to be that agent. A frame that names no sender is sent as `unknown`, which
//! claims nothing. A registration is itself proven: it carries the agent's
//! token, and the agent it names must be the one that token was minted for.

use crate::backend::messagebus::Priority;
use crate::backend::storage::store::Store;

/// The priority a frame asks for; anything unrecognised is normal.
pub(super) fn priority(requested: Option<&str>) -> Priority {
    match requested {
        Some("high") => Priority::High,
        Some("urgent") => Priority::Urgent,
        _ => Priority::Normal,
    }
}

/// Whether a `bus:*` frame may be acted on: a `bus:register` is proven by its
/// token, a send, inject or broadcast by the registration; other frames
/// claim nothing.
pub(super) fn frame_allowed(
    store: &Store,
    msg_type: &str,
    registered: &str,
    (agent_id, token, from): (Option<&str>, Option<&str>, Option<&str>),
) -> Result<(), String> {
    match (msg_type, agent_id) {
        ("bus:register", Some(agent_id)) => register_allowed(store, token, agent_id),
        ("bus:send" | "bus:inject" | "bus:broadcast", _) => sender_allowed(store, registered, from),
        _ => Ok(()),
    }
}

/// Whether a `bus:register` frame carrying `token` may register `agent_id`: the
/// token has to be one this srv minted, for the agent that name selects.
pub(super) fn register_allowed(store: &Store, token: Option<&str>, agent_id: &str) -> Result<(), String> {
    let uid = token
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .and_then(|t| store.token_index()?.uid_for(t));
    let Some(uid) = uid else {
        return Err("FORBIDDEN: bus:register needs the agent's token (AGENTMUX_AGENT_TOKEN)".to_string());
    };
    if super::actor::names_the_caller(store, &uid, agent_id) {
        Ok(())
    } else {
        Err("FORBIDDEN: bus:register may register only the agent its token names".to_string())
    }
}

/// Whether a frame from a connection registered as `registered` (empty: it
/// never registered) may name `from`. Two names are the same agent when they
/// resolve to one id, as `check_s1` compares them.
pub(super) fn sender_allowed(store: &Store, registered: &str, from: Option<&str>) -> Result<(), String> {
    let Some(from) = from.map(str::trim).filter(|f| !f.is_empty()) else {
        return Ok(());
    };
    if registered.is_empty() {
        return Err("FORBIDDEN: register this connection (bus:register) as the sender before sending as it".to_string());
    }
    if registered == from {
        return Ok(());
    }
    let resolve = |name: &str| crate::backend::agent_resolve::resolve_agent_id(store, name);
    match (resolve(registered), resolve(from)) {
        (Ok(a), Ok(b)) if a == b => Ok(()),
        _ => Err("FORBIDDEN: the sender must be the agent this connection registered as".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::agents::test_agent_def;

    // `resolve_agent_id`'s registry tier reads the real `~/.agentmux` unless the home is overridden.
    use crate::test_support::ISOLATED_AUTH_ENV_LOCK as ENV_GUARD;

    fn with_store<T>(f: impl FnOnce(&Store) -> T) -> T {
        let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", home.path().to_str().unwrap());
        let out = f(&store());
        std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
        out
    }

    fn store() -> Store {
        let store = Store::open_in_memory().unwrap();
        for (id, name, slug) in [("uid-bf-a", "Alpha", "alpha"), ("uid-bf-b", "Beta", "beta")] {
            let mut def = test_agent_def(id, name, "claude", "agent", 1, "");
            def.slug = slug.to_string();
            store.agent_def_insert(&mut def).unwrap();
        }
        store
    }

    #[test]
    fn a_frame_may_name_only_the_agent_its_connection_registered_as() {
        with_store(|store| {
        assert!(sender_allowed(store, "alpha", Some("alpha")).is_ok());
        assert!(sender_allowed(store, "alpha", Some("uid-bf-a")).is_ok(), "the same agent by its id");
        let e = sender_allowed(store, "alpha", Some("beta")).unwrap_err();
        assert!(e.starts_with("FORBIDDEN"), "{e}");
        assert!(sender_allowed(store, "alpha", Some("nobody-at-all")).is_err(), "a name that is no agent");
        });
    }

    #[test]
    fn a_registration_is_proven_by_the_token_of_the_agent_it_names() {
        with_store(|store| {
            store.attach_token_index().unwrap();
            let token = store.agent_token_ensure("uid-bf-a").unwrap();
            assert!(register_allowed(store, Some(&token), "alpha").is_ok());
            assert!(register_allowed(store, Some(&token), "Alpha").is_ok(), "by display name");
            let other = register_allowed(store, Some(&token), "beta").unwrap_err();
            assert!(other.starts_with("FORBIDDEN"), "{other}");
            for missing in [None, Some(""), Some("  "), Some("not-a-token")] {
                let e = register_allowed(store, missing, "alpha").unwrap_err();
                assert!(e.contains("needs the agent's token"), "{missing:?}: {e}");
            }
        });
    }

    #[test]
    fn a_connection_that_never_registered_names_no_sender() {
        with_store(|store| {
        let e = sender_allowed(store, "", Some("alpha")).unwrap_err();
        assert!(e.contains("bus:register"), "{e}");
        assert!(sender_allowed(store, "", None).is_ok(), "no sender claims nothing");
        assert!(sender_allowed(store, "", Some("  ")).is_ok());
        assert!(sender_allowed(store, "alpha", None).is_ok());
        });
    }

    #[test]
    fn priority_defaults_to_normal() {
        assert!(matches!(priority(Some("urgent")), Priority::Urgent));
        assert!(matches!(priority(Some("high")), Priority::High));
        assert!(matches!(priority(Some("whatever")), Priority::Normal));
        assert!(matches!(priority(None), Priority::Normal));
    }
}
