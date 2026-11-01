// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a cloud job's manifest declares beyond a template's own fields
//! (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §4.3): who owns it, which reporter's events count, the schedule
//! it is expected on, how long a run may take, and how to start it.
//!
//! These fields belong to a template whose `kind` is `cloud`, and a job template may not carry them. AgentMux holds
//! no cloud or CI credentials, so `runNow` only names a page to open or an instruction for the owner agent; it never
//! starts anything itself.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::manifest::{JobTemplateKind, JobTemplateManifest};
use super::JobTemplates;
use crate::backend::cron::schedule::JobSchedule;
use crate::backend::jobs::cloud_events::{CloudJob, CloudJobs};
use crate::backend::jobs::reporters;

/// The longest a run may take before it is `lost`: a week.
const MAX_DURATION_MINUTES: u32 = 7 * 24 * 60;
/// The longest grace after an expected slot: a day.
const MAX_GRACE_MINUTES: u32 = 24 * 60;
const MAX_TEXT_CHARS: usize = 500;

/// The reporter whose events count for the job: its name in Settings and its public key. Approving the manifest pins
/// the pair.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CloudReporter {
    pub name: String,
    /// An Ed25519 public key, base64.
    pub key: String,
}

/// A schedule the job is expected to run on.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ts_rs::TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CloudSchedule {
    pub cron: String,
    pub tz: String,
    /// Minutes after a slot before a missing report counts as missed.
    pub grace_minutes: u32,
}

/// The one word `expect` takes when no schedule applies.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub enum NoSchedule {
    #[serde(rename = "none")]
    None,
}

/// `expect`: a schedule, or `"none"` for a job that events start.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ts_rs::TS)]
#[serde(untagged)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub enum CloudExpect {
    Never(NoSchedule),
    Schedule(CloudSchedule),
}

/// `runNow`: one of a page that starts the job, or an instruction for the owner agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ts_rs::TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CloudRunNow {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask_owner: Option<String>,
}

/// Whether the manifest carries any cloud field.
fn declares_cloud_fields(m: &JobTemplateManifest) -> bool {
    m.owner.is_some() || m.reporter.is_some() || m.expect.is_some() || m.max_duration_minutes.is_some() || m.run_now.is_some() || m.aggregate
}

fn text_ok(s: &str) -> bool {
    !s.trim().is_empty() && s.chars().count() <= MAX_TEXT_CHARS && !s.chars().any(char::is_control)
}

/// Check the cloud fields: present and well-formed on a cloud template, absent on a job template.
pub fn validate(m: &JobTemplateManifest) -> Result<(), String> {
    if m.kind == JobTemplateKind::Job {
        return if declares_cloud_fields(m) {
            Err("owner, reporter, expect, maxDurationMinutes, runNow and aggregate belong to a cloud template".to_string())
        } else {
            Ok(())
        };
    }
    let owner = m.owner.as_deref().ok_or("a cloud template names its owner, an agent on this computer")?;
    if owner.trim().is_empty() || owner.chars().count() > 64 || owner.chars().any(char::is_control) {
        return Err("owner must be an agent's name or id, up to 64 characters".to_string());
    }
    let reporter = m.reporter.as_ref().ok_or("a cloud template names its reporter: {\"name\": …, \"key\": …}")?;
    if !reporters::is_name(&reporter.name) {
        return Err("reporter.name is lowercase letters, digits and -, up to 40".to_string());
    }
    if reporter.key != reporter.key.trim() || !reporters::is_public_key(&reporter.key) {
        return Err("reporter.key isn't a 32-byte Ed25519 public key in base64".to_string());
    }
    match m.expect.as_ref().ok_or("a cloud template says what to expect: a schedule, or \"none\" for a job events start")? {
        CloudExpect::Never(_) => {}
        CloudExpect::Schedule(s) => {
            JobSchedule::cron(&s.cron, &s.tz).map_err(|e| format!("expect: invalid cron {:?} in {:?}: {e}", s.cron, s.tz))?;
            if s.grace_minutes > MAX_GRACE_MINUTES {
                return Err(format!("expect.graceMinutes is at most {MAX_GRACE_MINUTES}"));
            }
        }
    }
    match m.max_duration_minutes {
        Some(n) if (1..=MAX_DURATION_MINUTES).contains(&n) => {}
        Some(_) => return Err(format!("maxDurationMinutes is 1 to {MAX_DURATION_MINUTES}")),
        None => return Err("a cloud template says how long a run may take (maxDurationMinutes)".to_string()),
    }
    if let Some(run_now) = &m.run_now {
        match (run_now.link.as_deref(), run_now.ask_owner.as_deref()) {
            (Some(link), None) => {
                let https = link.strip_prefix("https://").is_some_and(|rest| !rest.is_empty() && !rest.contains(char::is_whitespace));
                if !https || !text_ok(link) {
                    return Err("runNow.link is an https address".to_string());
                }
            }
            (None, Some(ask)) if text_ok(ask) => {}
            (None, Some(_)) => return Err(format!("runNow.askOwner is 1 to {MAX_TEXT_CHARS} characters of text")),
            _ => return Err("runNow is one of link or askOwner".to_string()),
        }
    }
    Ok(())
}

/// The cloud jobs the operator approved: a cloud template, approved and unchanged since, is the job its id names.
/// Anything else is "unknown job".
impl CloudJobs for Arc<JobTemplates> {
    fn cloud_job(&self, job_id: &str) -> Option<CloudJob> {
        let t = self.approved_cloud(job_id)?;
        let reporter_key = t.manifest.reporter?.key;
        Some(CloudJob { id: t.id, name: t.manifest.name, reporter_key })
    }
}
