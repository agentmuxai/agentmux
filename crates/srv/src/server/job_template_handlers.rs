// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Job templates' RPCs and the operator's host route
//! (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §5.4, §5.5, §7).
//!
//! Anyone may list the templates and preview a brief. Installing one and
//! browsing a catalog are the operator's own client's (`operator_only`).
//! Approving a template, forgetting a publisher's key and pinning a
//! catalog's key are carried only by the host, as a widget's approval is:
//! the instance auth key, which every agent has, isn't enough.

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::backend::job_templates::catalog::{self, Catalog, JobTemplateCatalogEntry};
use crate::backend::job_templates::{self as jt, JobTemplateInfo, JobTemplates};
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::RpcContext;
use crate::backend::widget_signature::{self as sig, WidgetPublisherPin};

use super::AppState;

/// A catalog the operator added, as Settings shows it.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JobTemplateCatalogInfo {
    pub url: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JobTemplatesResult {
    pub templates: Vec<JobTemplateInfo>,
    pub publishers: Vec<WidgetPublisherPin>,
    pub catalogs: Vec<JobTemplateCatalogInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JobTemplateGetReq {
    pub id: String,
    /// Parameters to render the brief's preview with.
    #[serde(default)]
    #[ts(type = "Record<string, unknown> | null", optional)]
    pub params: Option<Map<String, Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JobTemplateDetail {
    pub template: JobTemplateInfo,
    /// `brief.md` as written, placeholders and all.
    pub brief: Option<String>,
    /// The brief rendered with the request's parameters, or why it can't be.
    pub preview: Option<String>,
    pub preview_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JobTemplateInstallReq {
    /// A template folder, its `template.json`, or a `.zip`.
    pub path: String,
    #[serde(default)]
    #[ts(optional)]
    pub replace: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JobTemplateInstallResult {
    pub id: String,
    pub templates: Vec<JobTemplateInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JobTemplateIdReq {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JobTemplateCatalogReq {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JobTemplateCatalogResult {
    pub url: String,
    pub entries: Vec<JobTemplateCatalogEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JobTemplateCatalogInstallReq {
    pub url: String,
    pub id: String,
}

fn svc() -> Result<&'static Arc<JobTemplates>, String> {
    jt::service().ok_or_else(|| "job templates aren't available on this AgentMux".to_string())
}

/// `f` on a blocking thread: every call reads and hashes template files.
async fn blocking<T: Send + 'static>(f: impl FnOnce(&JobTemplates) -> Result<T, String> + Send + 'static) -> Result<T, String> {
    let s = svc()?.clone();
    tokio::task::spawn_blocking(move || f(&s)).await.map_err(|e| e.to_string())?
}

fn result(s: &JobTemplates) -> JobTemplatesResult {
    JobTemplatesResult {
        templates: s.list(),
        publishers: s.publishers(),
        catalogs: s.catalogs().into_iter().map(|c| JobTemplateCatalogInfo { fingerprint: sig::fingerprint(&c.public_key), url: c.url }).collect(),
    }
}

fn detail(s: &JobTemplates, req: JobTemplateGetReq) -> Result<JobTemplateDetail, String> {
    let (template, pkg) = s.get(&req.id).ok_or_else(|| format!("no job template {}", req.id))?;
    let brief = pkg.as_ref().and_then(|p| p.brief.clone());
    let rendered = match (&pkg, &req.params) {
        (Some(p), Some(params)) => Some(jt::manifest::render(&p.manifest, p.brief.as_deref().unwrap_or(""), params)),
        _ => None,
    };
    let (preview, preview_error) = match rendered {
        Some(Ok(r)) => (Some(r.brief), None),
        Some(Err(e)) => (None, Some(e)),
        None => (None, None),
    };
    Ok(JobTemplateDetail { template, brief, preview, preview_error })
}

async fn install(req: JobTemplateInstallReq, ctx: &RpcContext) -> Result<JobTemplateInstallResult, String> {
    ctx.operator_only("only the operator installs a job template")?;
    let source = std::path::PathBuf::from(req.path.trim());
    let replace = req.replace.unwrap_or(false);
    blocking(move |s| {
        let id = s.install(&source, replace)?;
        tracing::info!(id = %id, "job template installed (awaiting approval)");
        Ok(JobTemplateInstallResult { id, templates: s.list() })
    })
    .await
}

fn pinned_catalog(url: &str) -> Result<Catalog, String> {
    svc()?.catalogs().into_iter().find(|c| c.url == url).ok_or_else(|| format!("{url} isn't one of your catalogs"))
}

pub fn register(engine: &Arc<WshRpcEngine>) {
    engine.register_typed("jobs.templates.list", |_req: Value, _ctx| async move { blocking(|s| Ok(result(s))).await });

    engine.register_typed("jobs.templates.get", |req: JobTemplateGetReq, _ctx| async move { blocking(move |s| detail(s, req)).await });

    engine.register_typed("jobs.templates.install", |req: JobTemplateInstallReq, ctx| async move { install(req, &ctx).await });

    engine.register_typed("jobs.templates.uninstall", |req: JobTemplateIdReq, ctx: RpcContext| async move {
        ctx.operator_only("only the operator uninstalls a job template")?;
        blocking(move |s| {
            s.uninstall(&req.id)?;
            tracing::info!(id = %req.id, "job template uninstalled");
            Ok(result(s))
        })
        .await
    });

    // jobs.templates.catalog: a private catalog's templates, its index
    // checked against the key the operator pinned for it.
    engine.register_typed("jobs.templates.catalog", |req: JobTemplateCatalogReq, ctx: RpcContext| async move {
        ctx.operator_only("only the operator browses a job template catalog")?;
        let entries = catalog::fetch(&pinned_catalog(&req.url)?).await?;
        Ok(JobTemplateCatalogResult { url: req.url, entries })
    });

    // jobs.templates.catalog.install: download one, check it against the
    // freshly fetched index, and install it, waiting for approval.
    engine.register_typed("jobs.templates.catalog.install", |req: JobTemplateCatalogInstallReq, ctx: RpcContext| async move {
        ctx.operator_only("only the operator installs a job template")?;
        let entries = catalog::fetch(&pinned_catalog(&req.url)?).await?;
        let entry = entries.iter().find(|e| e.id == req.id).ok_or_else(|| format!("the catalog doesn't list {}", req.id))?;
        let id = catalog::install(svc()?.clone(), entry).await?;
        tracing::info!(id = %id, version = %entry.version, "catalog job template installed (awaiting approval)");
        Ok(JobTemplateInstallResult { id, templates: blocking(|s| Ok(s.list())).await? })
    });
}

/// `POST /api/v1/host/job_templates`: the operator's decisions, relayed by
/// the host with its own token. `action` is `approve` (`id`, `hash`, and the
/// `signer` fingerprint their prompt showed), `forget_key` (`publisher`),
/// `add_catalog` (`url`, `public_key`) or `remove_catalog` (`url`).
pub(crate) async fn handle_host_job_templates(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    if !super::ui_handlers::from_host(&state, &headers).await {
        return (StatusCode::FORBIDDEN, Json(json!({ "ok": false, "error": "only the AgentMux host can decide on a job template" }))).into_response();
    }
    let s = |k: &str| body.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let (action, id, hash, signer, publisher, url, key) = (s("action"), s("id"), s("hash"), s("signer"), s("publisher"), s("url"), s("public_key"));
    let decided = blocking(move |t| {
        match action.as_str() {
            "approve" => t.approve(&id, &hash, &signer).inspect(|()| tracing::info!(id = %id, "job template approved by the operator")),
            "forget_key" => t.forget_publisher(&publisher),
            "add_catalog" => t.add_catalog(&url, &key),
            "remove_catalog" => t.remove_catalog(&url),
            _ => Err(format!("unknown action {action:?}")),
        }?;
        Ok(result(t))
    })
    .await;
    match decided {
        Ok(r) => (StatusCode::OK, Json(json!({ "ok": true, "result": r }))).into_response(),
        Err(e) => (StatusCode::CONFLICT, Json(json!({ "ok": false, "error": e }))).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn host_post(token: Option<&str>, body: Value) -> Request<Body> {
        let mut b = Request::builder()
            .uri("/api/v1/host/job_templates")
            .method("POST")
            .header("X-AuthKey", "test-secret-key")
            .header("Content-Type", "application/json");
        if let Some(t) = token {
            b = b.header("X-Host-Token", t);
        }
        b.body(Body::from(body.to_string())).unwrap()
    }

    async fn status(app: &axum::Router, req: Request<Body>) -> StatusCode {
        app.clone().oneshot(req).await.unwrap().status()
    }

    // §7: agents can't approve a template or pin a catalog's key; the
    // instance auth key, which every agent has, isn't enough, only the
    // host's token is.
    #[tokio::test]
    async fn only_the_host_approves_a_template_or_pins_a_catalog() {
        let svc = JobTemplates::for_tests();
        let dir = svc.templates_dir.join("acme.hostonly");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("template.json"),
            json!({ "templateVersion": 1, "id": "acme.hostonly", "name": "Host only", "version": "1.0.0", "kind": "job",
                    "description": "A test.", "action": { "kind": "message" } })
            .to_string(),
        )
        .unwrap();
        std::fs::write(dir.join("brief.md"), "Report in.").unwrap();
        let hash = svc.get("acme.hostonly").unwrap().0.hash;

        let state = crate::server::tests::test_state();
        *state.host_ipc.lock().await = Some(crate::server::state::HostIpc { port: 1, token: "host-ipc-token".into() });
        let app = crate::server::build_router(state);
        let approve = json!({ "action": "approve", "id": "acme.hostonly", "hash": hash, "signer": "" });
        assert_eq!(status(&app, host_post(None, approve.clone())).await, StatusCode::FORBIDDEN);
        assert_eq!(status(&app, host_post(Some("guess"), approve.clone())).await, StatusCode::FORBIDDEN);
        assert!(svc.resolve_approved("acme.hostonly", "1.0.0").is_err());
        assert_eq!(status(&app, host_post(Some("host-ipc-token"), approve)).await, StatusCode::OK);
        assert!(svc.resolve_approved("acme.hostonly", "1.0.0").is_ok());

        let key = sig::test_keys::public_key(9);
        let add = json!({ "action": "add_catalog", "url": "https://templates.example/index.json", "public_key": key });
        assert_eq!(status(&app, host_post(None, add.clone())).await, StatusCode::FORBIDDEN);
        assert_eq!(status(&app, host_post(Some("host-ipc-token"), add)).await, StatusCode::OK);
        assert!(svc.catalogs().iter().any(|c| c.public_key == key));
    }

    #[tokio::test]
    async fn an_agent_connection_cannot_install_a_template() {
        let agent = RpcContext { agent_id: "agent7".into(), ..Default::default() };
        let req = JobTemplateInstallReq { path: "/nowhere".into(), replace: None };
        assert!(install(req, &agent).await.unwrap_err().starts_with("FORBIDDEN"));
    }
}
