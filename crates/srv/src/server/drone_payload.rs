// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `getdronepayloadschema`: what a drone can read of what starts it, for the canvas
//! (Drone real-world jobs spec §3.6). One answer per job that runs the drone, and what a hand run has.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::backend::jobs::definition::{action_kind, trigger_kind};
use crate::backend::storage::cron::CronJob;
use crate::drone::payload_schema::{fields_of, PayloadField};
use crate::server::AppState;

/// What `getdronepayloadschema` is asked.
#[derive(Debug, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct PayloadSchemaReq {
    pub drone_id: String,
}

/// One job that runs the drone, and the fields its trigger gives a run.
#[derive(Debug, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JobPayload {
    pub job_id: String,
    pub job_name: String,
    /// `timer`, or the event source: `app_event`, `file_watch`, `webhook_local`, `poll`, `expect`.
    pub trigger: String,
    pub fields: Vec<PayloadField>,
}

/// The answer: one entry per job, and what a run started by hand (or by an agent) has.
#[derive(Debug, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct PayloadSchemaResp {
    pub jobs: Vec<JobPayload>,
    pub by_hand: Vec<PayloadField>,
}

/// The event source of `job`'s trigger, or `None` for a timer.
fn source_of(job: &CronJob) -> Option<String> {
    if job.trigger_kind != trigger_kind::EVENT {
        return None;
    }
    job.trigger_event.get("source").and_then(Value::as_str).map(str::to_string)
}

fn job_payload(job: &CronJob) -> JobPayload {
    let source = source_of(job);
    let filter: Map<String, Value> = job.trigger_event.get("filter").and_then(Value::as_object).cloned().unwrap_or_default();
    JobPayload {
        job_id: job.id.clone(),
        job_name: job.name.clone(),
        trigger: source.clone().unwrap_or_else(|| "timer".to_string()),
        fields: fields_of(source.as_deref(), &filter),
    }
}

/// `getdronepayloadschema`.
pub(crate) fn get_payload_schema(state: &AppState, cmd: PayloadSchemaReq) -> Result<PayloadSchemaResp, String> {
    let jobs = state.mstore.cron_list().map_err(|e| format!("getdronepayloadschema: {e}"))?;
    let runs_it = |j: &CronJob| j.action.get("kind").and_then(Value::as_str) == Some(action_kind::DRONE) && j.action.get("drone_id").and_then(Value::as_str) == Some(cmd.drone_id.as_str());
    Ok(PayloadSchemaResp { jobs: jobs.iter().filter(|j| runs_it(j)).map(job_payload).collect(), by_hand: fields_of(None, &Map::new()) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn job(trigger_kind: &str, trigger_event: Value) -> CronJob {
        CronJob { id: "j1".into(), name: "watch".into(), trigger_kind: trigger_kind.into(), trigger_event, ..Default::default() }
    }

    #[test]
    fn a_timer_job_has_the_framing_and_what_the_sender_gave() {
        let p = job_payload(&job("cron", Value::Null));
        assert_eq!(p.trigger, "timer");
        assert!(p.fields.iter().any(|f| f.path == "run_id" && f.clean));
    }

    #[test]
    fn an_event_job_has_its_sources_fields() {
        let p = job_payload(&job(trigger_kind::EVENT, json!({ "source": "poll", "filter": { "query": "tasks" } })));
        assert_eq!(p.trigger, "poll");
        assert!(p.fields.iter().any(|f| f.path == "item.title" && !f.clean));
        assert!(p.fields.iter().any(|f| f.path == "item.num" && f.clean));
    }
}
