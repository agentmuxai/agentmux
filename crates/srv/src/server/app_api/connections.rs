// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The connection RPCs the frontend's connection UI already calls
//! (`conntypeahead.tsx`, `blockframe.tsx`'s `ConnStatusOverlay`), which had no
//! handler until now (P0 of SPEC_REMOTE_TERMINALS_AND_DURABLE_SESSIONS_2026_10_02.md).
//!
//! A connection AgentMux cannot run is refused with a status the pane shows,
//! instead of the pane silently starting a local shell under a remote name:
//! WSL off Windows or for a distro that is not installed (P1), and SSH where
//! there is no `ssh` (P2).

use super::*;
use crate::backend::mps::Broker;
use crate::backend::remote::status::{self, state};
use crate::backend::remote::ConnTarget;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // Lists. One call each, with the constant spelled out: the RPC contract
    // test resolves every registered command name statically.
    //
    // The picker's "Remote" section: the user's ssh config hosts and any other
    // SSH connection used this session. The typeahead adds `local` and the
    // WSL distros itself.
    engine.register_handler(
        COMMAND_CONN_LIST,
        Box::new(|_data, _ctx| {
            Box::pin(async move {
                let hosts = tokio::task::spawn_blocking(crate::backend::remote::ssh_config::hosts)
                    .await
                    .unwrap_or_default();
                Ok(Some(serde_json::json!(remote_names(
                    &hosts,
                    &status::all()
                ))))
            })
        }),
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
/// that is installed (Windows only); accept an SSH destination where the
/// system `ssh` is (its own prompts then happen in the pane's terminal); refuse
/// a bad name with the reason. Every WSL or SSH outcome is recorded as the
/// status the pane's overlay shows.
pub(crate) async fn ensure(broker: &Broker, name: &str) -> Result<(), String> {
    let (installed, ssh_found) = match ConnTarget::parse(name)? {
        ConnTarget::Wsl(_) => (crate::backend::remote::wsl::list().await, false),
        ConnTarget::Ssh(_) => (Vec::new(), crate::backend::remote::ssh::binary().is_some()),
        ConnTarget::Local => (Vec::new(), false),
    };
    ensure_with(broker, name, &installed, cfg!(windows), ssh_found)
}

/// [`ensure`] with the installed distros, the platform and whether `ssh` was
/// found given, so it is testable anywhere.
pub(crate) fn ensure_with(
    broker: &Broker,
    name: &str,
    wsl_installed: &[String],
    on_windows: bool,
    ssh_found: bool,
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
        ConnTarget::Ssh(_) if ssh_found => {
            status::set(Some(broker), name, state::CONNECTED, None);
            return Ok(());
        }
        ConnTarget::Ssh(_) => crate::backend::remote::ssh::missing_binary_message(),
    };
    // Keyed by the name exactly as the pane's meta holds it, not the canonical
    // form: the pane's overlay looks its status up by `meta.connection`, so
    // " area54 " or "host:022" would otherwise never see their error (#4248).
    status::set(Some(broker), name, state::ERROR, Some(&message));
    Err(message)
}

/// Where an agent's `Shell` or `PtyShell` runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AgentTarget {
    Local,
    /// An installed WSL distro.
    Wsl(String),
    /// An SSH destination: runs only with the user's consent
    /// ([`consent_for_ssh`]), and every ssh prompt goes to the user
    /// (`remote::askpass`), never to the agent.
    Ssh(crate::backend::remote::conn::SshDest),
}

/// The connection an agent's `Shell` or `PtyShell` call asked for, checked the
/// same way a pane's is ([`ensure`]), `Err` with the reason when it cannot run.
pub(crate) async fn for_agent(broker: &Broker, name: Option<&str>) -> Result<AgentTarget, String> {
    let name = name.unwrap_or_default();
    ensure(broker, name).await?;
    Ok(match ConnTarget::parse(name)? {
        ConnTarget::Local => AgentTarget::Local,
        ConnTarget::Wsl(distro) => AgentTarget::Wsl(distro),
        ConnTarget::Ssh(dest) => AgentTarget::Ssh(dest),
    })
}

/// The agent id of the agent in `agent_block_id`, as AgentMux launched it (its
/// block's own `cmd:env`, never anything a request carries). `None` for an
/// agent without one: it is never given a shared stand-in name, which would
/// let one answer about it stand for every other agent without an id.
pub(crate) fn agent_of(state: &AppState, agent_block_id: &str) -> Option<String> {
    state
        .mstore
        .get::<crate::backend::obj::Block>(agent_block_id)
        .ok()
        .flatten()
        .and_then(|b| {
            b.meta
                .get("cmd:env")
                .and_then(|e| e.get("AGENTMUX_AGENT_ID"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .filter(|a| !a.trim().is_empty())
}

/// How a dialog names an agent: its id, as one plain capped line.
pub(crate) fn agent_label(agent: Option<&str>) -> String {
    agent
        .map(|a| one_line(a, 60))
        .unwrap_or_else(|| "An agent with no AgentMux id".to_string())
}

/// One line of agent-supplied text for a dialog: shown as plain text, newlines
/// folded and length capped, so it cannot lay out a message of its own.
pub(crate) fn one_line(text: &str, max: usize) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        flat
    } else {
        format!("{}…", flat.chars().take(max).collect::<String>())
    }
}

/// How long the user has to answer a consent or askpass dialog.
const DIALOG_TIMEOUT_MS: u64 = 120_000;

/// The user's answer to [`ask_user`]. `answered` is false when the window was
/// closed, timed out, or could not be opened.
#[derive(Debug, Default, Clone, serde::Deserialize)]
pub(crate) struct UserAnswer {
    #[serde(default)]
    pub answered: bool,
    #[serde(default)]
    pub approve: bool,
    /// For a `secret` question: never logged.
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub checkbox: bool,
}

/// Ask the user a question about the agent in `agent_block_id`, in an approval
/// subwindow the host opens over the window showing it (crates/cef
/// `ssh_approval`), and wait for the answer.
///
/// Through the host's own IPC server and its token, which no agent holds, and
/// answered in a window the browser API never resolves a pane into: an agent
/// can neither click this answer nor forge it, unlike a modal in the main
/// window answered through srv's own services (both reachable with the auth
/// key every agent has). With no host connected (headless) there is no one to
/// ask: `Err`, so whatever needed the answer does not happen.
pub(crate) async fn ask_user(
    state: &AppState,
    agent_block_id: &str,
    question: serde_json::Value,
) -> Result<UserAnswer, String> {
    let host = state
        .host_ipc
        .lock()
        .await
        .clone()
        .ok_or_else(|| "no AgentMux window is connected to ask the user in".to_string())?;
    let mut body = question;
    body["block_id"] = serde_json::json!(agent_block_id);
    body["timeout_ms"] = serde_json::json!(DIALOG_TIMEOUT_MS);
    let url = format!("http://127.0.0.1:{}/agentmux/approval/ask", host.port);
    let resp = state
        .http_client
        .post(&url)
        .header("Authorization", format!("Bearer {}", host.token))
        .timeout(std::time::Duration::from_millis(DIALOG_TIMEOUT_MS + 15_000))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("could not ask the user: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("could not ask the user: the window host answered HTTP {}", resp.status()));
    }
    let v: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("could not ask the user: {e}"))?;
    if v.get("ok").and_then(|o| o.as_bool()) != Some(true) {
        let e = v.get("error").and_then(|e| e.as_str()).unwrap_or("no answer");
        return Err(format!("could not ask the user: {e}"));
    }
    serde_json::from_value(v.get("data").cloned().unwrap_or_default())
        .map_err(|e| format!("could not ask the user: {e}"))
}

/// The user's consent for the agent in `agent_block_id` to run `what` on the
/// SSH `connection` with the user's identity (spec §8.2). `connection` is the
/// canonical name (`ConnTarget::name`), so one host is one consent however it
/// is spelled. "Always" for that agent and host is remembered
/// (`remote::agent_access`) and asks nothing again; otherwise the user is
/// asked ([`ask_user`]). An agent without an id is asked every time: "always"
/// is neither offered nor stored for it.
pub(crate) async fn consent_for_ssh(
    state: &AppState,
    agent_block_id: &str,
    connection: &str,
    what: &str,
) -> Result<(), String> {
    use crate::backend::remote::agent_access;
    let agent_id = agent_of(state, agent_block_id);
    let agent = agent_label(agent_id.as_deref());
    let connection = connection.trim();
    if agent_id
        .as_deref()
        .is_some_and(|id| agent_access::always_allowed(id, connection))
    {
        tracing::info!(agent = %agent, connection = %connection, what = %one_line(what, 200), "agent ssh access: allowed (always)");
        return Ok(());
    }
    let question = serde_json::json!({
        "kind": "consent",
        "title": format!("Agent access to {}", one_line(connection, 80)),
        "message": format!(
            "{agent} wants to run this on {}, as you, with your SSH keys:\n\n{}\n\nAllow it?",
            one_line(connection, 80),
            one_line(what, 400)
        ),
        "checkbox": match &agent_id {
            Some(_) => format!("Always allow {agent} on {}", one_line(connection, 80)),
            None => String::new(),
        },
        "ok_label": "Allow",
        "cancel_label": "Deny",
    });
    let answer = ask_user(state, agent_block_id, question)
        .await
        .map_err(|e| format!("{agent} needs the user's permission to use {connection}, and {e}"))?;
    if !(answer.answered && answer.approve) {
        tracing::info!(agent = %agent, connection = %connection, answered = answer.answered, "agent ssh access: not allowed");
        return Err(if answer.answered {
            format!("the user denied {agent} access to {connection}")
        } else {
            format!("the user did not answer whether {agent} may use {connection}")
        });
    }
    let always = answer.checkbox && agent_id.is_some();
    if let Some(id) = agent_id.as_deref().filter(|_| always) {
        if let Err(e) = agent_access::remember(id, connection) {
            tracing::warn!(error = %e, "agent ssh access: could not remember 'always'");
        }
    }
    tracing::info!(agent = %agent, connection = %connection, what = %one_line(what, 200), always, "agent ssh access: allowed");
    Ok(())
}

/// `POST /api/v1/askpass` body, from `agentmux-bashwrap` as ssh's askpass.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct AskpassRequest {
    secret: String,
    prompt: String,
    /// ssh's `SSH_ASKPASS_PROMPT`: `confirm`, `none`, or empty.
    #[serde(default)]
    hint: String,
}

/// `POST /api/v1/askpass`: show an ssh prompt to the user ([`ask_user`]) and
/// return the answer (`remote::askpass`). Only a live secret is answered, and
/// the secret, not anything in the request, says which agent and host the
/// question names.
pub(crate) async fn handle_askpass(
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::Json(req): axum::Json<AskpassRequest>,
) -> axum::response::Response {
    use crate::backend::remote::askpass::{self, PromptKind};
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    let refuse = |code: StatusCode, error: String| {
        (code, axum::Json(serde_json::json!({ "error": error }))).into_response()
    };
    let Some(grant) = askpass::lookup(&req.secret) else {
        return refuse(StatusCode::FORBIDDEN, "no such ssh prompt".to_string());
    };
    let kind = askpass::classify(&req.prompt, &req.hint);
    // The grant's names are AgentMux's own, shown as plain capped lines all
    // the same; ssh's prompt is ssh's text.
    let question = serde_json::json!({
        "kind": match kind {
            PromptKind::YesNo => "yesno",
            PromptKind::Info => "info",
            PromptKind::Secret => "secret",
        },
        "title": format!("SSH: {}", one_line(&grant.connection, 80)),
        "message": format!(
            "ssh, connecting to {} for {}, {}:\n\n{}",
            one_line(&grant.connection, 80),
            agent_label(Some(grant.agent.as_str()).filter(|a| !a.is_empty())),
            if kind == PromptKind::Info { "says" } else { "asks" },
            req.prompt.trim().chars().take(2000).collect::<String>()
        ),
        "ok_label": match kind {
            PromptKind::YesNo => "Yes",
            PromptKind::Info => "OK",
            PromptKind::Secret => "OK",
        },
        "cancel_label": if kind == PromptKind::YesNo { "No" } else { "Cancel" },
    });
    // A notice ("touch your security key"): ssh does not wait for an answer
    // and ends askpass itself once done, so it is shown and not waited on.
    if kind == PromptKind::Info {
        let state = state.clone();
        let block = grant.agent_block_id.clone();
        tokio::spawn(async move {
            let _ = ask_user(&state, &block, question).await;
        });
        return axum::Json(serde_json::json!({ "answer": "" })).into_response();
    }
    match ask_user(&state, &grant.agent_block_id, question).await {
        Ok(answer) if answer.answered => {
            // Never logged: the answer may be a password.
            let text = match kind {
                PromptKind::YesNo if answer.approve => "yes".to_string(),
                PromptKind::YesNo => "no".to_string(),
                PromptKind::Secret if answer.approve => answer.text,
                PromptKind::Secret => {
                    return refuse(StatusCode::GONE, "cancelled".to_string());
                }
                PromptKind::Info => String::new(),
            };
            axum::Json(serde_json::json!({ "answer": text })).into_response()
        }
        Ok(_) => refuse(StatusCode::GONE, "not answered".to_string()),
        Err(e) => refuse(StatusCode::CONFLICT, e),
    }
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
    /// Whether an agent can pass it as `connection`: every connection AgentMux
    /// can run. An SSH host also asks the user for consent the first time
    /// ([`consent_for_ssh`]).
    pub agent_can_use: bool,
}

/// The picker's remote names: the ssh config hosts, then any other SSH
/// connection known this session, each once.
pub(crate) fn remote_names(ssh_hosts: &[String], known: &[ConnStatus]) -> Vec<String> {
    use crate::backend::remote::conn::same_connection;
    let mut out: Vec<String> = ssh_hosts.to_vec();
    for s in known {
        let is_ssh = matches!(ConnTarget::parse(&s.connection), Ok(ConnTarget::Ssh(_)));
        if is_ssh && !out.iter().any(|n| same_connection(n, &s.connection)) {
            out.push(s.connection.clone());
        }
    }
    out
}

/// The connections there are: this machine, each installed WSL distro, the
/// user's ssh config hosts, and any other connection a pane has used this
/// session, with its status and whether an agent can use it.
pub(crate) fn list(
    wsl_installed: &[String],
    ssh_hosts: &[String],
    known: &[ConnStatus],
) -> Vec<ConnEntry> {
    use crate::backend::remote::conn::same_connection;
    let status_of = |name: &str| known.iter().find(|s| same_connection(&s.connection, name));
    let mut out = vec![ConnEntry {
        connection: "local".to_string(),
        kind: "local",
        status: state::CONNECTED.to_string(),
        error: String::new(),
        agent_can_use: true,
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
            agent_can_use: true,
        });
    }
    for host in ssh_hosts {
        let (status, error) = match status_of(host) {
            Some(s) => (s.status.clone(), s.error.clone()),
            None => ("available".to_string(), String::new()),
        };
        out.push(ConnEntry {
            connection: host.clone(),
            kind: "ssh",
            status,
            error,
            agent_can_use: true,
        });
    }
    for s in known {
        if out
            .iter()
            .any(|e| same_connection(&e.connection, &s.connection))
        {
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
            agent_can_use: true,
        });
    }
    out
}

/// `GET /api/v1/conn/list`: the agent `ConnList` tool (spec §8.1).
pub(crate) async fn handle_conn_list() -> axum::Json<Vec<ConnEntry>> {
    let installed = crate::backend::remote::wsl::list().await;
    let hosts = tokio::task::spawn_blocking(crate::backend::remote::ssh_config::hosts)
        .await
        .unwrap_or_default();
    axum::Json(list(&installed, &hosts, &status::all()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_is_accepted_and_remote_is_refused_with_a_status() {
        let broker = Broker::new();
        let none: &[String] = &[];
        let ensure = |name: &str| ensure_with(&broker, name, none, true, false);
        assert_eq!(ensure("local"), Ok(()));
        assert_eq!(ensure(""), Ok(()));

        let err = ensure("test-ensure-host").unwrap_err();
        assert!(err.contains("need the ssh command"), "{err}");
        // With ssh: connected, so the overlay leaves the terminal to ssh's own prompts.
        assert_eq!(
            ensure_with(&broker, "test-ensure-ssh-ok", none, true, true),
            Ok(())
        );
        assert!(status::all()
            .iter()
            .any(|s| s.connection == "test-ensure-ssh-ok" && s.connected));
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
            ensure_with(&broker, "wsl://ubuntu", &installed, true, false),
            Ok(())
        );
        let st = status::all()
            .into_iter()
            .find(|s| s.connection == "wsl://ubuntu")
            .unwrap();
        assert!(st.connected);

        let err = ensure_with(&broker, "wsl://Arch", &installed, true, false).unwrap_err();
        assert!(err.contains("'Arch' is not installed"), "{err}");

        let err = ensure_with(&broker, "wsl://Ubuntu", &installed, false, false).unwrap_err();
        assert!(err.contains("only available on Windows"), "{err}");
    }

    #[test]
    fn the_status_is_keyed_by_the_name_the_pane_holds() {
        let broker = Broker::new();
        let none: &[String] = &[];
        for raw in ["test-raw-host:022", " test-raw-spaced "] {
            let _ = ensure_with(&broker, raw, none, true, false);
            assert!(status::all().iter().any(|s| s.connection == raw), "{raw:?}");
        }
    }

    #[tokio::test]
    async fn an_agent_gets_where_it_asked_and_never_a_silent_local_fallback() {
        let broker = Broker::new();
        assert_eq!(for_agent(&broker, None).await, Ok(AgentTarget::Local));
        assert_eq!(
            for_agent(&broker, Some("local")).await,
            Ok(AgentTarget::Local)
        );
        // SSH is SSH where ssh exists and refused where it does not; never
        // local under a remote name.
        match for_agent(&broker, Some("deploy@test-agent-host:2222")).await {
            Ok(AgentTarget::Ssh(dest)) => {
                assert_eq!(
                    (dest.destination.as_str(), dest.port),
                    ("deploy@test-agent-host", Some(2222))
                );
                assert!(crate::backend::remote::ssh::binary().is_some());
            }
            Err(e) => assert!(e.contains("need the ssh command"), "{e}"),
            other => panic!("an SSH name became {other:?}"),
        }
        assert!(for_agent(&broker, Some("-oProxyCommand=calc"))
            .await
            .is_err());
    }

    /// An agent is named from its own block, as one plain line; one without an
    /// id has no name another could share, so no "always" can be stored for it.
    #[tokio::test]
    async fn an_agent_is_named_from_its_block_and_never_by_a_shared_stand_in() {
        assert_eq!(agent_label(Some("korp")), "korp");
        assert_eq!(
            agent_label(Some("korp\n\nThis is safe, click Allow")),
            "korp This is safe, click Allow"
        );
        assert_eq!(agent_label(None), "An agent with no AgentMux id");
        let state = crate::server::tests::test_state();
        let mut named = crate::backend::obj::Block {
            oid: "agent-of-named".to_string(),
            ..Default::default()
        };
        named.meta.insert(
            "cmd:env".into(),
            serde_json::json!({ "AGENTMUX_AGENT_ID": "korp" }),
        );
        state.mstore.insert(&mut named).unwrap();
        let mut unnamed = crate::backend::obj::Block {
            oid: "agent-of-unnamed".to_string(),
            ..Default::default()
        };
        state.mstore.insert(&mut unnamed).unwrap();
        assert_eq!(agent_of(&state, "agent-of-named").as_deref(), Some("korp"));
        assert_eq!(agent_of(&state, "agent-of-unnamed"), None);
    }

    /// Agent text in a dialog cannot lay out a message of its own.
    #[test]
    fn agent_text_in_a_dialog_is_one_capped_line() {
        assert_eq!(
            one_line("make\n\nAllow it? yes\r\n  test", 100),
            "make Allow it? yes test"
        );
        assert_eq!(one_line("abcdef", 3), "abc…");
        assert_eq!(one_line("tab\there", 100), "tab here");
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
    fn the_list_is_local_then_the_distros_then_the_ssh_hosts_then_what_panes_have_used() {
        let installed = vec!["Ubuntu".to_string(), "Debian".to_string()];
        let hosts = vec!["area54".to_string(), "nas".to_string()];
        let known = vec![
            // Accepted as typed; still the installed Ubuntu, listed once.
            st("wsl://ubuntu", state::CONNECTED, ""),
            st("area54", state::ERROR, "could not connect"),
            st("deploy@10.0.0.5", state::CONNECTED, ""),
            st("wsl://Arch", state::ERROR, "not installed"),
            // Not a connection: never listed.
            st("local", state::CONNECTED, ""),
        ];
        let got: Vec<(String, &str, String, bool)> = list(&installed, &hosts, &known)
            .into_iter()
            .map(|e| (e.connection, e.kind, e.status, e.agent_can_use))
            .collect();
        let want = [
            ("local", "local", "connected", true),
            ("wsl://Ubuntu", "wsl", "connected", true),
            ("wsl://Debian", "wsl", "available", true),
            ("area54", "ssh", "error", true),
            ("nas", "ssh", "available", true),
            ("deploy@10.0.0.5", "ssh", "connected", true),
            ("wsl://Arch", "wsl", "error", true),
        ];
        let want: Vec<(String, &str, String, bool)> = want
            .iter()
            .map(|(c, k, s, a)| (c.to_string(), *k, s.to_string(), *a))
            .collect();
        assert_eq!(got, want);
    }

    /// The picker's "Remote" section: config hosts first, then other SSH
    /// connections used this session, never WSL or local.
    #[test]
    fn the_picker_gets_the_ssh_hosts_and_the_ssh_connections_used() {
        let hosts = vec!["area54".to_string(), "nas".to_string()];
        let known = vec![
            st("area54", state::CONNECTED, ""),
            st("deploy@10.0.0.5:2222", state::DISCONNECTED, ""),
            st("wsl://Ubuntu", state::CONNECTED, ""),
            st("local", state::CONNECTED, ""),
        ];
        assert_eq!(
            remote_names(&hosts, &known),
            ["area54", "nas", "deploy@10.0.0.5:2222"]
        );
    }
}
