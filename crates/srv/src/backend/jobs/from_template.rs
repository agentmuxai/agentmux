// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! A job made from a template, and a template's update diff
//! (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §5.4).
//!
//! [`apply`] works on the definition as JSON, before it is parsed: it pins the
//! job to the approved template (id, version, hash), checks the parameters
//! against the template's schema, renders the brief from them, and sets the
//! outputs and secrets the template declares. A template that isn't approved,
//! or changed since it was, can't make a job (`resolve_approved`). On a create
//! it also fills in what the request left out from the template's defaults
//! (name, trigger, action, policy).
//!
//! A job keeps the version it was made from. A newer one is offered with
//! [`update_diff`]; accepting it is an edit that names the new version, which
//! [`apply`] reads as an update and lays the template's policy defaults over
//! the job's.

use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::backend::job_templates::{self as jt, ApprovedTemplate, JobTemplateState, JobTemplates};
use crate::backend::jobs::definition::{JobDefinition, TemplateRef};

/// Whether an edit (`fields` laid over `before`) changes what a template's
/// render depends on: its pin or parameters, or the brief itself. An action
/// edit that leaves the brief as it was (a new target) needs none. One that
/// leaves the brief out, or writes another, is rendered again, so the
/// template stays the brief's only author.
pub fn edit_needs_render(fields: &Map<String, Value>, before: &JobDefinition) -> bool {
    if fields.contains_key("template") || fields.contains_key("params") {
        return true;
    }
    let before = serde_json::to_value(before).unwrap_or_default();
    fields.get("action").is_some_and(|action| {
        let sent = Value::Object(Map::from_iter([("action".to_string(), action.clone())]));
        brief_of(&sent) != brief_of(&before)
    })
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn service() -> Result<&'static std::sync::Arc<JobTemplates>, String> {
    jt::service().ok_or_else(|| "job templates aren't available on this AgentMux".to_string())
}

/// The template reference `def` carries, if any.
fn reference(def: &Value) -> Option<&Value> {
    def.get("template").filter(|t| !t.is_null())
}

/// Lay the approved template under `def`, a job definition as JSON. Does
/// nothing when `def` names no template.
///
/// `previous` is the pin the job has now, on an edit; `None` on a create. A
/// create takes the template's defaults for what `def` leaves out. An edit
/// that names a different version or content than `previous` also takes the
/// template's policy defaults over the job's (what [`update_diff`] showed).
pub fn apply(def: &mut Value, previous: Option<&TemplateRef>) -> Result<(), String> {
    if reference(def).is_none() {
        return Ok(());
    }
    apply_with(def, previous, service()?)
}

pub(crate) fn apply_with(def: &mut Value, previous: Option<&TemplateRef>, svc: &JobTemplates) -> Result<(), String> {
    let Some(asked) = reference(def).cloned() else { return Ok(()) };
    let id = text(&asked, "id").trim().to_string();
    if id.is_empty() {
        return Err("template.id is required".to_string());
    }
    let version = match text(&asked, "version").trim() {
        "" => svc
            .get(&id)
            .and_then(|(info, _)| info.manifest.map(|m| m.version))
            .ok_or_else(|| format!("no job template {id}"))?,
        v => v.to_string(),
    };
    let approved = svc.resolve_approved(&id, &version).map_err(|e| {
        if previous.is_some() && e.contains(" is at version ") {
            format!("{e}: this job is pinned to {version}; accept the template's update first")
        } else {
            e
        }
    })?;
    let hash = text(&asked, "hash");
    if !hash.is_empty() && hash != approved.hash {
        return Err(format!("{id} changed since the job's template pin was recorded; it needs the operator's approval again"));
    }
    let Some(obj) = def.as_object_mut() else { return Err("a job definition is a JSON object".to_string()) };
    let given = obj.get("params").and_then(Value::as_object).cloned().unwrap_or_default();
    let rendered = approved.render(&given)?;
    let moved = previous.is_some_and(|p| p.id != approved.id || p.version != approved.version || p.hash != approved.hash);
    obj.insert("params".into(), Value::Object(rendered.params.clone()));
    obj.insert("template".into(), json!({ "id": approved.id, "version": approved.version, "hash": approved.hash }));
    obj.insert("outputs".into(), json!(approved.manifest.outputs));
    obj.insert("secrets".into(), secrets_of(&approved, &rendered.params));
    set_brief(obj, &approved, &rendered.brief);
    if previous.is_none() {
        fill_defaults(obj, &approved);
    } else if moved {
        let overlay = overlay_policy(obj.get("policy"), template_policy(&approved).as_ref());
        obj.insert("policy".into(), overlay);
    }
    Ok(())
}

/// The template's secrets as the definition's: its declared environment names
/// and the Vault entries the credential parameters name. A credential the
/// parameters leave out (an optional one) is not a secret of this job.
fn secrets_of(t: &ApprovedTemplate, params: &Map<String, Value>) -> Value {
    let list: Vec<Value> = t
        .manifest
        .secrets
        .iter()
        .filter_map(|s| params.get(&s.param).and_then(Value::as_str).map(|vault| json!({ "name": s.name, "vault": vault })))
        .collect();
    Value::Array(list)
}

/// The action with the rendered brief in it: a task's payload, else the brief.
fn set_brief(obj: &mut Map<String, Value>, t: &ApprovedTemplate, brief: &str) {
    let mut action = obj.get("action").and_then(Value::as_object).cloned().unwrap_or_default();
    if text(&Value::Object(action.clone()), "kind").is_empty() {
        if let Some(kind) = t.manifest.action.as_ref().and_then(|a| a.get("kind")) {
            action.insert("kind".into(), kind.clone());
        }
    }
    let field = if action.get("kind").and_then(Value::as_str) == Some("task") { "payload" } else { "brief" };
    action.insert(field.into(), Value::String(brief.to_string()));
    obj.insert("action".into(), Value::Object(action));
}

/// What a create leaves out, from the template: the name, the trigger and the
/// policy. The request's own fields win.
fn fill_defaults(obj: &mut Map<String, Value>, t: &ApprovedTemplate) {
    if obj.get("name").and_then(Value::as_str).is_none_or(|n| n.trim().is_empty()) {
        obj.insert("name".into(), Value::String(t.manifest.name.clone()));
    }
    if obj.get("trigger").is_none_or(Value::is_null) {
        obj.insert("trigger".into(), t.manifest.trigger.clone().unwrap_or_else(|| json!({ "kind": "manual" })));
    }
    if let Some(defaults) = template_policy(t) {
        let own = obj.get("policy").cloned();
        obj.insert("policy".into(), overlay_policy(Some(&defaults), own.as_ref()));
    }
    // The action's own fields (a target) win over the template's kind.
    if let Some(base) = t.manifest.action.as_ref().and_then(Value::as_object) {
        let mut action = base.clone();
        for (k, v) in obj.get("action").and_then(Value::as_object).cloned().unwrap_or_default() {
            action.insert(k, v);
        }
        obj.insert("action".into(), Value::Object(action));
    }
}

/// Policy settings this build can't run yet (`cron::runnable_yet`). A
/// template's default for one is left out, so it doesn't refuse every job
/// made from the template.
const NOT_RUNNABLE_YET: [&str; 1] = ["max_turns"];

/// The template's policy defaults a job can take.
fn template_policy(t: &ApprovedTemplate) -> Option<Value> {
    let mut policy = t.manifest.policy.clone()?;
    if let Some(obj) = policy.as_object_mut() {
        NOT_RUNNABLE_YET.iter().for_each(|k| {
            obj.remove(*k);
        });
    }
    Some(policy)
}

/// `base` with `top` laid over it, key by key, and `caps` key by key.
fn overlay_policy(base: Option<&Value>, top: Option<&Value>) -> Value {
    let mut out = base.and_then(Value::as_object).cloned().unwrap_or_default();
    for (key, value) in top.and_then(Value::as_object).into_iter().flatten() {
        match (key.as_str(), out.get(key).and_then(Value::as_object), value.as_object()) {
            ("caps", Some(have), Some(over)) => {
                let mut caps = have.clone();
                caps.extend(over.iter().map(|(k, v)| (k.clone(), v.clone())));
                out.insert(key.clone(), Value::Object(caps));
            }
            _ => {
                out.insert(key.clone(), value.clone());
            }
        }
    }
    Value::Object(out)
}

// ── The Jobs tab's view of a pin ────────────────────────────────────────────

/// A newer version of a job's template, installed.
#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct TemplateUpdateView {
    pub version: String,
    /// The operator approved it: accepting is possible. If not, the tab
    /// sends them to approve it first.
    pub approved: bool,
}

/// The template a job was made from, as the tab's row shows it.
#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct TemplatePinView {
    pub id: String,
    pub version: String,
    pub update: Option<TemplateUpdateView>,
}

/// What the installed template is, for the pins that name it.
pub type Latest = std::collections::HashMap<String, Option<(String, String, bool)>>;

/// `pin` as the row shows it. `latest` remembers each template read, so a
/// list of jobs made from one reads it once.
pub fn pin_view(pin: &TemplateRef, latest: &mut Latest) -> TemplatePinView {
    let newest = latest
        .entry(pin.id.clone())
        .or_insert_with(|| {
            let (info, _) = jt::service()?.get(&pin.id)?;
            Some((info.manifest?.version, info.hash, info.state == JobTemplateState::Approved))
        })
        .clone();
    let update = newest
        .filter(|(version, hash, _)| *version != pin.version || *hash != pin.hash)
        .map(|(version, _, approved)| TemplateUpdateView { version, approved });
    TemplatePinView { id: pin.id.clone(), version: pin.version.clone(), update }
}

// ── The update diff ─────────────────────────────────────────────────────────

/// What `jobs.templates.update_diff` says about a job's template.
#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub enum UpdateStatus {
    /// The job isn't made from a template, or is at the template's version.
    None,
    /// A newer version is installed and approved: the diff is what accepting does.
    Available,
    /// A newer version is installed, but the operator hasn't approved it.
    NeedsApproval,
    /// The template is gone or invalid, so there is nothing to update to.
    Unavailable,
}

/// A text before and after.
#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct TextChange {
    pub before: String,
    pub after: String,
}

/// A list's difference.
#[derive(Debug, Clone, Default, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ListChange {
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

impl ListChange {
    fn between(before: &[String], after: &[String]) -> Option<ListChange> {
        let added: Vec<String> = after.iter().filter(|a| !before.contains(a)).cloned().collect();
        let removed: Vec<String> = before.iter().filter(|b| !after.contains(b)).cloned().collect();
        (!added.is_empty() || !removed.is_empty()).then_some(ListChange { added, removed })
    }
}

/// One policy setting that accepting the update changes.
#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct PolicyChange {
    /// `timeout_minutes`, `caps.run_usd`…
    pub field: String,
    /// The job's value now, in words (`—` when it has none).
    pub before: String,
    pub after: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct TemplateUpdateDiff {
    pub job_id: String,
    pub template_id: String,
    pub status: UpdateStatus,
    /// Why there is no update to accept, in words.
    pub reason: Option<String>,
    pub from_version: String,
    pub to_version: String,
    /// What the update's pin would be: the content the operator sees.
    pub to_hash: String,
    pub brief: Option<TextChange>,
    pub outputs: Option<ListChange>,
    pub secrets: Option<ListChange>,
    pub policy: Vec<PolicyChange>,
    /// The new version can't take the job's parameters as they are.
    pub params_error: Option<String>,
}

impl TemplateUpdateDiff {
    fn without_update(job: &JobDefinition, status: UpdateStatus, reason: Option<String>) -> Self {
        let pin = job.template.clone().unwrap_or(TemplateRef { id: String::new(), version: String::new(), hash: String::new() });
        TemplateUpdateDiff {
            job_id: job.id.clone(),
            template_id: pin.id,
            status,
            reason,
            from_version: pin.version.clone(),
            to_version: pin.version,
            to_hash: String::new(),
            brief: None,
            outputs: None,
            secrets: None,
            policy: Vec::new(),
            params_error: None,
        }
    }
}

fn show(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => "—".to_string(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => match n.as_f64() {
            Some(f) if f.fract() == 0.0 && f.abs() < 1e15 => format!("{}", f as i64),
            _ => n.to_string(),
        },
        Some(other) => other.to_string(),
    }
}

/// Equal as settings: `1` and `1.0` are the same figure.
fn same(a: Option<&Value>, b: Option<&Value>) -> bool {
    match (a, b) {
        (Some(Value::Number(x)), Some(Value::Number(y))) => x.as_f64() == y.as_f64(),
        _ => a == b,
    }
}

/// The settings of `after` that differ from `before`'s, caps one by one.
fn policy_changes(before: &Value, after: &Value) -> Vec<PolicyChange> {
    let mut out = Vec::new();
    let mut keys: Vec<&String> = after.as_object().into_iter().flat_map(|m| m.keys()).collect();
    keys.sort();
    for key in keys {
        let (b, a) = (before.get(key), after.get(key));
        if same(b, a) {
            continue;
        }
        if key == "caps" {
            let caps = |v: Option<&Value>| v.and_then(Value::as_object).cloned().unwrap_or_default();
            let (bc, ac) = (caps(b), caps(a));
            let mut names: Vec<&String> = ac.keys().collect();
            names.sort();
            for name in names.into_iter().filter(|n| !same(bc.get(*n), ac.get(*n))) {
                out.push(PolicyChange { field: format!("caps.{name}"), before: show(bc.get(name)), after: show(ac.get(name)) });
            }
            continue;
        }
        out.push(PolicyChange { field: key.clone(), before: show(b), after: show(a) });
    }
    out
}

/// Compare `job`'s pinned template with the one installed now, as a job
/// edit that accepts it would make it.
pub fn update_diff(job: &JobDefinition) -> TemplateUpdateDiff {
    match service() {
        Ok(svc) => update_diff_with(job, svc),
        Err(e) => TemplateUpdateDiff::without_update(job, UpdateStatus::Unavailable, Some(e)),
    }
}

pub(crate) fn update_diff_with(job: &JobDefinition, svc: &JobTemplates) -> TemplateUpdateDiff {
    let Some(pin) = &job.template else { return TemplateUpdateDiff::without_update(job, UpdateStatus::None, None) };
    let Some((info, _)) = svc.get(&pin.id) else {
        let why = format!("{} isn't installed any more", pin.id);
        return TemplateUpdateDiff::without_update(job, UpdateStatus::Unavailable, Some(why));
    };
    let Some(manifest) = info.manifest.clone() else {
        let why = info.error.unwrap_or_else(|| format!("{} isn't a valid template", pin.id));
        return TemplateUpdateDiff::without_update(job, UpdateStatus::Unavailable, Some(why));
    };
    if manifest.version == pin.version && info.hash == pin.hash {
        return TemplateUpdateDiff::without_update(job, UpdateStatus::None, None);
    }
    let mut diff = TemplateUpdateDiff::without_update(job, UpdateStatus::Available, None);
    diff.to_version = manifest.version.clone();
    diff.to_hash = info.hash.clone();
    if info.state != JobTemplateState::Approved {
        diff.status = UpdateStatus::NeedsApproval;
        diff.reason = Some(format!("{} {} hasn't been approved yet: approve it to update this job", pin.id, manifest.version));
        return diff;
    }
    // What accepting does: the same edit `apply` makes.
    let mut after = serde_json::to_value(job).unwrap_or_default();
    after["template"] = json!({ "id": pin.id, "version": manifest.version, "hash": info.hash });
    match apply_with(&mut after, job.template.as_ref(), svc) {
        Err(e) => diff.params_error = Some(e),
        Ok(()) => {
            let new_brief = brief_of(&after);
            let old_brief = brief_of(&serde_json::to_value(job).unwrap_or_default());
            if old_brief != new_brief {
                diff.brief = Some(TextChange { before: old_brief, after: new_brief });
            }
            let list = |v: &Value, key: &str, pick: fn(&Value) -> Option<String>| -> Vec<String> {
                v.get(key).and_then(Value::as_array).into_iter().flatten().filter_map(pick).collect()
            };
            let name = |s: &Value| s.get("name").and_then(Value::as_str).map(str::to_string);
            let plain = |s: &Value| s.as_str().map(str::to_string);
            diff.outputs = ListChange::between(&job.outputs, &list(&after, "outputs", plain));
            let old_secrets: Vec<String> = job.secrets.iter().map(|s| s.name.clone()).collect();
            diff.secrets = ListChange::between(&old_secrets, &list(&after, "secrets", name));
            let old_policy = serde_json::to_value(&job.policy).unwrap_or_default();
            diff.policy = policy_changes(&old_policy, after.get("policy").unwrap_or(&Value::Null));
        }
    }
    diff
}

/// The text an action carries: a task's payload, else the brief.
pub fn brief_of(def: &Value) -> String {
    let action = def.get("action").unwrap_or(&Value::Null);
    let key = if text(action, "kind") == "task" { "payload" } else { "brief" };
    text(action, key).to_string()
}

#[cfg(test)]
#[path = "from_template_tests.rs"]
mod tests;
