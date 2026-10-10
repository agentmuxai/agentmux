// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The widget packages' RPCs and routes
//! (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §5.2, §8):
//! listing and managing packages, serving an approved package's files, and
//! the user's approval, which only the host can carry.

use std::sync::Arc;

use axum::extract::{Path as UrlPath, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::widget_packages::{self, FileError, WidgetKind, WidgetPackageInfo};

use super::AppState;

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct WidgetPackagesResult {
    pub packages: Vec<WidgetPackageInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct WidgetIdReq {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct WidgetSetEnabledReq {
    pub id: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct WidgetInstallReq {
    /// A package folder, its `widget.json`, or a `.zip`.
    pub path: String,
    /// Replace an installed package with the same id.
    #[serde(default)]
    pub replace: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct WidgetInstallResult {
    pub id: String,
    pub packages: Vec<WidgetPackageInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct WidgetReadFileReq {
    pub id: String,
    pub hash: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct WidgetReadFileResult {
    pub content: String,
}

fn svc() -> Result<&'static Arc<widget_packages::WidgetPackages>, String> {
    widget_packages::service().ok_or_else(|| "widget packages aren't available on this AgentMux".to_string())
}

pub fn register_widget_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // widgets.list: every package and its state.
    engine.register_typed("widgets.list", |_req: serde_json::Value, _ctx| async move {
        Ok(WidgetPackagesResult { packages: svc()?.list() })
    });

    let st = state.clone();
    engine.register_typed("widgets.rescan", move |_req: serde_json::Value, _ctx| {
        let st = st.clone();
        async move {
            svc()?;
            Ok(WidgetPackagesResult { packages: widget_packages::refresh_off_thread(&st.config_watcher, &st.event_bus, &st.broker).await })
        }
    });

    let st = state.clone();
    engine.register_typed("widgets.setenabled", move |req: WidgetSetEnabledReq, _ctx| {
        let st = st.clone();
        async move {
            svc()?.set_enabled(&req.id, req.enabled)?;
            Ok(WidgetPackagesResult { packages: widget_packages::refresh_off_thread(&st.config_watcher, &st.event_bus, &st.broker).await })
        }
    });

    // widgets.install: copy a package in. It doesn't run until the user
    // approves it in the UI (the host-only route below).
    let st = state.clone();
    engine.register_typed("widgets.install", move |req: WidgetInstallReq, _ctx| {
        let st = st.clone();
        async move {
            let dir = svc()?.widgets_dir.clone();
            let source = std::path::PathBuf::from(req.path.trim());
            let id = tokio::task::spawn_blocking(move || widget_packages::install(&dir, &source, req.replace))
                .await
                .map_err(|e| e.to_string())??;
            tracing::info!(id = %id, "widget package installed (awaiting approval)");
            let packages = widget_packages::refresh_off_thread(&st.config_watcher, &st.event_bus, &st.broker).await;
            Ok(WidgetInstallResult { id, packages })
        }
    });

    // widgets.uninstall: delete the package folder and its approval. A v1
    // widget is a widgets.json entry, which stays the user's to edit.
    let st = state.clone();
    engine.register_typed("widgets.uninstall", move |req: WidgetIdReq, _ctx| {
        let st = st.clone();
        async move {
            let s = svc()?;
            let (dir, implied) = s.package_dir(&req.id).ok_or("no such widget")?;
            if implied {
                return Err(format!("{} comes from widgets.json: remove its entry there", req.id));
            }
            if !dir.starts_with(&s.widgets_dir) {
                return Err("that widget isn't in the widgets folder".to_string());
            }
            tokio::task::spawn_blocking(move || std::fs::remove_dir_all(&dir))
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| format!("can't remove it: {e}"))?;
            s.forget(&req.id)?;
            tracing::info!(id = %req.id, "widget package uninstalled");
            Ok(WidgetPackagesResult { packages: widget_packages::refresh_off_thread(&st.config_watcher, &st.event_bus, &st.broker).await })
        }
    });

    // widgets.readfile: a trusted widget's module, for the loader. Only the
    // approved version's bytes, checked like the files route.
    engine.register_typed("widgets.readfile", |req: WidgetReadFileReq, _ctx| async move {
        let s = svc()?;
        let pkg = s.list().into_iter().find(|p| p.id == req.id).ok_or("no such widget")?;
        if pkg.kind != WidgetKind::Trusted {
            return Err("only a trusted widget's module is read this way".to_string());
        }
        let key = pkg
            .files_url
            .as_deref()
            .and_then(|u| u.trim_end_matches('/').rsplit('/').next())
            .ok_or("that widget isn't approved and enabled")?
            .to_string();
        let bytes = s.read_file(&req.id, &req.hash, &key, &req.path).map_err(|e| match e {
            FileError::NotFound => "no such file in the approved widget".to_string(),
            FileError::Changed => "the widget changed since it was approved".to_string(),
        })?;
        Ok(WidgetReadFileResult { content: String::from_utf8_lossy(&bytes).into_owned() })
    });
}

/// `GET /agentmux/widget-files/<id>/<hash>/<key>/<path>`: an approved,
/// enabled package's file, as approved (spec §5.2). No auth key: an iframe
/// loads these, and the key segment is what keeps other web pages out.
pub(crate) async fn handle_widget_file(UrlPath((id, hash, key, path)): UrlPath<(String, String, String, String)>) -> Response {
    let Ok(s) = svc() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let s = s.clone();
    let path_for_read = path.clone();
    let read = tokio::task::spawn_blocking(move || s.read_file(&id, &hash, &key, &path_for_read)).await;
    let bytes = match read {
        Ok(Ok(b)) => b,
        Ok(Err(FileError::Changed)) => {
            return (StatusCode::CONFLICT, "this widget changed since it was approved").into_response();
        }
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    let mut resp = bytes.into_response();
    let h = resp.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(widget_packages::content_type(&path)));
    h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(widget_packages::WIDGET_CSP));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    resp
}

/// `POST /api/v1/host/widget_approval`: the user's answer to an install
/// prompt (spec §8.3), relayed by the CEF host with its own token. The
/// instance auth key, which every agent has, isn't enough.
pub(crate) async fn handle_host_widget_approval(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Response {
    if !super::ui_handlers::from_host(&state, &headers).await {
        return (StatusCode::FORBIDDEN, Json(json!({ "ok": false, "error": "only the AgentMux host can approve a widget" })))
            .into_response();
    }
    let s = |k: &str| body.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let (id, hash, decision) = (s("id"), s("hash"), s("decision"));
    let Ok(service) = svc() else {
        return (StatusCode::NOT_FOUND, Json(json!({ "ok": false, "error": "widget packages aren't available" }))).into_response();
    };
    let result = match decision.as_str() {
        "approve" => service.approve(&id, &hash),
        "cancel" => Ok(()),
        _ => Err(format!("unknown decision {decision:?}")),
    };
    match result {
        Ok(()) => {
            if decision == "approve" {
                tracing::info!(id = %id, "widget package approved by the user");
            }
            let packages = widget_packages::refresh_off_thread(&state.config_watcher, &state.event_bus, &state.broker).await;
            (StatusCode::OK, Json(json!({ "ok": true, "packages": packages }))).into_response()
        }
        Err(e) => (StatusCode::CONFLICT, Json(json!({ "ok": false, "error": e }))).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    async fn send(app: &axum::Router, req: Request<Body>) -> (StatusCode, HeaderMap, Vec<u8>) {
        let resp = app.clone().oneshot(req).await.unwrap();
        let (status, headers) = (resp.status(), resp.headers().clone());
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap().to_vec();
        (status, headers, body)
    }

    fn approval(headers: &[(&str, &str)], id: &str, hash: &str) -> Request<Body> {
        let mut b = Request::builder()
            .uri("/api/v1/host/widget_approval")
            .method("POST")
            .header("X-AuthKey", "test-secret-key")
            .header("Content-Type", "application/json");
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        b.body(Body::from(json!({ "id": id, "hash": hash, "decision": "approve" }).to_string())).unwrap()
    }

    fn get(uri: &str) -> Request<Body> {
        Request::builder().uri(uri).body(Body::empty()).unwrap()
    }

    // SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §5.2, §8.3: nothing is
    // served before the user approves; the instance auth key, which every
    // agent has, can't approve; only the host's token can; and an edit after
    // approval is never served.
    #[tokio::test]
    async fn only_the_host_can_approve_and_only_approved_bytes_are_served() {
        let tmp = tempfile::tempdir().unwrap();
        let widgets = tmp.path().join("widgets");
        let pkg = widgets.join("acme.test");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(
            pkg.join("widget.json"),
            json!({ "manifestVersion": 1, "id": "acme.test", "name": "Test", "version": "1.0.0", "contributes": { "panes": [{ "name": "main" }] } })
                .to_string(),
        )
        .unwrap();
        std::fs::write(pkg.join("index.html"), "<p>approved</p>").unwrap();
        let svc = widget_packages::WidgetPackages::new(widgets.clone(), &tmp.path().join("data"), "test-secret-key".into())
            .install_global();
        let hash = svc.rescan(&Default::default())[0].hash.clone();
        let key = widget_packages::files_key("test-secret-key", "acme.test", &hash);
        let file_uri = format!("/agentmux/widget-files/acme.test/{hash}/{key}/index.html");

        let state = crate::server::tests::test_state();
        *state.host_ipc.lock().await = Some(crate::server::state::HostIpc { port: 1, token: "host-ipc-token".into() });
        let app = crate::server::build_router(state);

        let (s, _, _) = send(&app, get(&file_uri)).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "not served before approval");

        let (s, _, _) = send(&app, approval(&[], "acme.test", &hash)).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "the auth key alone can't approve");
        let (s, _, _) = send(&app, approval(&[("X-Host-Token", "guess")], "acme.test", &hash)).await;
        assert_eq!(s, StatusCode::FORBIDDEN);
        let (s, _, _) = send(&app, approval(&[("X-Host-Token", "host-ipc-token")], "acme.test", "stale")).await;
        assert_eq!(s, StatusCode::CONFLICT, "an approval of another version is refused");
        let (s, _, body) = send(&app, approval(&[("X-Host-Token", "host-ipc-token")], "acme.test", &hash)).await;
        assert_eq!(s, StatusCode::OK, "{}", String::from_utf8_lossy(&body));

        let (s, headers, body) = send(&app, get(&file_uri)).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(body, b"<p>approved</p>");
        assert!(headers[header::CONTENT_SECURITY_POLICY].to_str().unwrap().contains("connect-src 'none'"));
        assert_eq!(headers[header::CONTENT_TYPE], "text/html; charset=utf-8");

        let (s, _, _) = send(&app, get(&format!("/agentmux/widget-files/acme.test/{hash}/wrongkey/index.html"))).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "the key segment is required");

        std::fs::write(pkg.join("index.html"), "<p>edited</p>").unwrap();
        let (s, _, body) = send(&app, get(&file_uri)).await;
        assert_eq!(s, StatusCode::CONFLICT, "an edit after approval isn't served");
        assert!(!String::from_utf8_lossy(&body).contains("edited"));
    }
}
