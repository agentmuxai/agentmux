// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What each Cron tool sends to srv and what it tells the agent back, pinned against a
//! stand-in srv, so `call` can be split into one function per tool without changing either.

use super::super::test_stub::{stub_srv, Fixture};
use super::*;

/// One call: how many requests it makes, the last one (path, and the body fields named
/// here) and the text it returns.
struct Case {
    tool: &'static str,
    args: Value,
    reply: Value,
    requests: usize,
    path: &'static str,
    sent: Value,
    says: &'static [&'static str],
}

fn cases() -> Vec<Case> {
    let c = |tool, args, reply, requests, path, sent, says| Case { tool, args, reply, requests, path, sent, says };
    let resolved = |extra: Value| {
        let mut reply = json!({ "resolution": "one", "uid": "u-1" });
        reply.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        reply
    };
    vec![
        c(
            "CronCreate",
            json!({
                "name": "nightly", "expression": "0 0 * * *", "prompt": "p", "to": "agent2", "max_fires": 3, "max_age_secs": 60,
                "catch_up": "all", "tz": "UTC", "policy": { "x": 1 }, "template": "t", "params": { "a": 1 }
            }),
            resolved(json!({
                "job": { "name": "named", "id": "j1", "tz": "Europe/Oslo", "next_fire": "2026-11-04T00:00:00Z" },
                "notes": ["cap clamped", "timeout added"]
            })),
            2,
            "/agentmux/jobs",
            json!({
                "name": "nightly", "expression": "0 0 * * *", "prompt": "p", "target": "agent2", "target_uid": "u-1",
                "created_by": "tester", "max_fires": 3, "max_age_secs": 60, "catch_up": "all", "tz": "UTC",
                "policy": { "x": 1 }, "template": "t", "params": { "a": 1 }
            }),
            &[
                "Created cron job 'named' (id=j1)",
                "Expression: 0 0 * * * Europe/Oslo",
                "Next fire: 2026-11-04T00:00:00Z",
                "Target: agent2",
                "\nNote: cap clamped\nNote: timeout added",
            ],
        ),
        c(
            "CronCreate",
            json!({ "name": "n", "expression": "e", "prompt": "p", "to": "agent2", "max_fires": 0, "policy": 3, "template": null, "params": [1] }),
            resolved(json!({})),
            2,
            "/agentmux/jobs",
            json!({ "target_uid": "u-1", "max_fires": null, "max_age_secs": null, "catch_up": null, "tz": null, "policy": null, "template": null, "params": null }),
            &["Created cron job 'n' (id=?)", "Expression: e UTC", "Next fire: unknown", "Target: agent2"],
        ),
        c(
            "CronCreate",
            json!({ "name": "n", "expression": "e", "prompt": "p", "to": "ghost" }),
            json!({ "resolution": "none" }),
            2,
            "/agentmux/jobs",
            json!({ "target": "ghost", "target_uid": "" }),
            &["Created cron job 'n'"],
        ),
        c("CronDelete", json!({ "id": "j1" }), json!({}), 1, "/agentmux/jobs/j1", json!({}), &["Deleted cron job j1"]),
        c(
            "CronList",
            json!({}),
            json!({ "jobs": [
                {
                    "id": "j1", "name": "nightly", "enabled": true, "next_fire": "soon", "fire_count": 2, "max_fires": 5,
                    "expires_in_secs": 30, "over_cap": "10 USD", "missed_count": 1, "last_miss_reason": "paused",
                    "expression": "0 0 * * *", "last_run": { "id": "r1", "state": "finished", "outcome": "ok" }
                },
                { "id": "j2", "name": "b", "enabled": false }
            ] }),
            1,
            "/agentmux/jobs",
            json!({}),
            &[
                "2 cron job(s):",
                "j1  nightly  [enabled]  fires=2/5  missed=1 (paused)  OVER CAP TODAY (10 USD)  next=soon  expr='0 0 * * *'  expires_in=30s",
                "last run: r1  finished  outcome=ok",
                "j2  b  [paused]  fires=0  next=—  expr='?'",
                "last run: none",
            ],
        ),
        c("CronList", json!({}), json!({ "jobs": [] }), 1, "/agentmux/jobs", json!({}), &["No cron jobs configured."]),
        c(
            "CronList",
            json!({}),
            json!({ "jobs": [], "paused_all": { "set_at_ms": 0 } }),
            1,
            "/agentmux/jobs",
            json!({}),
            &["The operator paused every job (Pause all,", "until they resume them.\nNo cron jobs configured."],
        ),
        c(
            "CronList",
            json!({}),
            json!({ "jobs": [{ "id": "j1", "name": "n", "enabled": true }], "paused_all": { "set_at_ms": 0 } }),
            1,
            "/agentmux/jobs",
            json!({}),
            &["The operator paused every job", "1 cron job(s):"],
        ),
        c("CronPause", json!({ "id": "j1" }), json!({}), 1, "/agentmux/jobs/j1", json!({ "action": "pause" }), &["Cron job j1 paused"]),
        c("CronResume", json!({ "id": "j1" }), json!({}), 1, "/agentmux/jobs/j1", json!({ "action": "resume" }), &["Cron job j1 resumed"]),
        c(
            "CronHistory",
            json!({ "id": "j1" }),
            json!({ "runs": [{
                "id": "r1", "state": "finished", "outcome": "ok", "skip_count": 3, "infra": "error",
                "cost_known": true, "cost_usd": 1.5, "cost_partial": true, "summary": "done"
            }] }),
            1,
            "/agentmux/jobs/j1/runs?limit=20",
            json!({}),
            &["1 run(s) of job j1, newest first:", "r1  finished x3  infra=error  outcome=ok", "$1.50 (partial)", "\"done\""],
        ),
        c(
            "CronHistory",
            json!({ "id": "j1", "limit": 5 }),
            json!({ "runs": [] }),
            1,
            "/agentmux/jobs/j1/runs?limit=5",
            json!({}),
            &["No runs recorded for job j1."],
        ),
        c(
            "CronRunNow",
            json!({ "id": "j1" }),
            json!({ "outcome": "delivered", "run_id": "r9" }),
            1,
            "/agentmux/jobs/j1/run",
            json!({}),
            &["Ran job j1 now: delivered (run r9)"],
        ),
        c(
            "CronRunNow",
            json!({ "id": "j1" }),
            json!({ "outcome": "skipped", "run_id": "r9", "error": "paused" }),
            1,
            "/agentmux/jobs/j1/run",
            json!({}),
            &["Ran job j1 now: skipped (run r9) — paused"],
        ),
        c("CronRunNow", json!({ "id": "j1" }), json!({}), 1, "/agentmux/jobs/j1/run", json!({}), &["Ran job j1 now: failed (run (not recorded))"]),
        c(
            "CronReport",
            json!({ "run_id": "r9", "outcome": "findings", "summary": "s", "links": ["l"] }),
            json!({}),
            1,
            "/agentmux/jobs/runs/r9/report",
            json!({ "outcome": "findings", "summary": "s", "links": ["l"] }),
            &["Reported run r9: findings"],
        ),
        c(
            "CronReport",
            json!({ "run_id": "r9", "outcome": "ok" }),
            json!({}),
            1,
            "/agentmux/jobs/runs/r9/report",
            json!({ "summary": "", "links": null }),
            &["Reported run r9: ok"],
        ),
    ]
}

#[tokio::test]
async fn every_cron_tool_sends_the_request_it_did_and_says_what_it_said() {
    for case in cases() {
        let srv = stub_srv("200 OK", case.reply.clone());
        let fx = Fixture::new(&srv);
        let said = call(case.tool, &case.args, &fx.cx()).await.unwrap_or_else(|e| panic!("{} {}: {e}", case.tool, case.args));
        let seen = srv.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), case.requests, "{} {}: requests {seen:?}", case.tool, case.args);
        let (path, body) = seen.last().unwrap();
        assert_eq!(path, case.path, "{} {}", case.tool, case.args);
        for (key, want) in case.sent.as_object().unwrap() {
            assert_eq!(body.get(key), Some(want), "{} {}: body {body}", case.tool, case.args);
        }
        for want in case.says {
            assert!(said.contains(want), "{} {}: said {said:?}, wanted {want:?}", case.tool, case.args);
        }
    }
}

#[tokio::test]
async fn a_name_to_resolve_is_sent_to_srv_before_the_job_is_created() {
    let srv = stub_srv("200 OK", json!({ "resolution": "one", "uid": "u-1" }));
    let fx = Fixture::new(&srv);
    let args = json!({ "name": "n", "expression": "e", "prompt": "p", "to": "agent2" });
    call("CronCreate", &args, &fx.cx()).await.unwrap();
    let seen = srv.seen.lock().unwrap().clone();
    assert_eq!(seen[0], ("/agentmux/agents/resolve".to_string(), json!({ "name": "agent2" })));
}

#[tokio::test]
async fn a_tool_missing_what_it_needs_refuses_before_it_asks_srv() {
    let full = json!({ "name": "n", "expression": "e", "prompt": "p", "to": "t" });
    let without = |key: &str| {
        let mut args = full.clone();
        args.as_object_mut().unwrap().remove(key);
        args
    };
    let refusals = [
        ("CronCreate", json!({}), "missing required parameter: name"),
        ("CronCreate", without("name"), "missing required parameter: name"),
        ("CronCreate", without("expression"), "missing required parameter: expression"),
        ("CronCreate", without("prompt"), "missing required parameter: prompt"),
        ("CronCreate", without("to"), "missing required parameter: to (target agent id)"),
        ("CronCreate", json!({ "name": "", "expression": "e", "prompt": "p", "to": "t" }), "missing required parameter: name"),
        ("CronDelete", json!({}), "missing required parameter: id"),
        ("CronPause", json!({}), "missing required parameter: id"),
        ("CronResume", json!({}), "missing required parameter: id"),
        ("CronHistory", json!({}), "missing required parameter: id"),
        ("CronRunNow", json!({}), "missing required parameter: id"),
        ("CronReport", json!({}), "missing required parameter: run_id"),
        ("CronReport", json!({ "run_id": "r" }), "missing required parameter: outcome"),
    ];
    for (tool, args, wanted) in refusals {
        let srv = stub_srv("200 OK", json!({}));
        let fx = Fixture::new(&srv);
        let err = call(tool, &args, &fx.cx()).await.expect_err(&format!("{tool} {args} should refuse")).to_string();
        assert!(err.contains(wanted), "{tool} {args}: {err:?}, wanted {wanted:?}");
        assert!(srv.seen.lock().unwrap().is_empty(), "{tool} {args} asked srv");
    }
}

#[tokio::test]
async fn a_refusal_from_srv_names_the_tool_and_the_status() {
    let create = json!({ "name": "n", "expression": "e", "prompt": "p", "to": "t" });
    let failures = [
        ("500 Internal Server Error", "CronCreate", create.clone(), "could not resolve agent \"t\": HTTP 500 Internal Server Error — \"boom\""),
        // A 404 from the resolver only means the srv predates it: the job request still goes out.
        ("404 Not Found", "CronCreate", create, "Schedule create failed: HTTP 404 Not Found — \"boom\""),
        ("500 Internal Server Error", "CronDelete", json!({ "id": "j" }), "Schedule delete failed: HTTP 500 Internal Server Error — \"boom\""),
        ("500 Internal Server Error", "CronList", json!({}), "Schedule list failed: HTTP 500 Internal Server Error — \"boom\""),
        ("500 Internal Server Error", "CronPause", json!({ "id": "j" }), "CronPause failed: HTTP 500 Internal Server Error — \"boom\""),
        ("500 Internal Server Error", "CronResume", json!({ "id": "j" }), "CronResume failed: HTTP 500 Internal Server Error — \"boom\""),
        ("500 Internal Server Error", "CronHistory", json!({ "id": "j" }), "Schedule history failed: HTTP 500 Internal Server Error — \"boom\""),
        ("500 Internal Server Error", "CronRunNow", json!({ "id": "j" }), "Schedule run_now failed: HTTP 500 Internal Server Error — \"boom\""),
        ("500 Internal Server Error", "CronReport", json!({ "run_id": "r", "outcome": "ok" }), "Schedule report failed: HTTP 500 Internal Server Error — \"boom\""),
    ];
    for (status, tool, args, wanted) in failures {
        let srv = stub_srv(status, json!("boom"));
        let fx = Fixture::new(&srv);
        let err = call(tool, &args, &fx.cx()).await.expect_err(&format!("{tool} should fail")).to_string();
        assert!(err.contains(wanted), "{tool} {args}: {err:?}, wanted {wanted:?}");
    }
}

#[tokio::test]
async fn a_job_srv_does_not_know_is_told_to_the_agent_not_raised() {
    let missing = [
        ("CronDelete", "Job 'j' not found (already deleted or wrong id)"),
        ("CronPause", "Job 'j' not found"),
        ("CronResume", "Job 'j' not found"),
        ("CronRunNow", "Job 'j' not found"),
    ];
    for (tool, wanted) in missing {
        let srv = stub_srv("404 Not Found", json!({}));
        let fx = Fixture::new(&srv);
        let said = call(tool, &json!({ "id": "j" }), &fx.cx()).await.unwrap_or_else(|e| panic!("{tool}: {e}"));
        assert_eq!(said, wanted, "{tool}");
    }
}

#[tokio::test]
async fn a_name_that_is_not_a_cron_tool_is_not_in_the_family() {
    let srv = stub_srv("200 OK", json!({}));
    let fx = Fixture::new(&srv);
    assert!(call("CronNope", &json!({}), &fx.cx()).await.is_err());
    assert!(srv.seen.lock().unwrap().is_empty());
}
