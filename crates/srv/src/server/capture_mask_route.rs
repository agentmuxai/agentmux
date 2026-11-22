// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /api/v1/ui/capture_mask`: what each AgentMux window shows that a `UI` window capture blacks out
//! (docs/specs/SPEC_DEEP_LINKS_ONE_FRAMEWORK_2026_11_20.md §7, G5; `sagas::capture_mask`).

use serde::Deserialize;

use super::*;
use crate::sagas::capture_mask;

#[derive(Debug, Deserialize)]
pub(super) struct CaptureMaskRequest {
    capture_id: String,
    phase: String,
}

/// What the caller is told when it may not ask, or asked wrongly; `None` when the ask goes ahead.
fn refusal(req: &CaptureMaskRequest, agent: bool, container: bool) -> Option<(StatusCode, &'static str)> {
    if !agent {
        return Some((StatusCode::FORBIDDEN, "the capture mask is for an agent known by its token"));
    }
    if container {
        return Some((
            StatusCode::FORBIDDEN,
            "a container agent can't capture the host's windows: use UI screenshot of your own pane, or of a browser pane you opened",
        ));
    }
    let id_ok = !req.capture_id.is_empty() && req.capture_id.len() <= 64 && req.capture_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if !id_ok || !matches!(req.phase.as_str(), "begin" | "end") {
        return Some((StatusCode::BAD_REQUEST, "capture_mask needs a capture_id and a phase of begin or end"));
    }
    None
}

pub(super) async fn handle_capture_mask(
    State(state): State<AppState>,
    grant: Option<axum::Extension<crate::backend::container::credential::ContainerGrant>>,
    caller: Option<axum::Extension<super::caller::Caller>>,
    Json(req): Json<CaptureMaskRequest>,
) -> impl IntoResponse {
    let agent = caller.as_ref().and_then(|c| c.0.uid()).is_some();
    if let Some((status, why)) = refusal(&req, agent, grant.is_some()) {
        return (status, Json(json!({ "error": why }))).into_response();
    }
    let masks = capture_mask::ask(&state, &req.capture_id, &req.phase, capture_mask::WAIT_FOR).await;
    (StatusCode::OK, Json(json!({ "masks": masks }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(id: &str, phase: &str) -> CaptureMaskRequest {
        CaptureMaskRequest { capture_id: id.into(), phase: phase.into() }
    }

    #[test]
    fn only_a_host_agent_asks_and_a_container_is_told_what_to_do_instead() {
        assert_eq!(refusal(&req("c1", "begin"), false, false).map(|r| r.0), Some(StatusCode::FORBIDDEN));
        let (status, why) = refusal(&req("c1", "begin"), true, true).unwrap();
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(why.contains("UI screenshot of your own pane"), "{why}");
        assert!(refusal(&req("c1", "begin"), true, false).is_none());
        assert!(refusal(&req("c1", "end"), true, false).is_none());
    }

    #[test]
    fn a_bad_id_or_phase_is_refused() {
        for (id, phase) in [("", "begin"), ("c 1", "begin"), (&"x".repeat(65)[..], "end"), ("c1", "middle")] {
            assert_eq!(refusal(&req(id, phase), true, false).map(|r| r.0), Some(StatusCode::BAD_REQUEST), "{id:?} {phase:?}");
        }
    }
}
