// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The connection RPCs the frontend's connection UI already calls
//! (`conntypeahead.tsx`, `blockframe.tsx`'s `ConnStatusOverlay`), which had no
//! handler until now (P0 of SPEC_REMOTE_TERMINALS_AND_DURABLE_SESSIONS_2026_10_02.md).
//!
//! P0 is honest rather than capable: `local` works as it always has, and a WSL
//! or SSH connection is refused with a status the pane shows, instead of the
//! pane silently starting a local shell under a remote name. P1 (WSL) and P2
//! (SSH) replace the refusals.

use super::*;
use crate::backend::mps::Broker;
use crate::backend::remote::status::{self, state};
use crate::backend::remote::ConnTarget;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // Lists. Empty until the phases that fill them: the typeahead already adds
    // `local` itself. One call each, with the constant spelled out: the RPC
    // contract test resolves every registered command name statically.
    engine.register_handler(
        COMMAND_CONN_LIST,
        Box::new(|_data, _ctx| Box::pin(async move { Ok(Some(serde_json::json!([]))) })),
    );
    engine.register_handler(
        COMMAND_WSL_LIST,
        Box::new(|_data, _ctx| Box::pin(async move { Ok(Some(serde_json::json!([]))) })),
    );
    engine.register_handler(
        COMMAND_CONN_LIST_AWS,
        Box::new(|_data, _ctx| Box::pin(async move { Ok(Some(serde_json::json!([]))) })),
    );

    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_CONN_ENSURE,
        Box::new(move |data, _ctx| {
            let broker = broker.clone();
            Box::pin(async move {
                // `{ connname, logblockid? }`
                let name = data
                    .get("connname")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                ensure(&broker, &name).map(|_| None)
            })
        }),
    );

    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_CONN_CONNECT,
        Box::new(move |data, _ctx| {
            let broker = broker.clone();
            Box::pin(async move {
                // `{ host, keywords?, logblockid? }`
                let name = data
                    .get("host")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                ensure(&broker, &name).map(|_| None)
            })
        }),
    );

    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_CONN_DISCONNECT,
        Box::new(move |data, _ctx| {
            let broker = broker.clone();
            Box::pin(async move {
                // The connection name, as a bare string.
                let name = data.as_str().unwrap_or_default().to_string();
                if let Ok(target) = ConnTarget::parse(&name) {
                    if !target.is_local() {
                        status::set(Some(&broker), &name, state::DISCONNECTED, None);
                    }
                }
                Ok(None)
            })
        }),
    );
}

/// What `connensure` and `connconnect` do in P0: accept `local`, refuse a bad
/// name with the reason, and refuse a WSL or SSH connection as not available
/// yet, recording the error status the pane's overlay shows.
pub(crate) fn ensure(broker: &Broker, name: &str) -> Result<(), String> {
    let target = ConnTarget::parse(name)?;
    let message = match &target {
        ConnTarget::Local => return Ok(()),
        ConnTarget::Wsl(_) => "WSL terminals are not available in this version of AgentMux yet",
        ConnTarget::Ssh(_) => "SSH terminals are not available in this version of AgentMux yet",
    };
    // Keyed by the name exactly as the pane's meta holds it, not the canonical
    // form: the pane's overlay looks its status up by `meta.connection`, so
    // " area54 " or "host:022" would otherwise never see their error (Codex P2
    // on #4248).
    status::set(Some(broker), name, state::ERROR, Some(message));
    Err(message.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_is_accepted_and_remote_is_refused_with_a_status() {
        let broker = Broker::new();
        assert_eq!(ensure(&broker, "local"), Ok(()));
        assert_eq!(ensure(&broker, ""), Ok(()));

        let err = ensure(&broker, "test-ensure-host").unwrap_err();
        assert!(err.contains("SSH terminals are not available"), "{err}");
        let st = status::all()
            .into_iter()
            .find(|s| s.connection == "test-ensure-host")
            .unwrap();
        assert_eq!(
            (st.status.as_str(), st.error.as_str()),
            (state::ERROR, err.as_str())
        );

        assert!(ensure(&broker, "wsl://Ubuntu").unwrap_err().contains("WSL"));
        // A name that is not a connection says why, and records nothing.
        assert!(ensure(&broker, "-oProxyCommand=calc")
            .unwrap_err()
            .contains("cannot start with '-'"));
        assert!(!status::all().iter().any(|s| s.connection.starts_with('-')));
    }

    #[test]
    fn the_status_is_keyed_by_the_name_the_pane_holds() {
        let broker = Broker::new();
        for raw in ["test-raw-host:022", " test-raw-spaced "] {
            let _ = ensure(&broker, raw);
            assert!(status::all().iter().any(|s| s.connection == raw), "{raw:?}");
        }
    }
}
