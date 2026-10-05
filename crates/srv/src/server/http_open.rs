// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Pane and agent open.
//! Split out of server/mod.rs unchanged (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4.1).

use super::*;

/// `POST /api/v1/pane/open` — open a pane (editor/term/browser/…).
///
/// Called by `agentmux-mcp`'s `OpenEditor` tool. Thin HTTP wrapper over
/// `app_api::open_pane` (the same logic the WebSocket `pane.open` RPC uses):
/// creates the block, enqueues the layout action, and broadcasts the updates
/// so the frontend renders the pane. Body is `CommandPaneOpenData`.
pub(super) async fn handle_pane_open(
    State(state): State<AppState>,
    Json(req): Json<crate::backend::rpc_types::CommandPaneOpenData>,
) -> impl IntoResponse {
    // A pane on an SSH host reads and writes there as the user, with their
    // SSH keys: over HTTP (where agents call from) that takes the agent's
    // signed identity and the user's consent for that host, as `Shell` does
    // (remote terminals spec §8.2). Named as the field or inside `meta`.
    let mut opened_by_agent = None;
    if let Some(conn) = ssh_connection_of(&req) {
        let block = req.split_reference_block_id.clone().unwrap_or_default();
        let agent = match app_api::connections::verified_agent(&state, &block, req.auth.as_ref()) {
            Ok(a) => a,
            Err(e) => return (StatusCode::FORBIDDEN, Json(json!({ "error": e }))).into_response(),
        };
        let what = format!(
            "open {} in {}",
            req.file.as_deref().or(req.cwd.as_deref()).unwrap_or("a pane"),
            req.view
        );
        if let Err(e) = app_api::connections::consent_for_ssh(&state, &block, &agent, &conn, &what).await {
            return (StatusCode::FORBIDDEN, Json(json!({ "error": e }))).into_response();
        }
        opened_by_agent = Some(agent);
    }
    match app_api::open_pane(&state, req).await {
        Ok(result) => {
            // The pane is the agent's: installing the helper from it is
            // always asked about (SPEC_REMOTES_PANE_2026_10_05.md §4.9).
            if let Some(agent) = &opened_by_agent {
                crate::backend::remote::helper_consent::note_agent_pane(&result.block_id, agent);
            }
            (StatusCode::OK, Json(json!(result))).into_response()
        }
        Err(e) => {
            // Argument/validation errors from build_pane_meta are the caller's
            // fault (400); everything else is a server-side failure (500).
            let status = if e.starts_with("MISSING_ARG") || e.starts_with("INVALID_VIEW") || e.starts_with("INVALID_ARG") {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            (status, Json(json!({ "error": e }))).into_response()
        }
    }
}

/// The SSH connection a pane.open names (the field, or `connection` in a
/// caller-supplied `meta`), in its canonical form; `None` for this computer
/// or WSL.
fn ssh_connection_of(req: &crate::backend::rpc_types::CommandPaneOpenData) -> Option<String> {
    let from_meta = req
        .meta
        .as_ref()
        .and_then(|m| m.get("connection"))
        .and_then(|v| v.as_str());
    [req.connection.as_deref(), from_meta]
        .into_iter()
        .flatten()
        .find_map(|c| match crate::backend::remote::ConnTarget::parse(c.trim()) {
            Ok(t @ crate::backend::remote::ConnTarget::Ssh(_)) => Some(t.name()),
            _ => None,
        })
}

/// `POST /api/v1/agent/open` — launch an agent into a pane (idempotent: an
/// agent already open in the target tab returns its existing block with
/// `created: false`, same as the RPC path).
///
/// Called by `agentmux-mcp`'s `OpenAgent` tool. Thin HTTP wrapper over
/// `app_api::open_agent_impl` — the same logic the WebSocket `agent.open`
/// RPC uses. Body is `CommandAgentOpenData` (`agent_id` by id or name;
/// optional `tab_id`/`focus`/split placement).
pub(super) async fn handle_agent_open(
    State(state): State<AppState>,
    Json(req): Json<crate::backend::rpc_types::CommandAgentOpenData>,
) -> impl IntoResponse {
    match app_api::open_agent_impl(&state, req).await {
        Ok(result) => (StatusCode::OK, Json(json!(result))).into_response(),
        Err(e) => (open_error_status(&e), Json(json!({ "error": e }))).into_response(),
    }
}

/// The HTTP status for `agent.open`'s own error vocabulary. AGENT_NOT_FOUND /
/// TEMPLATE_NOT_OPENABLE / INVALID_PROVIDER / CLI_NOT_AVAILABLE are the caller's
/// problem (a bad target, or a CLI that could not be installed); CLI_INSTALLING
/// is neither — the CLI is on its way, so the same request will succeed shortly
/// (503, retryable); anything else is a server-side failure.
fn open_error_status(e: &str) -> StatusCode {
    if e.starts_with("CLI_INSTALLING") {
        StatusCode::SERVICE_UNAVAILABLE
    } else if e.starts_with("AGENT_NOT_FOUND")
        || e.starts_with("TEMPLATE_NOT_OPENABLE")
        || e.starts_with("INVALID_PROVIDER")
        || e.starts_with("CLI_NOT_AVAILABLE")
    {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

#[cfg(test)]
mod connection_tests {
    use super::*;
    use crate::backend::rpc_types::CommandPaneOpenData;

    fn cmd(view: &str, connection: Option<&str>, meta: Option<serde_json::Value>) -> CommandPaneOpenData {
        serde_json::from_value(serde_json::json!({
            "view": view,
            "file": "/home/u/a.txt",
            "connection": connection,
            "meta": meta,
        }))
        .unwrap()
    }

    #[test]
    fn the_ssh_host_a_pane_open_names_is_found_wherever_it_is() {
        assert_eq!(ssh_connection_of(&cmd("editor", Some("user@box"), None)).as_deref(), Some("user@box"));
        assert_eq!(
            ssh_connection_of(&cmd("files", None, Some(serde_json::json!({ "connection": "box:2222" })))).as_deref(),
            Some("box:2222")
        );
        assert_eq!(ssh_connection_of(&cmd("editor", Some("local"), None)), None);
        assert_eq!(ssh_connection_of(&cmd("editor", Some("wsl://Ubuntu"), None)), None);
        assert_eq!(ssh_connection_of(&cmd("editor", None, None)), None);
    }

    #[test]
    fn a_pane_on_a_connection_gets_it_in_its_meta_and_media_is_refused_there() {
        let meta = app_api::pane::build_pane_meta(&cmd("editor", Some("user@box"), None)).unwrap();
        assert_eq!(meta.get("connection"), Some(&serde_json::json!("user@box")));
        assert_eq!(meta.get("file"), Some(&serde_json::json!("/home/u/a.txt")));
        let local = app_api::pane::build_pane_meta(&cmd("editor", Some("local"), None)).unwrap();
        assert!(local.get("connection").is_none());
        let err = app_api::pane::build_pane_meta(&cmd("media", Some("user@box"), None)).unwrap_err();
        assert!(err.starts_with("INVALID_ARG"), "{err}");
    }
}

#[cfg(test)]
mod open_error_status_tests {
    use super::*;

    #[test]
    fn a_cli_that_is_still_installing_is_retryable_not_the_callers_fault() {
        assert_eq!(
            open_error_status("CLI_INSTALLING: claude 2.1.285 is still being installed"),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[test]
    fn the_callers_own_mistakes_are_400s() {
        for e in [
            "AGENT_NOT_FOUND: x",
            "TEMPLATE_NOT_OPENABLE: x",
            "INVALID_PROVIDER: x",
            "CLI_NOT_AVAILABLE: x could not be installed",
        ] {
            assert_eq!(open_error_status(e), StatusCode::BAD_REQUEST, "{e}");
        }
    }

    #[test]
    fn anything_else_is_a_server_failure() {
        assert_eq!(
            open_error_status("agent.open: meta serialize: boom"),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
