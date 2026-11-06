// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /api/v1/agent/skill`: an agent manages the skill catalog (list, get, upsert, delete, bind,
//! unbind, reach, record), the same operations Memory > Skills offers a person. Backs the `Skills` MCP tool.
//! The request carries the agent's own token; every change is recorded as that agent.

use super::*;
use crate::backend::storage::Skill;

#[derive(serde::Deserialize)]
pub(super) struct AgentSkillRequest {
    pub(super) agent_id: String,
    pub(super) action: String,
    #[serde(default)]
    pub(super) skill_id: String,
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) trigger: Option<String>,
    #[serde(default)]
    pub(super) skill_type: Option<String>,
    #[serde(default)]
    pub(super) description: Option<String>,
    #[serde(default)]
    pub(super) content: Option<String>,
    /// bind, unbind: the agent's id; default the calling agent.
    #[serde(default)]
    pub(super) target: Option<String>,
    #[serde(default)]
    pub(super) all_agents: Option<bool>,
    #[serde(default)]
    pub(super) new_agents: Option<bool>,
    /// get: a file beside the skill's `SKILL.md` (a computer-use skill's reference file, script or crop).
    #[serde(default)]
    pub(super) file: Option<String>,
    /// record: a test of a computer-use skill, as `verification/<folder>/<version>.json` states it.
    #[serde(default)]
    pub(super) skill_version: Option<String>,
    #[serde(default)]
    pub(super) app_version: Option<String>,
    #[serde(default)]
    pub(super) platform: Option<String>,
    #[serde(default)]
    pub(super) date: Option<String>,
    #[serde(default)]
    pub(super) runner: Option<String>,
    #[serde(default)]
    pub(super) route: Option<String>,
    #[serde(default)]
    pub(super) result: Option<String>,
    #[serde(default)]
    pub(super) notes: Option<String>,
    #[serde(default)]
    pub(super) evidence_sha256: Option<String>,
}

pub(super) async fn handle_agent_skill(
    State(state): State<AppState>,
    caller: Option<Extension<caller::Caller>>,
    Json(req): Json<AgentSkillRequest>,
) -> impl IntoResponse {
    let (slug, uid) = match actor::require_token_actor(&state, caller.as_deref(), actor::ActorSite::SkillManage, &req.agent_id).await {
        Ok(who) => who,
        Err(e) => return app_api_response::<()>(Err(e)),
    };
    let result = tokio::task::spawn_blocking(move || run(&state, &slug, &uid, req))
        .await
        .unwrap_or_else(|e| Err(format!("skill: {e}")));
    app_api_response(result)
}

/// A skill as the tool shows it: the text only when asked for (`get`).
fn summary(skill: &Skill, bound_count: i64, all_agents: bool, new_agents: bool, with_content: bool) -> serde_json::Value {
    let mut v = json!({
        "id": skill.id,
        "name": skill.name,
        "skill_type": skill.skill_type,
        "trigger": skill.trigger,
        "description": skill.description,
        "bound_count": bound_count,
        "all_agents": all_agents,
        "new_agents": new_agents,
    });
    if with_content {
        v["content"] = json!(skill.content);
    }
    v
}

fn global_skill(state: &AppState, id: &str) -> Result<Skill, String> {
    if id.is_empty() {
        return Err("skill: skill_id is required".to_string());
    }
    match state.identity_store.skill_get(id).map_err(|e| format!("skill: {e}"))? {
        Some(s) if s.is_global => Ok(s),
        Some(_) => Err("FORBIDDEN: only catalog skills can be managed here, not an agent's private skill".to_string()),
        None => Err(format!("skill: no skill {id}")),
    }
}

fn run(state: &AppState, slug: &str, uid: &str, req: AgentSkillRequest) -> Result<serde_json::Value, String> {
    let (mstore, catalog) = (&state.mstore, &state.identity_store);
    let changed = || state.broker.publish_event::<crate::backend::mps_events::SkillsChanged>(&(), vec![], 0);
    let target = || req.target.clone().filter(|t| !t.is_empty()).unwrap_or_else(|| uid.to_string());
    match req.action.as_str() {
        "list" => list_skills(state),
        "get" => get_skill(state, &req),
        "record" => {
            let skill = global_skill(state, &req.skill_id)?;
            let rec = verified_record(catalog, &skill, slug, &req)?;
            let added = catalog.skill_verified_append(&skill.id, &rec, slug, agentmux_common::time::now_ms()).map_err(|e| format!("skill: {e}"))?;
            tracing::info!(agent = %slug, skill = %skill.id, result = %rec.result, added, "skill: a test was recorded by an agent");
            if added {
                changed();
            }
            Ok(json!({ "recorded": added }))
        }
        "upsert" => {
            let now = agentmux_common::time::now_ms();
            let existing = if req.skill_id.is_empty() { None } else { global_skill_or_new(state, &req.skill_id)? };
            let is_new = existing.is_none();
            let id = match &existing {
                Some(s) => s.id.clone(),
                None if req.skill_id.is_empty() => uuid::Uuid::new_v4().to_string(),
                None => req.skill_id.clone(),
            };
            // An edit keeps what the request leaves out; a new skill needs a name.
            let name = match (&req.name, &existing) {
                (Some(n), _) if !n.trim().is_empty() => n.trim().to_string(),
                (_, Some(s)) => s.name.clone(),
                _ => return Err("skill: name is required".to_string()),
            };
            let pick = |given: &Option<String>, old: Option<&String>, default: &str| {
                given.clone().or_else(|| old.cloned()).unwrap_or_else(|| default.to_string())
            };
            let skill = Skill {
                id: id.clone(),
                name,
                trigger: pick(&req.trigger, existing.as_ref().map(|s| &s.trigger), ""),
                skill_type: pick(&req.skill_type, existing.as_ref().map(|s| &s.skill_type), "agent-skill"),
                description: pick(&req.description, existing.as_ref().map(|s| &s.description), ""),
                content: pick(&req.content, existing.as_ref().map(|s| &s.content), ""),
                is_global: true,
                created_at: existing.as_ref().map_or(now, |s| s.created_at),
                updated_at: now,
            };
            catalog.skill_upsert_unique_global(&skill).map_err(|e| format!("skill: {e}"))?;
            if is_new {
                catalog.skill_reach_default_for_new(&id, now).map_err(|e| format!("skill: {e}"))?;
            }
            tracing::info!(agent = %slug, skill = %id, "skill: upserted by an agent");
            changed();
            Ok(json!({ "id": id, "name": skill.name }))
        }
        "delete" => {
            let skill = global_skill(state, &req.skill_id)?;
            let deleted = mstore.skill_delete(catalog, &skill.id).map_err(|e| format!("skill: {e}"))?;
            tracing::info!(agent = %slug, skill = %skill.id, "skill: deleted by an agent");
            if deleted {
                changed();
            }
            Ok(json!({ "deleted": deleted }))
        }
        "bind" => {
            let skill = global_skill(state, &req.skill_id)?;
            mstore.skill_bind(catalog, &target(), &skill.id).map_err(|e| format!("skill: {e}"))?;
            changed();
            Ok(json!({ "bound": true }))
        }
        "unbind" => {
            let skill = global_skill(state, &req.skill_id)?;
            let unbound = mstore.skill_unbind(&target(), &skill.id).map_err(|e| format!("skill: {e}"))?;
            if unbound {
                changed();
            }
            Ok(json!({ "unbound": unbound }))
        }
        "reach" => {
            let skill = global_skill(state, &req.skill_id)?;
            let current = catalog.skill_reach_map().map_err(|e| format!("skill: {e}"))?.get(&skill.id).copied().unwrap_or_default();
            let all_agents = req.all_agents.unwrap_or(current.all_agents);
            let new_agents = req.new_agents.unwrap_or(current.new_agents_since > 0 || all_agents);
            catalog.skill_set_reach(&skill.id, all_agents, new_agents, agentmux_common::time::now_ms()).map_err(|e| format!("skill: {e}"))?;
            tracing::info!(agent = %slug, skill = %skill.id, all_agents, new_agents, "skill: reach changed by an agent");
            changed();
            Ok(json!({ "all_agents": all_agents, "new_agents": new_agents }))
        }
        other => Err(format!("skill: unknown action {other:?} (list, get, upsert, delete, bind, unbind, reach, record)")),
    }
}

/// Every catalog skill in short; a computer-use skill says so, with the app and version, so an agent can pick one for the window it is driving.
fn list_skills(state: &AppState) -> Result<serde_json::Value, String> {
    let items = state.mstore.skill_list_global(&state.identity_store).map_err(|e| format!("skill: {e}"))?;
    Ok(json!(items
        .iter()
        .map(|i| {
            let mut v = summary(&i.skill, i.bound_count, i.all_agents, i.new_agents, false);
            if let Some(app) = &i.app {
                v["app"] = json!({ "vendor": app.info.vendor, "version": app.info.version, "platforms": app.info.platforms });
            }
            v
        })
        .collect::<Vec<_>>()))
}

/// One skill in full; with `file`, one file beside its text. A computer-use skill lists those files, its reference crops and its tests.
fn get_skill(state: &AppState, req: &AgentSkillRequest) -> Result<serde_json::Value, String> {
    let (mstore, catalog) = (&state.mstore, &state.identity_store);
    let skill = global_skill(state, &req.skill_id)?;
    if let Some(file) = req.file.as_deref().filter(|f| !f.is_empty()) {
        return skill_file(catalog, &skill, file);
    }
    let items = mstore.skill_list_global(catalog).map_err(|e| format!("skill: {e}"))?;
    let item = items.iter().find(|i| i.skill.id == skill.id);
    let mut v = summary(&skill, item.map_or(0, |i| i.bound_count), item.is_some_and(|i| i.all_agents), item.is_some_and(|i| i.new_agents), true);
    if let Some(app) = item.and_then(|i| i.app.as_ref()) {
        v["app"] = serde_json::to_value(app).unwrap_or_default();
        v["note"] = json!("Read a file beside this text with get and file: its path from app.files. A reference crop comes back with its tags and a path to open as an image.");
    }
    Ok(v)
}

/// `Some(skill)` for an existing catalog skill, `None` for an id nothing has used yet (a caller may
/// choose its own id); an agent's private skill is refused.
fn global_skill_or_new(state: &AppState, id: &str) -> Result<Option<Skill>, String> {
    match state.identity_store.skill_get(id).map_err(|e| format!("skill: {e}"))? {
        Some(s) if s.is_global => Ok(Some(s)),
        Some(_) => Err("FORBIDDEN: cannot promote a private skill via the catalog".to_string()),
        None => Ok(None),
    }
}

/// `get` with `file`: one file beside a computer-use skill's `SKILL.md`. Text comes back as text. A reference crop comes
/// back as base64 with its tags, for the tool to write out and point the agent at (SPEC_PACKS §3.1).
fn skill_file(catalog: &crate::backend::storage::store::Store, skill: &Skill, file: &str) -> Result<serde_json::Value, String> {
    let app = catalog
        .skill_app_get(&skill.id)
        .map_err(|e| format!("skill: {e}"))?
        .ok_or_else(|| format!("skill: {} has no files beside its text", skill.name))?;
    let Some(f) = catalog.skill_file_get(&skill.id, file).map_err(|e| format!("skill: {e}"))? else {
        let names: Vec<&str> = app.files.iter().map(|f| f.path.as_str()).collect();
        return Err(format!("skill: {} has no file {file:?}; it has: {}", skill.name, names.join(", ")));
    };
    if let Some(bytes) = f.image {
        use base64::Engine;
        let tags = app.info.references.iter().find(|r| r.path == f.path);
        return Ok(json!({
            "id": skill.id,
            "file": f.path,
            "mime": "image/png",
            "tags": tags,
            "image_base64": base64::engine::general_purpose::STANDARD.encode(bytes),
        }));
    }
    let mut v = json!({ "id": skill.id, "file": f.path, "text": f.content });
    if f.script {
        v["script"] = json!(true);
        v["approved"] = json!(f.approved);
        if !f.approved {
            v["note"] = json!("This script has not been approved by a person, so it is not in your folder. Do not run a copy of it.");
        }
    }
    Ok(v)
}

/// The record `record` files: what a test of this skill on this app version found. Only the skill's installed version can be
/// recorded, so a record never speaks for text the skill no longer has.
fn verified_record(
    catalog: &crate::backend::storage::store::Store,
    skill: &Skill,
    slug: &str,
    req: &AgentSkillRequest,
) -> Result<crate::backend::storage::skill_app::VerifiedRecord, String> {
    let app = catalog.skill_app_get(&skill.id).map_err(|e| format!("skill: {e}"))?.ok_or_else(|| format!("skill: {} is not a computer-use skill", skill.name))?;
    let text = |v: &Option<String>| v.as_deref().map(str::trim).unwrap_or("").to_string();
    let need = |v: &Option<String>, what: &str| Some(text(v)).filter(|s| !s.is_empty()).ok_or_else(|| format!("skill: record needs {what}"));
    let skill_version = need(&req.skill_version, "skill_version")?;
    if skill_version != app.info.version {
        return Err(format!("skill: the installed version of {} is {}; a record is for that version", skill.name, app.info.version));
    }
    let app_version = need(&req.app_version, "app_version")?;
    let platform = need(&req.platform, "platform (windows, macos or linux)")?;
    if !["windows", "macos", "linux"].contains(&platform.as_str()) {
        return Err("skill: platform is windows, macos or linux".to_string());
    }
    let result = need(&req.result, "result (pass or fail)")?;
    if result != "pass" && result != "fail" {
        return Err("skill: result is pass or fail".to_string());
    }
    let route = text(&req.route);
    if !route.is_empty() && !["api", "mcp", "shortcuts", "desktop"].contains(&route.as_str()) {
        return Err("skill: route is api, mcp, shortcuts or desktop".to_string());
    }
    let evidence = text(&req.evidence_sha256).to_ascii_lowercase();
    if !evidence.is_empty() && (evidence.len() != 64 || !evidence.bytes().all(|b| b.is_ascii_hexdigit())) {
        return Err("skill: evidence_sha256 is the SHA-256 of the output file or screenshot, 64 hex digits; the evidence itself is never kept".to_string());
    }
    let date = Some(text(&req.date)).filter(|d| !d.is_empty()).unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%d").to_string());
    let well_formed = date.len() == 10 && date.bytes().enumerate().all(|(i, b)| if i == 4 || i == 7 { b == b'-' } else { b.is_ascii_digit() });
    if !well_formed {
        return Err("skill: date is YYYY-MM-DD".to_string());
    }
    let runner = Some(text(&req.runner)).filter(|r| !r.is_empty()).unwrap_or_else(|| format!("agent:{slug}"));
    let notes = text(&req.notes);
    if app_version.len() > 64 || runner.len() > 80 || notes.chars().count() > 1000 {
        return Err("skill: app_version, runner or notes is too long".to_string());
    }
    Ok(crate::backend::storage::skill_app::VerifiedRecord { skill_version, app_version, platform, date, runner, route, result, notes, evidence_sha256: evidence })
}
