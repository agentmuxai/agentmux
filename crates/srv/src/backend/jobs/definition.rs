// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The job definition (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md
//! §5.1): a trigger, an action, a policy and an owner, as JSON. Published as
//! `schema/agentmux-job.v1.schema.json`; `definition_tests.rs` pins the two
//! together.
//!
//! [`JobDefinition::parse`] is the strict reading, for definitions a caller
//! sends: unknown fields are refused, and so are the kinds reserved for later
//! phases (§6): `trigger.kind: event`, `action.kind: drone`, and any
//! `policy.approval` but `none`. A stored row is read leniently
//! (`storage::cron`), so a newer build's extra field doesn't lose the row.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::backend::cron::schedule::{JobSchedule, DEFAULT_TZ};
use crate::backend::storage::cron::CronJob;

/// The only `jobVersion` this build reads.
pub const JOB_VERSION: u32 = 1;

/// The shortest interval between two fires of an unattended agent action
/// (§5.1 "Triggers"), checked when a trigger or action is set.
pub const MIN_UNATTENDED_INTERVAL_SECS: i64 = 15 * 60;

/// The shortest `every` interval for any job.
pub const MIN_EVERY_SECS: i64 = 60;

/// What a kind reserved for a later phase answers (§6).
pub const LATER: &str = "available in a later version";

/// Trigger kinds (`trigger.kind`).
pub mod trigger_kind {
    pub const CRON: &str = "cron";
    pub const EVERY: &str = "every";
    pub const ONCE: &str = "once";
    pub const MANUAL: &str = "manual";
    /// Reserved, P5.
    pub const EVENT: &str = "event";
    #[cfg(test)]
    pub const ALL: [&str; 5] = [CRON, EVERY, ONCE, MANUAL, EVENT];
}

/// Action kinds (`action.kind`).
pub mod action_kind {
    pub const MESSAGE: &str = "message";
    pub const TASK: &str = "task";
    pub const SPAWN: &str = "spawn";
    /// Reserved, P4.
    pub const DRONE: &str = "drone";
    #[cfg(test)]
    pub const ALL: [&str; 4] = [MESSAGE, TASK, SPAWN, DRONE];
}

/// `policy.results` values.
pub const RESULTS: [&str; 2] = ["ledger", "task_on_failure"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobDefinition {
    #[serde(rename = "jobVersion")]
    pub job_version: u32,
    #[serde(default)]
    pub id: String,
    pub name: String,
    /// The owning agent's UID.
    #[serde(default)]
    pub owner: String,
    /// An agent UID, or `operator`. Set by srv from the caller, never taken
    /// from the request.
    #[serde(default)]
    pub created_by: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<TemplateRef>,
    #[serde(default)]
    pub params: Map<String, Value>,
    pub trigger: Trigger,
    pub action: Action,
    #[serde(default)]
    pub policy: Policy,
    #[serde(default)]
    pub outputs: Vec<String>,
    #[serde(default)]
    pub secrets: Vec<SecretRef>,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Trigger {
    Cron {
        cron: String,
        #[serde(default = "default_tz")]
        tz: String,
    },
    Every { seconds: i64 },
    /// ms epoch.
    Once { at: i64 },
    Manual,
    /// Reserved, P5: refused by [`JobDefinition::parse`].
    Event(Map<String, Value>),
}

impl Trigger {
    pub fn kind(&self) -> &'static str {
        match self {
            Trigger::Cron { .. } => trigger_kind::CRON,
            Trigger::Every { .. } => trigger_kind::EVERY,
            Trigger::Once { .. } => trigger_kind::ONCE,
            Trigger::Manual => trigger_kind::MANUAL,
            Trigger::Event(_) => trigger_kind::EVENT,
        }
    }
}

fn default_tz() -> String {
    DEFAULT_TZ.to_string()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    /// `to` is an agent UID, or a name for a job created by name.
    Message { to: String, brief: String },
    Task {
        title: String,
        #[serde(default)]
        payload: String,
        #[serde(default)]
        target: String,
    },
    Spawn {
        leader: String,
        brief: String,
        #[serde(default)]
        model: String,
        #[serde(default)]
        account: String,
        #[serde(default)]
        keep_worker: bool,
    },
    /// Reserved, P4: refused by [`JobDefinition::parse`].
    Drone {
        #[serde(default)]
        drone_id: String,
    },
}

impl Action {
    pub fn kind(&self) -> &'static str {
        match self {
            Action::Message { .. } => action_kind::MESSAGE,
            Action::Task { .. } => action_kind::TASK,
            Action::Spawn { .. } => action_kind::SPAWN,
            Action::Drone { .. } => action_kind::DRONE,
        }
    }

    /// Whether a run of it is an agent working unattended: a message starts
    /// a turn, a spawn starts a worker. Filing a task starts nothing.
    pub fn starts_an_agent(&self) -> bool {
        matches!(self, Action::Message { .. } | Action::Spawn { .. })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Overlap {
    #[default]
    Skip,
    BufferOne,
    CancelOther,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatchUp {
    #[default]
    Skip,
    Latest,
}

impl CatchUp {
    pub fn as_str(self) -> &'static str {
        match self {
            CatchUp::Skip => super::CATCH_UP_SKIP,
            CatchUp::Latest => super::CATCH_UP_LATEST,
        }
    }

    pub fn from_column(s: &str) -> Self {
        if s == super::CATCH_UP_LATEST { CatchUp::Latest } else { CatchUp::Skip }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Policy {
    #[serde(default)]
    pub overlap: Overlap,
    #[serde(default)]
    pub catch_up: CatchUp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_minutes: Option<u32>,
    /// Retries for infrastructure failures only, at most 3.
    #[serde(default)]
    pub retries: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caps: Option<Caps>,
    /// `none` only, until P4.
    #[serde(default = "approval_none")]
    pub approval: String,
    #[serde(default = "default_results")]
    pub results: Vec<String>,
}

fn approval_none() -> String {
    "none".to_string()
}

fn default_results() -> Vec<String> {
    RESULTS.iter().map(|s| s.to_string()).collect()
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            overlap: Overlap::default(),
            catch_up: CatchUp::default(),
            timeout_minutes: None,
            retries: 0,
            max_turns: None,
            group: None,
            caps: None,
            approval: approval_none(),
            results: default_results(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Caps {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub day_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub messages_per_day: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateRef {
    pub id: String,
    pub version: String,
    #[serde(default)]
    pub hash: String,
}

/// A secret the job's runs use, by Vault entry: never a value (§7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretRef {
    pub name: String,
    pub vault: String,
}

/// The fields each object may have, for [`JobDefinition::parse`].
const TOP_KEYS: &[&str] = &[
    "jobVersion", "id", "name", "owner", "created_by", "template", "params", "trigger", "action", "policy", "outputs",
    "secrets", "enabled",
];
const POLICY_KEYS: &[&str] =
    &["overlap", "catch_up", "timeout_minutes", "retries", "max_turns", "group", "caps", "approval", "results"];
const CAPS_KEYS: &[&str] = &["run_usd", "day_usd", "messages_per_day"];
const TEMPLATE_KEYS: &[&str] = &["id", "version", "hash"];
const SECRET_KEYS: &[&str] = &["name", "vault"];

fn trigger_keys(kind: &str) -> &'static [&'static str] {
    match kind {
        trigger_kind::CRON => &["kind", "cron", "tz"],
        trigger_kind::EVERY => &["kind", "seconds"],
        trigger_kind::ONCE => &["kind", "at"],
        _ => &["kind"],
    }
}

fn action_keys(kind: &str) -> &'static [&'static str] {
    match kind {
        action_kind::MESSAGE => &["kind", "to", "brief"],
        action_kind::TASK => &["kind", "title", "payload", "target"],
        action_kind::SPAWN => &["kind", "leader", "brief", "model", "account", "keep_worker"],
        _ => &["kind"],
    }
}

/// Refuse a field `obj` may not have, naming it with its place (`at`).
fn known_keys(obj: Option<&Value>, allowed: &[&str], at: &str) -> Result<(), String> {
    let Some(Value::Object(map)) = obj else { return Ok(()) };
    match map.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(k) => Err(format!("unknown field {at}{k}")),
        None => Ok(()),
    }
}

/// How [`JobDefinition::validate`] reads the definition.
#[derive(Debug, Clone, Copy)]
pub struct Check {
    /// The trigger or the action is being set now (a create, or an edit
    /// that changes either): the minimum interval applies and a `once` time
    /// must be ahead. A job kept as it was, a migrated one-minute job
    /// included, isn't refused for them.
    pub fresh: bool,
    pub now_ms: i64,
}

impl JobDefinition {
    /// Read a definition a caller sent: refuse unknown fields and the kinds
    /// reserved for later, then [`Self::validate`] it.
    pub fn parse(value: &Value, check: Check) -> Result<JobDefinition, String> {
        if !value.is_object() {
            return Err("a job definition is a JSON object".to_string());
        }
        known_keys(Some(value), TOP_KEYS, "")?;
        let kind = |field: &str| value.get(field).and_then(|v| v.get("kind")).and_then(Value::as_str).unwrap_or("");
        let (trigger, action) = (kind("trigger"), kind("action"));
        if trigger == trigger_kind::EVENT {
            return Err(format!("trigger kind event is {LATER}"));
        }
        if action == action_kind::DRONE {
            return Err(format!("action kind drone is {LATER}"));
        }
        known_keys(value.get("trigger"), trigger_keys(trigger), "trigger.")?;
        known_keys(value.get("action"), action_keys(action), "action.")?;
        known_keys(value.get("policy"), POLICY_KEYS, "policy.")?;
        known_keys(value.get("policy").and_then(|p| p.get("caps")), CAPS_KEYS, "policy.caps.")?;
        known_keys(value.get("template"), TEMPLATE_KEYS, "template.")?;
        for s in value.get("secrets").and_then(Value::as_array).into_iter().flatten() {
            known_keys(Some(s), SECRET_KEYS, "secrets[].")?;
        }
        let def: JobDefinition = serde_json::from_value(value.clone()).map_err(|e| format!("invalid job definition: {e}"))?;
        def.validate(check)?;
        Ok(def)
    }

    /// Check what the types can't: values, bounds and the reserved kinds.
    pub fn validate(&self, check: Check) -> Result<(), String> {
        if self.job_version != JOB_VERSION {
            return Err(format!("jobVersion {} isn't supported (use {JOB_VERSION})", self.job_version));
        }
        let name = self.name.trim();
        if name.is_empty() || name.chars().count() > 200 {
            return Err("name is required, at most 200 characters".to_string());
        }
        if let Some(t) = &self.template {
            if t.id.trim().is_empty() || t.version.trim().is_empty() {
                return Err("template: id and version are required".to_string());
            }
        }
        if serde_json::to_string(&self.params).map_or(0, |s| s.len()) > 16 * 1024 {
            return Err("params are at most 16 KB".to_string());
        }
        self.validate_trigger(check)?;
        self.validate_action()?;
        self.policy.validate()?;
        validate_outputs(&self.outputs)?;
        validate_secrets(&self.secrets)
    }

    fn validate_trigger(&self, check: Check) -> Result<(), String> {
        let interval = match &self.trigger {
            Trigger::Cron { cron, tz } => {
                let schedule = JobSchedule::cron(cron, tz).map_err(|e| format!("trigger: invalid cron '{cron}': {e}"))?;
                schedule.min_gap_secs(check.now_ms, 500)
            }
            &Trigger::Every { seconds } => {
                if seconds < MIN_EVERY_SECS {
                    return Err(format!("trigger: every runs at most once a minute (seconds >= {MIN_EVERY_SECS})"));
                }
                Some(seconds)
            }
            &Trigger::Once { at } => {
                if at <= 0 || (check.fresh && at <= check.now_ms) {
                    return Err("trigger: once needs a time ahead, `at` in ms epoch".to_string());
                }
                None
            }
            Trigger::Manual => None,
            Trigger::Event(_) => return Err(format!("trigger kind event is {LATER}")),
        };
        match interval {
            Some(secs) if check.fresh && self.action.starts_an_agent() && secs < MIN_UNATTENDED_INTERVAL_SECS => Err(format!(
                "trigger: an unattended agent job runs at most every {} minutes; this one would run {} apart",
                MIN_UNATTENDED_INTERVAL_SECS / 60,
                span(secs)
            )),
            _ => Ok(()),
        }
    }

    fn validate_action(&self) -> Result<(), String> {
        let blank = |s: &str| s.trim().is_empty();
        match &self.action {
            Action::Message { to, brief } if blank(to) || blank(brief) => Err("action: message needs to and brief".to_string()),
            Action::Task { title, .. } if blank(title) => Err("action: task needs a title".to_string()),
            Action::Spawn { leader, brief, .. } if blank(leader) || blank(brief) => {
                Err("action: spawn needs leader and brief".to_string())
            }
            Action::Drone { .. } => Err(format!("action kind drone is {LATER}")),
            _ => Ok(()),
        }
    }

    /// The definition of a stored job.
    pub fn from_job(job: &CronJob) -> JobDefinition {
        let trigger = match job.trigger_kind.as_str() {
            trigger_kind::EVERY => Trigger::Every { seconds: job.every_secs.unwrap_or(0) },
            trigger_kind::ONCE => Trigger::Once { at: job.once_at_ms.unwrap_or(0) },
            trigger_kind::MANUAL => Trigger::Manual,
            _ => Trigger::Cron { cron: job.expression.clone(), tz: if job.tz.is_empty() { default_tz() } else { job.tz.clone() } },
        };
        let message = || Action::Message {
            to: if job.target_uid.is_empty() { job.target.clone() } else { job.target_uid.clone() },
            brief: job.prompt.clone(),
        };
        let action = match job.action_kind.as_str() {
            "" | action_kind::MESSAGE => message(),
            _ => serde_json::from_value(job.action.clone()).unwrap_or_else(|_| message()),
        };
        let mut policy = job.policy.clone();
        policy.catch_up = CatchUp::from_column(&job.catch_up);
        JobDefinition {
            job_version: JOB_VERSION,
            id: job.id.clone(),
            name: job.name.clone(),
            owner: job.owner_uid().to_string(),
            created_by: if job.created_by_uid.is_empty() { job.created_by.clone() } else { job.created_by_uid.clone() },
            template: job.template.clone(),
            params: job.params.clone(),
            trigger,
            action,
            policy,
            outputs: job.outputs.clone(),
            secrets: job.secrets.clone(),
            enabled: job.enabled,
        }
    }

    /// Write this definition into `job`, leaving its id, creator, fires and
    /// misses alone. A message's `to` becomes the job's target through
    /// `target_of`: `(name, uid)`.
    pub fn apply_to(&self, job: &mut CronJob, target_of: &dyn Fn(&str) -> (String, String)) {
        job.name = self.name.trim().to_string();
        job.owner = self.owner.clone();
        job.template = self.template.clone();
        job.params = self.params.clone();
        job.enabled = self.enabled;
        (job.expression, job.tz, job.every_secs, job.once_at_ms) = (String::new(), String::new(), None, None);
        job.trigger_kind = match &self.trigger {
            Trigger::Cron { cron, tz } => {
                (job.expression, job.tz) = (cron.trim().to_string(), if tz.trim().is_empty() { default_tz() } else { tz.trim().to_string() });
                trigger_kind::CRON
            }
            &Trigger::Every { seconds } => {
                job.every_secs = Some(seconds);
                trigger_kind::EVERY
            }
            &Trigger::Once { at } => {
                job.once_at_ms = Some(at);
                trigger_kind::ONCE
            }
            Trigger::Manual | Trigger::Event(_) => trigger_kind::MANUAL,
        }
        .to_string();
        job.action_kind = self.action.kind().to_string();
        match &self.action {
            Action::Message { to, brief } => {
                (job.target, job.target_uid) = target_of(to.trim());
                job.prompt = brief.clone();
                job.action = Value::Null;
            }
            other => {
                (job.target, job.target_uid, job.prompt) = (String::new(), String::new(), String::new());
                job.action = serde_json::to_value(other).unwrap_or(Value::Null);
            }
        }
        job.catch_up = self.policy.catch_up.as_str().to_string();
        job.policy = self.policy.clone();
        job.outputs = self.outputs.clone();
        job.secrets = self.secrets.clone();
    }
}

impl Policy {
    fn validate(&self) -> Result<(), String> {
        if self.approval != "none" {
            return Err(format!("policy.approval: only none for now; approvals are {LATER}"));
        }
        if self.retries > 3 {
            return Err("policy.retries: at most 3".to_string());
        }
        if self.timeout_minutes.is_some_and(|t| !(1..=1440).contains(&t)) {
            return Err("policy.timeout_minutes: 1 to 1440".to_string());
        }
        if self.max_turns.is_some_and(|t| !(1..=10_000).contains(&t)) {
            return Err("policy.max_turns: 1 to 10000".to_string());
        }
        if let Some(group) = &self.group {
            let ok = (1..=40).contains(&group.len())
                && group.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                && !group.starts_with('-');
            if !ok {
                return Err("policy.group: lowercase letters, digits and -, at most 40".to_string());
            }
        }
        if let Some(caps) = &self.caps {
            if [caps.run_usd, caps.day_usd].into_iter().flatten().any(|c| !c.is_finite() || c < 0.0) {
                return Err("policy.caps: dollar figures are 0 or more".to_string());
            }
        }
        match self.results.iter().find(|r| !RESULTS.contains(&r.as_str())) {
            Some(r) => Err(format!("policy.results: unknown '{r}' (use {})", RESULTS.join(", "))),
            None => Ok(()),
        }
    }
}

/// `report_file`, `kanban_task`, `message:owner`, `pull_request`…: a word,
/// and an optional `:qualifier`. Declared, not enforced, until P4.
fn validate_outputs(outputs: &[String]) -> Result<(), String> {
    if outputs.len() > 20 {
        return Err("outputs: at most 20".to_string());
    }
    let word = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_lowercase() || b == b'_');
    let qualifier = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b));
    for o in outputs {
        let ok = match o.split_once(':') {
            Some((w, q)) => word(w) && qualifier(q),
            None => word(o),
        };
        if !ok {
            return Err(format!("outputs: '{o}' isn't an output name (report_file, kanban_task, message:owner, …)"));
        }
    }
    Ok(())
}

/// Names an environment variable can have, never `AGENTMUX_…` (srv's own,
/// `shell_secrets.rs` refuses them too), each once, each with its Vault entry.
fn validate_secrets(secrets: &[SecretRef]) -> Result<(), String> {
    if secrets.len() > 20 {
        return Err("secrets: at most 20".to_string());
    }
    let mut seen = std::collections::BTreeSet::new();
    for s in secrets {
        let ok = (1..=64).contains(&s.name.len())
            && s.name.starts_with(|c: char| c.is_ascii_uppercase())
            && s.name.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_');
        if !ok || s.name.starts_with("AGENTMUX_") {
            return Err(format!("secrets: '{}' isn't a usable name (A-Z, 0-9 and _, not AGENTMUX_…)", s.name));
        }
        if s.vault.trim().is_empty() {
            return Err(format!("secrets: '{}' needs its Vault entry id", s.name));
        }
        if !seen.insert(s.name.as_str()) {
            return Err(format!("secrets: '{}' is listed twice", s.name));
        }
    }
    Ok(())
}

/// A span of seconds in words: "5 min", "40 s".
fn span(secs: i64) -> String {
    if secs >= 60 { format!("{} min", secs / 60) } else { format!("{secs} s") }
}

#[cfg(test)]
#[path = "definition_tests.rs"]
mod tests;
