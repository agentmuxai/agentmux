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
    // Lists. SSH hosts arrive with P2; the typeahead adds `local` itself.
    // One call each, with the constant spelled out: the RPC contract test
    // resolves every registered command name statically.
    engine.register_handler(
        COMMAND_CONN_LIST,
        Box::new(|_data, _ctx| Box::pin(async move { Ok(Some(serde_json::json!([]))) })),
    );
    engine.register_handler(
        COMMAND_CONN_LIST_AWS,
        Box::new(|_data, _ctx| Box::pin(async move { Ok(Some(serde_json::json!([]))) })),
    );
    // Installed WSL distributions (empty off Windows, or with no WSL).
    engine.register_handler(
        COMMAND_WSL_LIST,
        Box::new(|_data, _ctx| {
            Box::pin(async move {
                let distros = crate::backend::remote::wsl::list().await;
                Ok(Some(serde_json::json!(distros)))
            })
        }),
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
                ensure(&broker, &name).await.map(|_| None)
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
                ensure(&broker, &name).await.map(|_| None)
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

/// What `connensure` and `connconnect` do: accept `local`; accept a WSL distro
/// that is installed (Windows only); refuse a bad name with the reason; refuse
/// SSH as not available yet (P2). Every WSL or SSH outcome is recorded as the
/// status the pane's overlay shows.
pub(crate) async fn ensure(broker: &Broker, name: &str) -> Result<(), String> {
    let installed = match ConnTarget::parse(name)? {
        ConnTarget::Wsl(_) => crate::backend::remote::wsl::list().await,
        _ => Vec::new(),
    };
    ensure_with(broker, name, &installed, cfg!(windows))
}

/// [`ensure`] with the installed distros and the platform given, so it is
/// testable anywhere.
pub(crate) fn ensure_with(
    broker: &Broker,
    name: &str,
    wsl_installed: &[String],
    on_windows: bool,
) -> Result<(), String> {
    let target = ConnTarget::parse(name)?;
    let message = match &target {
        ConnTarget::Local => return Ok(()),
        ConnTarget::Wsl(_) if !on_windows => "WSL is only available on Windows".to_string(),
        ConnTarget::Wsl(distro) if wsl_installed.iter().any(|d| d.eq_ignore_ascii_case(distro)) => {
            status::set(Some(broker), name, state::CONNECTED, None);
            return Ok(());
        }
        ConnTarget::Wsl(distro) => {
            format!("WSL distribution '{distro}' is not installed (wsl.exe --list shows what is)")
        }
        ConnTarget::Ssh(_) => {
            "SSH terminals are not available in this version of AgentMux yet".to_string()
        }
    };
    // Keyed by the name exactly as the pane's meta holds it, not the canonical
    // form: the pane's overlay looks its status up by `meta.connection`, so
    // " area54 " or "host:022" would otherwise never see their error (#4248).
    status::set(Some(broker), name, state::ERROR, Some(&message));
    Err(message)
}

/// The connection an agent's `Shell` or `PtyShell` call asked for, checked the
/// same way a pane's is ([`ensure`]): `Ok(None)` for this machine, `Ok(Some(distro))`
/// for an installed WSL distro, `Err` with the reason otherwise.
pub(crate) async fn for_agent(
    broker: &Broker,
    name: Option<&str>,
) -> Result<Option<String>, String> {
    let name = name.unwrap_or_default();
    ensure(broker, name).await?;
    Ok(match ConnTarget::parse(name)? {
        ConnTarget::Wsl(distro) => Some(distro),
        _ => None,
    })
}

/// One row of the agent `ConnList` tool.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub(crate) struct ConnEntry {
    pub connection: String,
    /// `local`, `wsl` or `ssh`.
    pub kind: &'static str,
    /// `connected`, `available` (usable, not opened yet), or a status from
    /// `remote::status` (`connecting`, `disconnected`, `error`).
    pub status: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
}

/// What an agent may connect to: this machine, each installed WSL distro, and
/// any other connection a pane has used this session, with its status.
pub(crate) fn list(wsl_installed: &[String], known: &[ConnStatus]) -> Vec<ConnEntry> {
    let status_of = |name: &str| known.iter().find(|s| s.connection == name);
    let mut out = vec![ConnEntry {
        connection: "local".to_string(),
        kind: "local",
        status: state::CONNECTED.to_string(),
        error: String::new(),
    }];
    for distro in wsl_installed {
        let name = ConnTarget::Wsl(distro.clone()).name();
        let (status, error) = match status_of(&name) {
            Some(s) => (s.status.clone(), s.error.clone()),
            None => ("available".to_string(), String::new()),
        };
        out.push(ConnEntry {
            connection: name,
            kind: "wsl",
            status,
            error,
        });
    }
    for s in known {
        if out.iter().any(|e| e.connection == s.connection) {
            continue;
        }
        let kind = match ConnTarget::parse(&s.connection) {
            Ok(ConnTarget::Wsl(_)) => "wsl",
            Ok(ConnTarget::Ssh(_)) => "ssh",
            _ => continue,
        };
        out.push(ConnEntry {
            connection: s.connection.clone(),
            kind,
            status: s.status.clone(),
            error: s.error.clone(),
        });
    }
    out
}

/// `GET /api/v1/conn/list`: the agent `ConnList` tool (spec §8.1).
pub(crate) async fn handle_conn_list() -> axum::Json<Vec<ConnEntry>> {
    let installed = crate::backend::remote::wsl::list().await;
    axum::Json(list(&installed, &status::all()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_is_accepted_and_remote_is_refused_with_a_status() {
        let broker = Broker::new();
        let none: &[String] = &[];
        let ensure = |name: &str| ensure_with(&broker, name, none, true);
        assert_eq!(ensure("local"), Ok(()));
        assert_eq!(ensure(""), Ok(()));

        let err = ensure("test-ensure-host").unwrap_err();
        assert!(err.contains("SSH terminals are not available"), "{err}");
        let st = status::all()
            .into_iter()
            .find(|s| s.connection == "test-ensure-host")
            .unwrap();
        assert_eq!(
            (st.status.as_str(), st.error.as_str()),
            (state::ERROR, err.as_str())
        );

        // A name that is not a connection says why, and records nothing.
        assert!(ensure("-oProxyCommand=calc")
            .unwrap_err()
            .contains("cannot start with '-'"));
        assert!(!status::all().iter().any(|s| s.connection.starts_with('-')));
    }

    #[test]
    fn an_installed_wsl_distro_connects_and_a_missing_one_says_so() {
        let broker = Broker::new();
        let installed = vec!["Ubuntu".to_string(), "Debian".to_string()];
        assert_eq!(
            ensure_with(&broker, "wsl://ubuntu", &installed, true),
            Ok(())
        );
        let st = status::all()
            .into_iter()
            .find(|s| s.connection == "wsl://ubuntu")
            .unwrap();
        assert!(st.connected);

        let err = ensure_with(&broker, "wsl://Arch", &installed, true).unwrap_err();
        assert!(err.contains("'Arch' is not installed"), "{err}");

        let err = ensure_with(&broker, "wsl://Ubuntu", &installed, false).unwrap_err();
        assert!(err.contains("only available on Windows"), "{err}");
    }

    #[test]
    fn the_status_is_keyed_by_the_name_the_pane_holds() {
        let broker = Broker::new();
        let none: &[String] = &[];
        for raw in ["test-raw-host:022", " test-raw-spaced "] {
            let _ = ensure_with(&broker, raw, none, true);
            assert!(status::all().iter().any(|s| s.connection == raw), "{raw:?}");
        }
    }

    #[tokio::test]
    async fn an_agent_gets_local_or_the_distro_and_never_a_silent_fallback() {
        let broker = Broker::new();
        assert_eq!(for_agent(&broker, None).await, Ok(None));
        assert_eq!(for_agent(&broker, Some("local")).await, Ok(None));
        // SSH is refused, not run locally under a remote name.
        let err = for_agent(&broker, Some("test-agent-host"))
            .await
            .unwrap_err();
        assert!(err.contains("SSH terminals are not available"), "{err}");
        assert!(for_agent(&broker, Some("-oProxyCommand=calc"))
            .await
            .is_err());
    }

    fn st(connection: &str, status: &str, error: &str) -> ConnStatus {
        ConnStatus {
            status: status.to_string(),
            connection: connection.to_string(),
            error: error.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn the_list_is_local_then_the_distros_then_what_panes_have_used() {
        let installed = vec!["Ubuntu".to_string(), "Debian".to_string()];
        let known = vec![
            st("wsl://Ubuntu", state::CONNECTED, ""),
            st("area54", state::ERROR, "SSH terminals are not available"),
            st("wsl://Arch", state::ERROR, "not installed"),
            // Not a connection: never listed.
            st("local", state::CONNECTED, ""),
        ];
        let got: Vec<(String, &str, String)> = list(&installed, &known)
            .into_iter()
            .map(|e| (e.connection, e.kind, e.status))
            .collect();
        let want = [
            ("local", "local", "connected"),
            ("wsl://Ubuntu", "wsl", "connected"),
            ("wsl://Debian", "wsl", "available"),
            ("area54", "ssh", "error"),
            ("wsl://Arch", "wsl", "error"),
        ];
        let want: Vec<(String, &str, String)> = want
            .iter()
            .map(|(c, k, s)| (c.to_string(), *k, s.to_string()))
            .collect();
        assert_eq!(got, want);
    }
}
