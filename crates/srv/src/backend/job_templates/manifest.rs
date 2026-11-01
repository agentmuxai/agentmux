// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `template.json` and `brief.md`: what a job template declares, its checks,
//! the Rule of Two, and the brief it renders from parameters
//! (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §5.4, §6).
//!
//! The trigger, action and policy are defaults for a job made from the
//! template. Only what this module needs is checked here (the trigger's and
//! the action's kind, the policy's `approval`); the job definition checks the
//! rest when a job is created.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::cloud::{CloudExpect, CloudReporter, CloudRunNow};
use crate::backend::widget_packages as wp;

pub const TEMPLATE_VERSION: u32 = 1;
const MAX_PARAMS: usize = 32;
const MAX_STRING_CHARS: usize = 4000;
const MAX_LIST_ITEMS: usize = 100;
const MAX_SECRETS: usize = 16;

/// The trigger kinds a job has (§5.1). `event` is accepted in a template, so
/// the Rule of Two can see it; a job can't use it before P5.
const TRIGGER_KINDS: &[&str] = &["cron", "every", "once", "manual", "event"];
const ACTION_KINDS: &[&str] = &["message", "task", "spawn"];

/// Outputs that stay inside AgentMux or with the job's owner.
const INSIDE_OUTPUTS: &[&str] = &["report_file", "kanban_task", "message:owner"];
/// Outputs that write outside: to a repository, another agent, or a deployment.
pub const OUTSIDE_OUTPUTS: &[&str] = &["pull_request", "comment", "message:others", "deploy"];

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ts_rs::TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub enum JobTemplateKind {
    Job,
    Cloud,
}

/// One parameter, as a JSON Schema subset: `string` (with `enum`, or
/// `format: "credential"` for a Vault entry reference), `number`, `boolean`,
/// and `array` of strings.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JobTemplateParam {
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "unknown")]
    pub default: Option<Value>,
    #[serde(default, rename = "enum", skip_serializing_if = "Option::is_none")]
    pub enum_values: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<JobTemplateParamItems>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JobTemplateParamItems {
    #[serde(rename = "type")]
    pub ty: String,
}

/// `params`: a JSON Schema object of parameters.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JobTemplateParams {
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub ty: Option<String>,
    #[serde(default)]
    pub properties: BTreeMap<String, JobTemplateParam>,
    #[serde(default)]
    pub required: Vec<String>,
}

/// A secret the job's agent uses through Vault: an environment name and
/// the `credential` parameter naming the Vault entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JobTemplateSecret {
    pub name: String,
    pub param: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JobTemplateManifest {
    pub template_version: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    pub kind: JobTemplateKind,
    pub description: String,
    #[serde(default)]
    pub params: JobTemplateParams,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "Record<string, unknown> | null")]
    pub trigger: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "Record<string, unknown> | null")]
    pub action: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "Record<string, unknown> | null")]
    pub policy: Option<Value>,
    #[serde(default)]
    pub outputs: Vec<String>,
    #[serde(default)]
    pub secrets: Vec<JobTemplateSecret>,
    /// The job reads text nobody vouches for (a pull request's, an inbound
    /// message's), whatever its trigger.
    #[serde(default)]
    pub reads_untrusted: bool,
    /// A cloud template's fields (§4.3); a job template carries none of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reporter: Option<CloudReporter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expect: Option<CloudExpect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_duration_minutes: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_now: Option<CloudRunNow>,
    /// The job reports a summary (runs per day, failures) instead of each run.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub aggregate: bool,
}

/// A parameter's type, from its schema.
#[derive(Debug, Clone, PartialEq)]
pub enum ParamKind {
    String,
    Number,
    Boolean,
    Enum(Vec<String>),
    StringList,
    Credential,
}

fn valid_param_name(n: &str) -> bool {
    n.len() <= 40 && n.starts_with(|c: char| c.is_ascii_lowercase()) && n.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

pub fn param_kind(name: &str, p: &JobTemplateParam) -> Result<ParamKind, String> {
    let kind = match (p.ty.as_str(), &p.enum_values, p.format.as_deref(), &p.items) {
        ("string", None, None, None) => ParamKind::String,
        ("string", Some(values), None, None) => {
            if values.is_empty() || values.len() > 50 || values.iter().collect::<HashSet<_>>().len() != values.len() {
                return Err(format!("parameter {name:?}: enum needs 1–50 different values"));
            }
            ParamKind::Enum(values.clone())
        }
        ("string", None, Some("credential"), None) => ParamKind::Credential,
        ("number", None, None, None) => ParamKind::Number,
        ("boolean", None, None, None) => ParamKind::Boolean,
        ("array", None, None, Some(items)) if items.ty == "string" => ParamKind::StringList,
        _ => {
            return Err(format!(
                "parameter {name:?} must be a string (with enum, or format \"credential\"), a number, a boolean, or an array of strings"
            ))
        }
    };
    Ok(kind)
}

/// `value` checked as a value of `kind`.
fn check_value(name: &str, kind: &ParamKind, value: &Value) -> Result<(), String> {
    let ok = match (kind, value) {
        (ParamKind::String, Value::String(s)) => s.chars().count() <= MAX_STRING_CHARS,
        (ParamKind::Number, Value::Number(_)) => true,
        (ParamKind::Boolean, Value::Bool(_)) => true,
        (ParamKind::Enum(values), Value::String(s)) => values.contains(s),
        (ParamKind::StringList, Value::Array(items)) => {
            items.len() <= MAX_LIST_ITEMS && items.iter().all(|i| i.as_str().is_some_and(|s| s.chars().count() <= MAX_STRING_CHARS))
        }
        // A Vault entry's id: one short token, never a value.
        (ParamKind::Credential, Value::String(s)) => {
            !s.is_empty() && s.len() <= 200 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b':')
        }
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err(format!("parameter {name:?} isn't a valid {}", kind_name(kind)))
    }
}

fn kind_name(kind: &ParamKind) -> &'static str {
    match kind {
        ParamKind::String => "string",
        ParamKind::Number => "number",
        ParamKind::Boolean => "boolean",
        ParamKind::Enum(_) => "choice",
        ParamKind::StringList => "list of strings",
        ParamKind::Credential => "Vault entry reference",
    }
}

/// A `kind` field of a JSON object, if it is one.
fn kind_of(v: &Value) -> Option<&str> {
    v.get("kind").and_then(Value::as_str)
}

impl JobTemplateManifest {
    /// Whether a run reads untrusted input: declared, or an event trigger.
    pub fn reads_untrusted_input(&self) -> bool {
        self.reads_untrusted || self.trigger.as_ref().and_then(kind_of) == Some("event")
    }

    /// Whether a run holds secrets: a declared secret or a credential parameter.
    pub fn holds_secrets(&self) -> bool {
        !self.secrets.is_empty() || self.params.properties.iter().any(|(n, p)| param_kind(n, p) == Ok(ParamKind::Credential))
    }

    pub fn outside_writes(&self) -> Vec<&str> {
        self.outputs.iter().map(String::as_str).filter(|o| OUTSIDE_OUTPUTS.contains(o)).collect()
    }

    /// `policy.approval` is `required`.
    pub fn requires_approval(&self) -> bool {
        self.policy.as_ref().and_then(|p| p.get("approval")).and_then(Value::as_str) == Some("required")
    }
}

/// The Rule of Two (§6): a run that reads untrusted input can't also hold
/// secrets and write outside, unless each run waits for the operator's
/// approval.
pub fn rule_of_two(m: &JobTemplateManifest) -> Result<(), String> {
    let outside = m.outside_writes();
    if m.reads_untrusted_input() && m.holds_secrets() && !outside.is_empty() && !m.requires_approval() {
        return Err(format!(
            "it reads untrusted input, holds secrets and writes outside ({}): one of the three must go, or its policy must say \"approval\": \"required\"",
            outside.join(", ")
        ));
    }
    Ok(())
}

/// Check a manifest and its brief; the first problem, in words the author
/// can act on. `folder` is the package's folder name.
pub fn validate(m: &JobTemplateManifest, folder: &str, brief: Option<&str>) -> Result<(), String> {
    if m.template_version != TEMPLATE_VERSION {
        return Err(format!("templateVersion {} isn't one this AgentMux reads (it reads {TEMPLATE_VERSION})", m.template_version));
    }
    if !wp::valid_id(&m.id) {
        return Err(format!("id {:?} must look like publisher.name (lowercase letters, digits, -)", m.id));
    }
    if m.id != folder {
        return Err(format!("id {:?} must match its folder name {:?}", m.id, folder));
    }
    if m.name.trim().is_empty() || m.name.chars().count() > 60 {
        return Err("name must be 1–60 characters".to_string());
    }
    if !wp::valid_semver(&m.version) {
        return Err(format!("version {:?} isn't semver (like 1.2.0)", m.version));
    }
    if m.description.trim().is_empty() || m.description.chars().count() > 500 {
        return Err("description must be 1–500 characters".to_string());
    }
    let kinds = validate_params(m)?;
    validate_defaults(m)?;
    validate_outputs_and_secrets(m, &kinds)?;
    super::cloud::validate(m)?;
    match (m.kind, brief) {
        (JobTemplateKind::Job, None) => return Err("a job template needs its brief in brief.md".to_string()),
        (_, Some(b)) => {
            for name in placeholders(b)? {
                if !kinds.contains_key(name) {
                    return Err(format!("brief.md uses {{{{{name}}}}}, which isn't one of its parameters"));
                }
            }
        }
        (JobTemplateKind::Cloud, None) => {}
    }
    rule_of_two(m)
}

fn validate_params(m: &JobTemplateManifest) -> Result<BTreeMap<String, ParamKind>, String> {
    let p = &m.params;
    if p.ty.as_deref().is_some_and(|t| t != "object") {
        return Err("params must be a JSON Schema object".to_string());
    }
    if p.properties.len() > MAX_PARAMS {
        return Err(format!("a template takes at most {MAX_PARAMS} parameters"));
    }
    let mut kinds = BTreeMap::new();
    for (name, param) in &p.properties {
        if !valid_param_name(name) {
            return Err(format!("parameter name {name:?} must be lowercase letters, digits and _, starting with a letter"));
        }
        let kind = param_kind(name, param)?;
        if param.title.as_deref().is_some_and(|t| t.chars().count() > 80) || param.description.as_deref().is_some_and(|d| d.chars().count() > 500) {
            return Err(format!("parameter {name:?}: title is at most 80 characters, description 500"));
        }
        if let Some(d) = &param.default {
            if kind == ParamKind::Credential {
                return Err(format!("parameter {name:?} names a Vault entry, which has no default"));
            }
            check_value(name, &kind, d).map_err(|e| format!("{e} as its default"))?;
        }
        kinds.insert(name.clone(), kind);
    }
    if let Some(r) = p.required.iter().find(|r| !kinds.contains_key(r.as_str())) {
        return Err(format!("params.required names {r:?}, which isn't a parameter"));
    }
    Ok(kinds)
}

fn validate_defaults(m: &JobTemplateManifest) -> Result<(), String> {
    if let Some(t) = &m.trigger {
        match kind_of(t) {
            Some(k) if TRIGGER_KINDS.contains(&k) => {}
            _ => return Err(format!("trigger.kind must be one of {}", TRIGGER_KINDS.join(", "))),
        }
    }
    match (m.kind, &m.action) {
        (JobTemplateKind::Job, None) => return Err("a job template needs an action".to_string()),
        (JobTemplateKind::Cloud, Some(_)) => return Err("a cloud template runs elsewhere and has no action".to_string()),
        (_, Some(a)) => match kind_of(a) {
            Some("drone") => return Err("a Drone action needs a later version of AgentMux".to_string()),
            Some(k) if ACTION_KINDS.contains(&k) => {
                if a.get("brief").is_some() {
                    return Err("the action's brief goes in brief.md, not in template.json".to_string());
                }
            }
            _ => return Err(format!("action.kind must be one of {}", ACTION_KINDS.join(", "))),
        },
        (JobTemplateKind::Cloud, None) => {}
    }
    if let Some(p) = &m.policy {
        if !p.is_object() {
            return Err("policy must be an object".to_string());
        }
        match p.get("approval") {
            None => {}
            Some(Value::String(a)) if a == "none" || a == "required" => {}
            Some(_) => return Err("policy.approval must be \"none\" or \"required\"".to_string()),
        }
    }
    Ok(())
}

fn validate_outputs_and_secrets(m: &JobTemplateManifest, kinds: &BTreeMap<String, ParamKind>) -> Result<(), String> {
    let mut seen = HashSet::new();
    for o in &m.outputs {
        if !INSIDE_OUTPUTS.contains(&o.as_str()) && !OUTSIDE_OUTPUTS.contains(&o.as_str()) {
            return Err(format!(
                "unknown output {o:?} (outputs are {}, {})",
                INSIDE_OUTPUTS.join(", "),
                OUTSIDE_OUTPUTS.join(", ")
            ));
        }
        if !seen.insert(o.as_str()) {
            return Err(format!("output {o:?} is listed twice"));
        }
    }
    if m.secrets.len() > MAX_SECRETS {
        return Err(format!("a template declares at most {MAX_SECRETS} secrets"));
    }
    let mut names = HashSet::new();
    for s in &m.secrets {
        let well_formed = s.name.len() <= 64
            && s.name.starts_with(|c: char| c.is_ascii_uppercase())
            && s.name.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_');
        if !well_formed || s.name.starts_with("AGENTMUX_") {
            return Err(format!("secret name {:?} must be uppercase letters, digits and _, and not start with AGENTMUX_", s.name));
        }
        if !names.insert(s.name.as_str()) {
            return Err(format!("secret {:?} is declared twice", s.name));
        }
        if kinds.get(&s.param) != Some(&ParamKind::Credential) {
            return Err(format!("secret {:?} must name a parameter with format \"credential\"", s.name));
        }
    }
    Ok(())
}

// ── The brief ───────────────────────────────────────────────────────────────

enum Segment<'a> {
    Text(&'a str),
    Param(&'a str),
}

/// `brief` split into text and `{{name}}` placeholders. Any other `{{` is an
/// error, so a typo can't pass as text.
fn segments(brief: &str) -> Result<Vec<Segment<'_>>, String> {
    let mut out = Vec::new();
    let mut rest = brief;
    while let Some(start) = rest.find("{{") {
        out.push(Segment::Text(&rest[..start]));
        let after = &rest[start + 2..];
        let name = after.find("}}").map(|end| &after[..end]).filter(|n| valid_param_name(n));
        let Some(name) = name else {
            let line = brief[..brief.len() - rest.len() + start].matches('\n').count() + 1;
            return Err(format!("brief.md line {line}: {{{{ must open a placeholder like {{{{repo}}}}"));
        };
        out.push(Segment::Param(name));
        rest = &after[name.len() + 2..];
    }
    out.push(Segment::Text(rest));
    Ok(out)
}

/// The parameter names `brief` uses.
pub fn placeholders(brief: &str) -> Result<Vec<&str>, String> {
    Ok(segments(brief)?
        .into_iter()
        .filter_map(|s| match s {
            Segment::Param(n) => Some(n),
            Segment::Text(_) => None,
        })
        .collect())
}

/// A brief rendered from parameters, and the parameters it used (the given
/// ones and the defaults).
#[derive(Debug, Clone, PartialEq)]
pub struct RenderedBrief {
    pub brief: String,
    pub params: Map<String, Value>,
}

/// The values for `m`'s parameters: each given one checked, defaults for
/// the rest. An unknown or missing parameter is an error.
pub fn resolve_params(m: &JobTemplateManifest, given: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    if let Some(unknown) = given.keys().find(|k| !m.params.properties.contains_key(k.as_str())) {
        return Err(format!("{} takes no parameter {unknown:?}", m.id));
    }
    let mut out = Map::new();
    for (name, p) in &m.params.properties {
        let kind = param_kind(name, p)?;
        match given.get(name).or(p.default.as_ref()) {
            Some(v) => {
                check_value(name, &kind, v)?;
                out.insert(name.clone(), v.clone());
            }
            None if m.params.required.contains(name) => return Err(format!("parameter {name:?} is required")),
            None => {}
        }
    }
    Ok(out)
}

/// A value as it reads in a brief.
fn as_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(items) => items.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "),
        other => other.to_string(),
    }
}

/// `brief` with each placeholder replaced by its parameter's value, in one
/// pass: a value is inserted as data and never read for placeholders.
pub fn render(m: &JobTemplateManifest, brief: &str, given: &Map<String, Value>) -> Result<RenderedBrief, String> {
    let params = resolve_params(m, given)?;
    let mut out = String::with_capacity(brief.len());
    for seg in segments(brief)? {
        match seg {
            Segment::Text(t) => out.push_str(t),
            Segment::Param(name) => {
                if !m.params.properties.contains_key(name) {
                    return Err(format!("brief.md uses {{{{{name}}}}}, which isn't one of its parameters"));
                }
                out.push_str(&params.get(name).map(as_text).unwrap_or_default());
            }
        }
    }
    Ok(RenderedBrief { brief: out, params })
}
