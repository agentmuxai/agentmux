// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Who is calling, derived from the request's credential — identity M4a
//! (`docs/specs/SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md`
//! §6.5.3).
//!
//! A request that carries `X-Agent-Token` for a token this srv minted is
//! [`Caller::Agent`] with that token's UID. Anything else is
//! [`Caller::Unattributed`] — the UI, `bashwrap`, `muxsh`, a tokenless agent,
//! a peer srv. **This is attribution, never authorization:** at the same-user
//! boundary a request that omits its token cannot be told apart from the UI
//! (§6.5.1), so nothing here refuses a request. An unknown or foreign-channel
//! token is counted and treated as absent — an agent deleted while running,
//! or a token that reached another channel's srv through a forwarded
//! environment, keeps working exactly as before.
//!
//! Derived only for requests that authenticated with the full instance key:
//! a token on a LAN-key request must not lift it to host trust (§6.5.3).

use axum::body::Body;
use axum::extract::State;
use axum::http::Request;
use axum::middleware::Next;
use axum::response::Response;

use super::{AppState, ReactiveAuthVia};

/// The request header an agent's MCP sets from its inherited
/// `AGENTMUX_AGENT_TOKEN`.
pub(crate) const AGENT_TOKEN_HEADER: &str = "X-Agent-Token";

/// Who a request is attributed to. Inserted into every request's extensions
/// on the authed and LAN-forward route groups.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Caller {
    /// Carried a token this srv minted for `uid`.
    Agent { uid: String },
    /// Anything else. Not an error, and not a lesser principal: most callers
    /// (the UI among them) are this.
    Unattributed,
}

impl Caller {
    /// The caller's UID, if attributed.
    pub(crate) fn uid(&self) -> Option<&str> {
        match self {
            Caller::Agent { uid } => Some(uid),
            Caller::Unattributed => None,
        }
    }
}

/// Resolve the request's [`Caller`]. Pure, so the rules are testable without
/// a router: `token` is the header value, `full_key` whether the request
/// authenticated with the full instance key, `lookup` the token index.
/// What the token index said about a token.
pub(crate) enum Lookup {
    Index(Option<String>),
    /// No index attached (it failed to build at boot) — counted apart from an
    /// unknown token, so the two are not confused.
    NoIndex,
}

pub(crate) fn derive_caller(
    token: Option<&str>,
    full_key: bool,
    lookup: impl FnOnce(&str) -> Lookup,
) -> Caller {
    let Some(token) = token.map(str::trim).filter(|t| !t.is_empty()) else {
        return Caller::Unattributed;
    };
    if !full_key {
        crate::backend::agent_resolve::record_uid_fallback("m4.token_on_lan_key");
        return Caller::Unattributed;
    }
    match lookup(token) {
        Lookup::Index(Some(uid)) => Caller::Agent { uid },
        Lookup::Index(None) => {
            crate::backend::agent_resolve::record_uid_fallback("m4.token_unknown");
            Caller::Unattributed
        }
        Lookup::NoIndex => {
            crate::backend::agent_resolve::record_uid_fallback("m4.token_index_unavailable");
            Caller::Unattributed
        }
    }
}

/// Inserts [`Caller`]. Layered *inside* `auth_middleware` /
/// `lan_or_full_auth_middleware`; both mark which key authenticated the
/// request (`ReactiveAuthVia`), and a `Caller` is derived only when that mark
/// says the full key — a request that reached here unmarked is Unattributed,
/// never assumed full-key (review on #3571). The WebSocket upgrade is always
/// Unattributed (§6.5.3): WebSocket RPC is the UI's channel, and a token
/// must never ride on it.
pub(crate) async fn caller_middleware(
    State(state): State<AppState>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    let full_key = req.extensions().get::<ReactiveAuthVia>() == Some(&ReactiveAuthVia::FullAuthKey);
    // A CORS preflight skips auth, so it is never marked; it is not a
    // token on a LAN key and must not be counted as one.
    let token = if req.uri().path() == "/ws" || req.method() == axum::http::Method::OPTIONS {
        None
    } else {
        req.headers()
            .get(AGENT_TOKEN_HEADER)
            .and_then(|v| v.to_str().ok())
    };
    let caller = derive_caller(token, full_key, |t| match state.mstore.token_index() {
        Some(index) => Lookup::Index(index.uid_for(t)),
        None => Lookup::NoIndex,
    });
    req.extensions_mut().insert(caller);
    next.run(req).await
}

/// `GET /agentmux/identity/fallbacks` — every §9.2 counter and the two
/// live gauges (§6.5.6). The phase gates read this: M4d and M5 wait for
/// `live.tokenless_or_unknown` to reach zero and for the fallback counters
/// to stop moving. Read-only; auth-gated like every loopback route.
///
/// Also reports who the request itself was attributed to (`caller`), so an
/// agent can check that its token is being recognised.
pub(crate) async fn handle_identity_fallbacks(
    State(state): State<AppState>,
    caller: Option<axum::Extension<Caller>>,
) -> axum::Json<serde_json::Value> {
    let counters: serde_json::Map<String, serde_json::Value> =
        crate::backend::agent_resolve::uid_fallback_counts()
            .into_iter()
            .map(|(site, n)| (site.to_string(), serde_json::json!(n)))
            .collect();
    // `has_row` reads the store per UID-less registration, so off the async
    // worker (incident #1782).
    let registrations = state.reactive_handler.list_agents();
    let mstore = state.mstore.clone();
    // A failed read reports the gauges as null, never as a drained zero.
    let gauges = tokio::task::spawn_blocking(move || {
        crate::backend::identity_spawn::live_gauges(
            &registrations,
            |block| crate::backend::blockcontroller::get_controller(block).is_some(),
            |block| crate::backend::identity_spawn::store_block_has_row(&mstore, block),
        )
    })
    .await
    .ok();
    let (tokenless_or_unknown, unidentified) = match gauges {
        Some((t, u)) => (serde_json::json!(t), serde_json::json!(u)),
        None => (serde_json::Value::Null, serde_json::Value::Null),
    };
    let caller_uid = caller.as_ref().and_then(|c| c.0.uid().map(str::to_string));
    axum::Json(serde_json::json!({
        // Counters and gauges are in memory, per srv, since this boot: a
        // counter that has not fired is absent (read it as zero), and a gate
        // reader must sample every channel's srv (spec §9.2).
        "boot_id": state.boot_id.to_string(),
        "caller": caller_uid,
        "counters": counters,
        "gauges": {
            "live.tokenless_or_unknown": tokenless_or_unknown,
            "live.unidentified": unidentified,
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(token: &str) -> Lookup {
        Lookup::Index((token == "tok-y").then(|| "uid-y".to_string()))
    }

    #[test]
    fn a_known_token_on_the_full_key_is_the_agent() {
        assert_eq!(
            derive_caller(Some("tok-y"), true, index),
            Caller::Agent {
                uid: "uid-y".into()
            }
        );
    }

    #[test]
    fn no_token_is_unattributed() {
        assert_eq!(derive_caller(None, true, index), Caller::Unattributed);
        assert_eq!(derive_caller(Some("  "), true, index), Caller::Unattributed);
    }

    /// Never a 401: an unknown token (deleted agent, another channel's srv)
    /// is treated as absent.
    #[test]
    fn an_unknown_token_is_unattributed_not_refused() {
        assert_eq!(
            derive_caller(Some("tok-gone"), true, index),
            Caller::Unattributed
        );
    }

    #[test]
    fn a_missing_index_is_unattributed() {
        assert_eq!(
            derive_caller(Some("tok-y"), true, |_| Lookup::NoIndex),
            Caller::Unattributed
        );
    }

    /// A token must not lift a LAN-key request to host trust.
    #[test]
    fn a_token_on_a_lan_key_request_is_ignored() {
        assert_eq!(
            derive_caller(Some("tok-y"), false, index),
            Caller::Unattributed
        );
    }
}
