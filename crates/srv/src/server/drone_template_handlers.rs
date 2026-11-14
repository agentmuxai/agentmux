// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `jobs.templates.make_drone`: a drone made from an approved Drone template
//! (docs/specs/SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md, "What is a template?").
//!
//! The template's graph is rendered with the parameters given (the defaults
//! for the rest) and saved as a new drone, so it is an ordinary drone from
//! then on: editable, versioned, and the thing a job's `drone` action runs.
//! Making it starts nothing. The template must be approved, exactly as a job
//! made from a brief-based template must.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::backend::job_templates::{self as jt, manifest::JobTemplateKind};
use crate::backend::rpc::engine::WshRpcEngine;
use crate::drone::types::{DroneDefinition, DroneViewport};
use crate::server::AppState;

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct MakeDroneReq {
    /// An approved Drone template.
    pub id: String,
    /// Its version; the installed one when left out.
    #[serde(default)]
    #[ts(optional)]
    pub version: Option<String>,
    #[serde(default)]
    #[ts(type = "Record<string, unknown>")]
    pub params: Map<String, Value>,
    /// The new drone's name; the template's when left out.
    #[serde(default)]
    #[ts(optional)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct MakeDroneResult {
    pub drone: DroneDefinition,
    /// The template pin a job made from this drone carries.
    pub template_version: String,
    pub template_hash: String,
}

/// The drone `req` describes, not yet saved.
fn build(svc: &jt::JobTemplates, req: &MakeDroneReq) -> Result<(DroneDefinition, String, String), String> {
    let id = req.id.trim();
    let version = match req.version.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) => v.to_string(),
        None => svc
            .get(id)
            .and_then(|(info, _)| info.manifest.map(|m| m.version))
            .ok_or_else(|| format!("no template {id}"))?,
    };
    let approved = svc.resolve_approved(id, &version)?;
    if approved.manifest.kind != JobTemplateKind::Drone {
        return Err(format!("{id} is not a Drone template"));
    }
    let graph = approved.graph.as_ref().ok_or_else(|| format!("{id} has no graph"))?;
    let rendered = jt::graph::render(graph, &approved.manifest, &req.params)?;
    let name = req.name.as_deref().map(str::trim).filter(|n| !n.is_empty()).unwrap_or(&approved.manifest.name);
    let drone = DroneDefinition {
        id: String::new(),
        name: name.to_string(),
        description: approved.manifest.description.clone(),
        graph: rendered,
        viewport: DroneViewport::default(),
        created_at: 0,
        updated_at: 0,
    };
    Ok((drone, approved.version, approved.hash))
}

pub(crate) fn make_drone(state: &AppState, svc: &jt::JobTemplates, req: &MakeDroneReq) -> Result<MakeDroneResult, String> {
    let (drone, template_version, template_hash) = build(svc, req)?;
    let drone = super::drone_handlers::upsert_drone(state, drone)?;
    tracing::info!(template = %req.id, drone = %drone.id, "drone made from a template");
    Ok(MakeDroneResult { drone, template_version, template_hash })
}

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let s = state.clone();
    engine.register_typed("jobs.templates.make_drone", move |req: MakeDroneReq, _ctx| {
        let s = s.clone();
        async move {
            let svc = jt::service().ok_or_else(|| "job templates aren't available on this AgentMux".to_string())?;
            make_drone(&s, svc, &req)
        }
    });
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::backend::job_templates::JobTemplates;
    use crate::drone::storage::DroneStore;

    fn service(tmp: &std::path::Path) -> JobTemplates {
        let svc = JobTemplates::new(tmp.join("job-templates"), &tmp.join("data"), &[]);
        let dir = svc.templates_dir.join("acme.report");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = json!({
            "templateVersion": 1, "id": "acme.report", "name": "Report", "version": "1.0.0", "kind": "drone",
            "description": "Tell an agent about a repository.",
            "params": { "type": "object", "properties": {
                "repo": { "type": "string" },
                "owner": { "type": "string", "default": "loap" }
            }, "required": ["repo"] },
            "trigger": { "kind": "manual" }
        });
        let graph = json!({ "nodes": [
            { "id": "tell", "position": { "x": 0, "y": 0 },
              "data": { "kind": "message", "to": "{{param.owner}}", "body": "Look at {{param.repo}}: {{trigger.kind}}" } }
        ], "edges": [] });
        std::fs::write(dir.join("template.json"), serde_json::to_vec(&manifest).unwrap()).unwrap();
        std::fs::write(dir.join("graph.json"), serde_json::to_vec(&graph).unwrap()).unwrap();
        svc
    }

    fn req(params: Value) -> MakeDroneReq {
        MakeDroneReq { id: "acme.report".into(), version: None, params: params.as_object().cloned().unwrap(), name: None }
    }

    #[tokio::test]
    async fn an_approved_template_makes_a_drone_with_its_parameters_in_it() {
        let state = crate::server::tests::test_state();
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(tmp.path());
        let r = req(json!({ "repo": "acme/site" }));
        assert!(make_drone(&state, &svc, &r).unwrap_err().contains("isn't approved"), "the operator approves first");
        let hash = svc.get("acme.report").unwrap().0.hash;
        svc.approve("acme.report", &hash, "").unwrap();

        let made = make_drone(&state, &svc, &r).unwrap();
        assert_eq!((made.drone.name.as_str(), made.template_version.as_str(), made.template_hash.as_str()), ("Report", "1.0.0", hash.as_str()));
        let data = &made.drone.graph.nodes[0].data;
        // The parameters are filled in; the drone's own references are left for the run.
        assert_eq!(data["to"], "loap");
        assert_eq!(data["body"], "Look at acme/site: {{trigger.kind}}");
        assert!(state.id_store.drone_get(&made.drone.id).unwrap().is_some(), "it is a saved drone now");
        let other = make_drone(&state, &svc, &MakeDroneReq { name: Some("Mine".into()), ..r.clone() }).unwrap();
        assert_ne!(other.drone.id, made.drone.id, "each use makes its own drone");
        assert_eq!(other.drone.name, "Mine");
    }

    #[tokio::test]
    async fn a_bad_parameter_makes_no_drone() {
        let state = crate::server::tests::test_state();
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(tmp.path());
        let hash = svc.get("acme.report").unwrap().0.hash;
        svc.approve("acme.report", &hash, "").unwrap();
        let before = state.id_store.drone_list().unwrap().len();
        for (params, why) in [
            (json!({}), "required"),
            (json!({ "repo": "x", "colour": "red" }), "takes no parameter"),
            (json!({ "repo": "{{trigger.job.name}}" }), "can't contain"),
        ] {
            let err = make_drone(&state, &svc, &req(params.clone())).unwrap_err();
            assert!(err.contains(why), "{params}: {err}");
        }
        assert_eq!(state.id_store.drone_list().unwrap().len(), before);
    }
}
