// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /api/v1/ui/browser/locate` — backs Browser's `locate`, and the first half of `click_at`: what a
//! click at a point of a screenshot would land on, as a reference that `click`, `fill`, `select` and
//! `check` take (the visual fallback, docs/specs/SPEC_BROWSER_PANE_FORMS_IN_FRAMES_AND_VISUAL_FALLBACK_2026_10_16.md
//! §4.6). The host does the hit test and keeps the reference; srv checks that the pane is the caller's and
//! turns the screenshot's pixels into the page's.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;

use agentmux_common::api_types::UiBrowserLocateRequest;

use crate::server::ui_handlers::{err_response, proxy_data, target_block_id};
use crate::server::AppState;

/// The point and the screenshot's width, checked: the host turns them into the page's pixels, as it alone knows
/// the page's width.
fn host_body(block_id: &str, req: &UiBrowserLocateRequest) -> Result<serde_json::Value, String> {
    if !req.x.is_finite() || !req.y.is_finite() || req.x < 0.0 || req.y < 0.0 {
        return Err("x and y are positions in the screenshot, counted from its top left corner".to_string());
    }
    if req.image_width.is_some_and(|w| !w.is_finite() || w < 1.0) {
        return Err("image_width is the screenshot's width in pixels".to_string());
    }
    Ok(json!({ "block_id": block_id, "x": req.x, "y": req.y, "image_width": req.image_width }))
}

pub(crate) async fn handle_ui_browser_locate(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<UiBrowserLocateRequest>,
) -> impl IntoResponse {
    // The same checks as a snapshot: the caller's identity, that the pane is theirs, and that it isn't
    // waiting for the user.
    let block_id = match target_block_id(&state, caller.as_deref(), &req.auth, req.pane.as_deref()) {
        Ok(b) => b,
        Err((code, e)) => return err_response(code, e),
    };
    let body = match host_body(&block_id, &req) {
        Ok(b) => b,
        Err(e) => return err_response(StatusCode::BAD_REQUEST, e),
    };
    tracing::info!(agent_id = %req.auth.agent_id, block_id = %block_id, "[ui-automation] browser locate");
    proxy_data(&state, "locate", body, std::time::Duration::from_secs(30)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(x: f64, y: f64, image_width: Option<f64>) -> UiBrowserLocateRequest {
        serde_json::from_value(json!({ "agent_id": "a", "ts_secs": 1, "sig": "s", "x": x, "y": y, "image_width": image_width })).unwrap()
    }

    #[test]
    fn the_host_gets_the_pane_the_point_and_the_images_width() {
        let body = host_body("b1", &req(200.0, 100.0, Some(1920.0))).unwrap();
        assert_eq!(body, json!({ "block_id": "b1", "x": 200.0, "y": 100.0, "image_width": 1920.0 }));
        let plain = host_body("b1", &req(10.0, 20.0, None)).unwrap();
        assert_eq!(plain["image_width"], serde_json::Value::Null);
    }

    #[test]
    fn a_point_or_width_that_cannot_be_in_an_image_is_refused() {
        assert!(host_body("b", &req(-1.0, 1.0, None)).unwrap_err().starts_with("x and y are"));
        assert!(host_body("b", &req(1.0, -0.5, Some(800.0))).is_err());
        for bad in [0.0, -5.0, 0.5] {
            assert!(host_body("b", &req(1.0, 1.0, Some(bad))).unwrap_err().starts_with("image_width is"));
        }
    }

    #[test]
    fn the_request_reads_its_point_beside_the_signature() {
        let r: UiBrowserLocateRequest =
            serde_json::from_value(json!({ "agent_id": "a", "ts_secs": 1, "sig": "s", "pane": "p", "x": 3, "y": 4 })).unwrap();
        assert_eq!((r.x, r.y, r.image_width, r.pane.as_deref()), (3.0, 4.0, None, Some("p")));
    }
}
