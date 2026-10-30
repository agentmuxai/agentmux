// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `GET /agentmux/file-icon?kind=<file|dir>&ext=<ext>&px=<n>`: the icon the
//! operating system shows for a file type, as a PNG. Takes no path: only a
//! validated type and one of a fixed list of sizes, and answers with pixels
//! alone (docs/specs/SPEC_FILES_SYSTEM_FILE_ICONS_2026_10_27.md §4.1, §6).

use axum::{
    extract::Query,
    http::{header, StatusCode},
    response::{IntoResponse, Json, Response},
};
use serde_json::json;

use crate::backend::file_icons::{self, request::IconRequest, IconError};

#[derive(serde::Deserialize)]
pub(super) struct IconQuery {
    kind: Option<String>,
    ext: Option<String>,
    px: Option<String>,
}

/// How long a page may reuse an icon without asking: a day. A changed default app
/// is a new cache entry in srv, so at worst a page shows the old icon until then.
const MAX_AGE: &str = "private, max-age=86400";

fn refusal(status: StatusCode, why: &str) -> Response {
    (status, Json(json!({ "error": why }))).into_response()
}

pub(super) async fn handle_file_icon(Query(q): Query<IconQuery>) -> Response {
    let req = match IconRequest::parse(q.kind.as_deref(), q.ext.as_deref(), q.px.as_deref()) {
        Ok(r) => r,
        Err(why) => return refusal(StatusCode::BAD_REQUEST, why),
    };
    match file_icons::icon_png(&req).await {
        Ok(png) => (
            [(header::CONTENT_TYPE, "image/png"), (header::CACHE_CONTROL, MAX_AGE)],
            png.as_ref().clone(),
        )
            .into_response(),
        Err(IconError::Unsupported) => refusal(StatusCode::NOT_IMPLEMENTED, "no system icons on this computer yet"),
        Err(IconError::NoIcon) => refusal(StatusCode::NOT_FOUND, "no icon for this type"),
        Err(IconError::Timeout) => refusal(StatusCode::GATEWAY_TIMEOUT, "the icon took too long"),
        Err(IconError::Busy) => refusal(StatusCode::SERVICE_UNAVAILABLE, "too many icon requests"),
        Err(IconError::Failed) => refusal(StatusCode::INTERNAL_SERVER_ERROR, "the icon could not be made"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request, routing::get, Router};
    use tower::ServiceExt;

    async fn status(uri: &str) -> StatusCode {
        let app = Router::new().route("/agentmux/file-icon", get(handle_file_icon));
        app.oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap()).await.unwrap().status()
    }

    #[tokio::test]
    async fn a_request_that_is_not_a_type_and_a_listed_size_is_a_400_before_anything_runs() {
        for bad in [
            "/agentmux/file-icon",
            "/agentmux/file-icon?kind=file&ext=rs",
            "/agentmux/file-icon?kind=file&ext=rs&px=17",
            "/agentmux/file-icon?kind=file&ext=..%2F..%2Fx&px=16",
            "/agentmux/file-icon?kind=file&ext=C%3A%5Cwindows%5Cnotepad.exe&px=16",
            "/agentmux/file-icon?kind=dir&ext=rs&px=16",
            "/agentmux/file-icon?kind=path&ext=rs&px=16",
        ] {
            assert_eq!(status(bad).await, StatusCode::BAD_REQUEST, "{bad}");
        }
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn without_a_system_source_the_answer_is_501_so_the_page_keeps_its_own_icons() {
        assert_eq!(status("/agentmux/file-icon?kind=file&ext=rs&px=16").await, StatusCode::NOT_IMPLEMENTED);
    }
}
