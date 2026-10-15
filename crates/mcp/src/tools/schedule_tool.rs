// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Schedule: one tool for prompts injected on a schedule, with an `action`
//! (docs/specs/SPEC_MUXAPI_TOOL_CONSOLIDATION_2026_10_15.md §4.2, slice S3). It
//! replaces the five Cron* tools and the three Loop* tools: `create` with
//! `cron` is a persistent job (cron.rs), with `every` a loop that lives as long
//! as this agent's session (loops.rs). Their handlers run unchanged.

use super::*;

pub(super) const ACTIONS: &[&str] = &["create", "list", "pause", "resume", "delete"];

/// Loops are named `loop-<n>` (loops.rs); everything else is a cron job id.
fn is_loop_id(id: &str) -> bool {
    id.starts_with("loop-")
}

/// The handler for an action, and its arguments as that handler takes them.
pub(super) fn route(arguments: &Value) -> Result<(&'static str, Value)> {
    let action = arguments.get("action").and_then(|v| v.as_str()).unwrap_or("");
    let mut args = match arguments {
        Value::Object(m) => m.clone(),
        _ => serde_json::Map::new(),
    };
    args.remove("action");
    let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let rename = |args: &mut serde_json::Map<String, Value>, ours: &str, theirs: &str| {
        if let Some(v) = args.remove(ours) {
            args.insert(theirs.to_string(), v);
        }
    };
    let tool = match action {
        "create" => match (args.contains_key("cron"), args.contains_key("every")) {
            (true, false) => {
                rename(&mut args, "cron", "expression");
                "CronCreate"
            }
            (false, true) => {
                rename(&mut args, "every", "interval");
                "Loop"
            }
            _ => anyhow::bail!(
                "Schedule create needs exactly one of cron (a persistent job on a UTC cron schedule) or every \
                 (a loop for as long as this session lasts)"
            ),
        },
        "list" => "list",
        "pause" | "resume" if is_loop_id(&id) => {
            anyhow::bail!("Schedule: a loop can't be paused; delete it and create it again")
        }
        "pause" => "CronPause",
        "resume" => "CronResume",
        "delete" if is_loop_id(&id) => {
            rename(&mut args, "id", "loop_id");
            "LoopStop"
        }
        "delete" => "CronDelete",
        "" => anyhow::bail!("Schedule: missing required parameter: action (one of {})", ACTIONS.join(", ")),
        other => anyhow::bail!("Schedule: unknown action {other:?}: use one of {}", ACTIONS.join(", ")),
    };
    Ok((tool, Value::Object(args)))
}

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    match name {
        "Schedule" => {
            let (tool, args) = route(arguments)?;
            match tool {
                // Both kinds in one answer: cron jobs, then this session's loops.
                "list" => {
                    let jobs = cron::call("CronList", &args, cx).await?;
                    let loops = loops::call("LoopList", &args, cx).await?;
                    Ok(format!("cron jobs:\n{jobs}\n\nloops (this session):\n{loops}"))
                }
                "Loop" | "LoopStop" => loops::call(tool, &args, cx).await,
                _ => cron::call(tool, &args, cx).await,
            }
        }
        _ => Err(not_in_family()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_is_a_cron_job_or_a_loop() {
        let (tool, args) = route(&json!({ "action": "create", "cron": "0 9 * * 1-5", "name": "n", "prompt": "p", "to": "a" })).unwrap();
        assert_eq!(tool, "CronCreate");
        assert_eq!(args["expression"], "0 9 * * 1-5");
        let (tool, args) = route(&json!({ "action": "create", "every": "5m", "prompt": "p" })).unwrap();
        assert_eq!(tool, "Loop");
        assert_eq!(args["interval"], "5m");
        assert!(route(&json!({ "action": "create", "prompt": "p" })).is_err());
        assert!(route(&json!({ "action": "create", "cron": "x", "every": "5m" })).is_err());
    }

    #[test]
    fn an_id_says_whether_it_is_a_loop_or_a_cron_job() {
        let (tool, args) = route(&json!({ "action": "delete", "id": "loop-3" })).unwrap();
        assert_eq!((tool, args["loop_id"].as_str()), ("LoopStop", Some("loop-3")));
        assert_eq!(route(&json!({ "action": "delete", "id": "c0ffee" })).unwrap().0, "CronDelete");
        assert_eq!(route(&json!({ "action": "pause", "id": "c0ffee" })).unwrap().0, "CronPause");
        assert!(route(&json!({ "action": "pause", "id": "loop-1" })).unwrap_err().to_string().contains("can't be paused"));
        assert!(route(&json!({ "action": "stop" })).unwrap_err().to_string().contains("unknown action"));
    }

    #[test]
    fn every_action_is_in_the_schema() {
        let schema: Value = serde_json::from_str(crate::tool_schemas::SCHEDULE_TOOL).unwrap();
        let mut listed: Vec<&str> = schema["inputSchema"]["properties"]["action"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        let mut want = ACTIONS.to_vec();
        listed.sort();
        want.sort();
        assert_eq!(listed, want);
    }
}
