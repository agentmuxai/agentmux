// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What each Tasks tool sends to srv and what it tells the agent back, pinned against a
//! stand-in srv, so `call` can be split into one function per tool without changing either.

use super::super::test_stub::{stub_srv, Fixture};
use super::*;

/// One call: the request it sends (path with query, and the body fields named here) and the
/// text it returns.
struct Case {
    tool: &'static str,
    args: Value,
    reply: Value,
    path: &'static str,
    sent: Value,
    says: &'static [&'static str],
}

fn cases() -> Vec<Case> {
    let c = |tool, args, reply, path, sent, says| Case { tool, args, reply, path, sent, says };
    vec![
        c(
            "WorkEnqueue",
            json!({ "title": "T", "payload": "P", "for": "me", "priority": 2, "kind": "review" }),
            json!({ "id": "abc", "num": 5 }),
            "/agentmux/work",
            json!({ "title": "T", "payload": "P", "kind": "review", "target_agent": "tester", "priority": 2, "created_by": "tester", "hold": false }),
            &["in your backlog: T. Tell the operator its number."],
        ),
        c(
            "WorkEnqueue",
            json!({ "title": "T", "payload": "P", "first": "spec" }),
            json!({ "id": "abc", "num": 5, "build_num": 6, "note": "wait" }),
            "/agentmux/work",
            json!({ "first": "spec", "target_agent": "" }),
            &["(the spec) and #6 (the build, which starts after it) in the queue: T.", "Note: wait"],
        ),
        c("WorkEnqueue", json!({ "title": "T", "payload": "P" }), json!({ "id": "abc", "num": 5 }), "/agentmux/work", json!({}), &["Enqueued", ": T"]),
        c(
            "WorkClaim",
            json!({ "kind": "review", "lease_ms": 5000 }),
            json!({ "claimed": true, "attempt": 1, "num": 12, "item": { "id": "i1", "title": "TT", "payload": "PP" } }),
            "/agentmux/work/claim",
            json!({ "agent_id": "tester", "block_id": "blk", "kind": "review", "lease_ms": 5000, "groups": [] }),
            &["Claimed", "(attempt 1)", "id=12 and attempt=1", "Title: TT", "PP"],
        ),
        c(
            "WorkClaim",
            json!({}),
            json!({ "claimed": true, "attempt": 2, "num": 12, "item": { "id": "i1", "title": "TT", "payload": "PP", "result": "blocked on X" } }),
            "/agentmux/work/claim",
            json!({}),
            &["NOTE — a previous agent held this and handed it back", "Their reason: blocked on X"],
        ),
        c("WorkClaim", json!({}), json!({ "claimed": false }), "/agentmux/work/claim", json!({}), &["No work available on the queue right now."]),
        c("WorkClaim", json!({}), json!({ "claimed": false, "reason": "quota" }), "/agentmux/work/claim", json!({}), &["Not claimed: quota."]),
        c(
            "WorkClaim",
            json!({ "id": "12" }),
            json!({ "claimed": false, "why": "it is on hold" }),
            "/agentmux/work/claim",
            json!({ "id": "12" }),
            &["Not claimed: it is on hold."],
        ),
        c(
            "WorkUpdate",
            json!({ "id": "#7", "attempt": 1, "note": "n", "links": { "pr": "https://x/pr/1", "branch": "b" } }),
            json!({}),
            "/agentmux/work/7/update",
            json!({ "agent_id": "tester", "attempt": 1, "note": "n", "pr_url": "https://x/pr/1", "branch": "b" }),
            &["Updated task 7."],
        ),
        c(
            "WorkAsk",
            json!({ "id": "7", "attempt": 1, "question": "Which one?" }),
            json!({}),
            "/agentmux/work/7/ask",
            json!({ "attempt": 1, "text": "Which one?" }),
            &["Asked: task 7 now waits in the operator's Needs you column"],
        ),
        c(
            "WorkSubmit",
            json!({ "id": "7", "attempt": 1, "result": "done, PR 1" }),
            json!({}),
            "/agentmux/work/7/submit",
            json!({ "attempt": 1, "text": "done, PR 1" }),
            &[],
        ),
        c("WorkHeartbeat", json!({ "id": "7", "attempt": 2 }), json!({}), "/agentmux/work/7/heartbeat", json!({ "attempt": 2 }), &["Lease extended on 7."]),
        c(
            "WorkComplete",
            json!({ "id": "7", "attempt": 2, "result": "ok" }),
            json!({}),
            "/agentmux/work/7/complete",
            json!({ "attempt": 2, "result": "ok" }),
            &[],
        ),
        c(
            "WorkRelease",
            json!({ "id": "7", "attempt": 2, "reason": "stuck" }),
            json!({ "state": "open" }),
            "/agentmux/work/7/release",
            json!({ "attempt": 2, "result": "stuck" }),
            &["Released 7 back to the queue for another agent."],
        ),
        c(
            "WorkRelease",
            json!({ "id": "7", "attempt": 2, "reason": "outside my seat", "handback": true }),
            json!({ "state": "open" }),
            "/agentmux/work/7/release",
            json!({ "attempt": 2, "result": "outside my seat", "handback": true }),
            &["Handed 7 back to the pool"],
        ),
        c(
            "WorkRelease",
            json!({ "id": "7", "attempt": 3 }),
            json!({ "state": "failed" }),
            "/agentmux/work/7/release",
            json!({}),
            &["parked as FAILED"],
        ),
        c("WorkHold", json!({ "id": "7", "reason": "wait" }), json!({}), "/agentmux/work/7/hold", json!({ "agent_id": "tester", "reason": "wait" }), &[]),
        c("WorkUnhold", json!({ "id": "7" }), json!({}), "/agentmux/work/7/unhold", json!({ "agent_id": "tester" }), &[]),
        c("WorkAccept", json!({ "id": "7", "note": "nice" }), json!({}), "/agentmux/work/7/accept", json!({ "agent_id": "tester", "text": "nice" }), &[]),
        c("WorkSendBack", json!({ "id": "7", "reason": "redo" }), json!({}), "/agentmux/work/7/send_back", json!({ "text": "redo" }), &[]),
        c(
            "WorkEdit",
            json!({ "id": "7", "title": "N", "priority": 3 }),
            json!({}),
            "/agentmux/work/act",
            json!({ "agent_id": "tester", "action": "edit", "id": "7", "title": "N", "priority": 3 }),
            &["Task #7: edit done."],
        ),
        c("WorkMove", json!({ "id": "7", "rank": 1.5 }), json!({}), "/agentmux/work/act", json!({ "action": "move", "rank": 1.5 }), &["Task #7: move done."]),
        c("WorkCancel", json!({ "id": "7", "text": "no" }), json!({}), "/agentmux/work/act", json!({ "action": "cancel", "text": "no" }), &["Task #7: cancel done."]),
        c("WorkTakeBack", json!({ "id": "7", "reason": "r" }), json!({}), "/agentmux/work/act", json!({ "action": "release", "text": "r" }), &["Task #7: release done."]),
        c(
            "WorkCancel",
            json!({ "id": "7" }),
            json!({ "confirm": "It has a PR." }),
            "/agentmux/work/act",
            json!({}),
            &["Task #7 not changed. It has a PR. Call again with confirm: true to go ahead."],
        ),
        c(
            "WorkList",
            json!({ "state": "open", "limit": 5 }),
            json!({ "items": [{ "num": 3, "state": "open", "title": "Tt", "result": "R" }] }),
            "/agentmux/work?state=open&limit=5",
            json!({}),
            &["1 work item(s):", "#3  open  Tt", "-> R"],
        ),
        // A claimed task's result is why the previous holder handed it back: not shown as the holder's verdict.
        c(
            "WorkList",
            json!({ "state": "claimed", "limit": 5 }),
            json!({ "items": [{ "num": 4, "state": "claimed", "claimed_by": "deephar", "title": "Tt", "result": "agent stopped" }] }),
            "/agentmux/work?state=claimed&limit=5",
            json!({}),
            &["#4  claimed, held by deephar  Tt", "(handed back earlier: agent stopped)"],
        ),
        // Who a task is for and who holds it.
        c(
            "WorkList",
            json!({ "state": "open", "limit": 5 }),
            json!({ "items": [
                { "num": 5, "state": "open", "target_agent": "Dax", "title": "Aimed" },
                { "num": 6, "state": "open", "target_group": "g1", "title": "Grouped" },
                { "num": 7, "state": "open", "title": "Anyone" },
                { "num": 8, "state": "claimed", "target_agent": "Dax", "claimed_by": "loap", "title": "Taken" },
                { "num": 9, "state": "claimed", "target_agent": "loap", "claimed_by": "loap", "title": "Started" },
            ] }),
            "/agentmux/work?state=open&limit=5",
            json!({}),
            &[
                "#5  open, for Dax  Aimed",
                "#6  open, for group g1  Grouped",
                "#7  open  Anyone",
                "#8  claimed, for Dax, held by loap  Taken",
                "#9  claimed, held by loap  Started",
            ],
        ),
        c("WorkList", json!({}), json!({ "items": [] }), "/agentmux/work?state=&limit=50", json!({}), &["Queue is empty."]),
        // Several boards.
        c(
            "WorkEnqueue",
            json!({ "title": "T", "payload": "P", "board": "work" }),
            json!({ "id": "abc", "num": 5, "ref": "work#5", "board": "work" }),
            "/agentmux/work",
            json!({ "board": "work" }),
            &["Enqueued task work#5: T"],
        ),
        c(
            "WorkClaim",
            json!({ "board": "work" }),
            json!({ "claimed": true, "attempt": 1, "num": 5, "ref": "work#5", "item": { "id": "abc", "title": "T", "payload": "P" } }),
            "/agentmux/work/claim",
            json!({ "board": "work" }),
            &["Claimed task work#5", "pass id=work#5 and attempt=1"],
        ),
        c(
            "WorkList",
            json!({ "board": "all" }),
            json!({ "board": "all", "note": "2 more for you on Work", "items": [{ "num": 3, "ref": "work#3", "state": "open", "title": "Tt" }] }),
            "/agentmux/work?state=&limit=50&board=all",
            json!({}),
            &["1 work item(s) on all:", "work#3  open  Tt", "2 more for you on Work"],
        ),
        c(
            "WorkList",
            json!({}),
            json!({ "board": "main", "note": "3 more for you on Work and Client X", "items": [] }),
            "/agentmux/work?state=&limit=50",
            json!({}),
            &["Queue is empty on board main.", "3 more for you on Work and Client X"],
        ),
        // A ref goes in the path with its # encoded; a bare number with a board named is that board's.
        c("WorkHold", json!({ "id": "work#7", "reason": "wait" }), json!({}), "/agentmux/work/work%237/hold", json!({ "reason": "wait" }), &[]),
        c("WorkHold", json!({ "id": "7", "board": "work", "reason": "wait" }), json!({}), "/agentmux/work/work%237/hold", json!({ "reason": "wait" }), &[]),
        c(
            "WorkEdit",
            json!({ "id": "work#7", "title": "N" }),
            json!({}),
            "/agentmux/work/act",
            json!({ "action": "edit", "id": "work#7", "title": "N" }),
            &["Task work#7: edit done."],
        ),
        // `move` with a board moves the task there; the board is where it goes, not the id's.
        c(
            "WorkMove",
            json!({ "id": "7", "board": "work", "drop_deps": true }),
            json!({ "ok": true, "ref": "work#2", "from": "#7", "dropped_dependencies": ["#9"] }),
            "/agentmux/work/act",
            json!({ "action": "move", "id": "7", "board": "work", "drop_deps": true }),
            &["Task #7 moved to work#2", "dependencies on #9 were removed"],
        ),
        c(
            "WorkBoards",
            json!({}),
            json!({ "default": "main", "boards": [
                { "slug": "main", "name": "Main", "default": true, "archived": false, "open": 3, "total": 12, "agents": [] },
                { "slug": "team", "name": "Team", "default": false, "archived": false, "open": 1, "total": 1, "agents": ["agentx"] }
            ] }),
            "/agentmux/work/boards?archived=false",
            json!({}),
            &["2 board(s) you see:", "main (Main) [default]: 3 open of 12 task(s)", "team (Team): 1 open of 1 task(s), agents: agentx"],
        ),
        c(
            "WorkBoardCreate",
            json!({ "name": "Work", "agents": ["agentx"] }),
            json!({ "ok": true, "board": { "slug": "work", "name": "Work", "default": false, "archived": false, "open": 0, "total": 0, "agents": ["agentx"] } }),
            "/agentmux/work/boards",
            json!({ "action": "create", "name": "Work", "agents": ["agentx"] }),
            &["Done. work (Work): 0 open of 0 task(s), agents: agentx"],
        ),
        c(
            "WorkBoardDelete",
            json!({ "board": "old" }),
            json!({ "ok": true, "deleted": false, "note": "board old and its 2 task(s) will be deleted in 15 seconds" }),
            "/agentmux/work/boards",
            json!({ "action": "delete", "board": "old" }),
            &["will be deleted in 15 seconds"],
        ),
    ]
}

#[tokio::test]
async fn every_tasks_tool_sends_the_request_it_did_and_says_what_it_said() {
    for case in cases() {
        let srv = stub_srv("200 OK", case.reply.clone());
        let fx = Fixture::new(&srv);
        let said = call(case.tool, &case.args, &fx.cx()).await.unwrap_or_else(|e| panic!("{} {}: {e}", case.tool, case.args));
        let seen = srv.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 1, "{} {}: requests {seen:?}", case.tool, case.args);
        let (path, body) = &seen[0];
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
async fn a_claim_by_id_that_got_another_task_gives_it_straight_back() {
    let srv = stub_srv("200 OK", json!({ "claimed": true, "attempt": 1, "num": 9, "item": { "id": "other", "title": "t", "payload": "p" } }));
    let fx = Fixture::new(&srv);
    let err = call("WorkClaim", &json!({ "id": "12" }), &fx.cx()).await.unwrap_err().to_string();
    assert!(err.contains("This AgentMux can't claim a task by id yet"), "{err}");
    let seen = srv.seen.lock().unwrap().clone();
    let paths: Vec<&str> = seen.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(paths, ["/agentmux/work/claim", "/agentmux/work/other/release"]);
    assert_eq!(seen[1].1["attempt"], 1);
}

#[tokio::test]
async fn a_tool_missing_what_it_needs_refuses_before_it_asks_srv() {
    let refusals = [
        ("WorkEnqueue", json!({}), "missing required parameter: title"),
        ("WorkEnqueue", json!({ "title": "T" }), "missing required parameter: payload"),
        ("WorkUpdate", json!({}), "missing required parameter: id"),
        ("WorkUpdate", json!({ "id": "a/b", "attempt": 1 }), "is neither a task number"),
        ("WorkAsk", json!({ "id": "7", "attempt": 1 }), "Tasks ask needs a question"),
        ("WorkSubmit", json!({ "id": "7", "attempt": 1 }), "Tasks submit needs a result"),
        ("WorkComplete", json!({ "id": "7", "attempt": 1 }), "Tasks complete requires a non-empty 'result'"),
        ("WorkHold", json!({}), "missing required parameter: id"),
        ("WorkAccept", json!({}), "missing required parameter: id"),
        ("WorkSendBack", json!({ "id": "7" }), "reason is required"),
        ("WorkEdit", json!({}), "missing required parameter: id"),
        // A field another move takes is refused by name, never answered "done".
        ("WorkEdit", json!({ "id": "7", "target_agent": "Dax" }), "to change who a task is for, use move with target_agent"),
        ("WorkEdit", json!({ "id": "7", "rank": 1.5 }), "rank belongs to move"),
        ("WorkMove", json!({ "id": "7", "title": "N" }), "title belongs to edit"),
        ("WorkCancel", json!({ "id": "7", "target_agent": "Dax" }), "Tasks cancel doesn't take target_agent"),
        ("WorkReopen", json!({ "id": "7", "hold": true }), "hold belongs to move"),
    ];
    for (tool, args, wanted) in refusals {
        let srv = stub_srv("200 OK", json!({}));
        let fx = Fixture::new(&srv);
        let err = call(tool, &args, &fx.cx()).await.expect_err(&format!("{tool} {args} should refuse")).to_string();
        assert!(err.contains(wanted), "{tool} {args}: {err:?}, wanted {wanted:?}");
        assert!(srv.seen.lock().unwrap().is_empty(), "{tool} {args} asked srv");
    }
}

/// What a call without `attempt` is told, given the tasks srv lists: the refusal asks srv which tasks the agent
/// holds (a GET for the claimed ones and one for the blocked ones), and sends nothing else.
async fn missing_attempt_says(tool: &str, args: Value, listed: Value) -> String {
    let srv = stub_srv("200 OK", listed);
    let fx = Fixture::new(&srv);
    let err = call(tool, &args, &fx.cx()).await.expect_err("a call without an attempt refuses").to_string();
    let seen = srv.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 2, "only the lists of claimed and of blocked tasks: {seen:?}");
    assert!(seen[0].0.starts_with("/agentmux/work?") && seen[0].0.contains("state=claimed"), "{seen:?}");
    assert!(seen[1].0.starts_with("/agentmux/work?") && seen[1].0.contains("state=blocked"), "{seen:?}");
    assert!(seen.iter().all(|s| s.0.contains("board=all")), "a task on any board: {seen:?}");
    err
}

#[tokio::test]
async fn a_call_without_an_attempt_is_told_the_attempt_it_holds() {
    let listed = json!({ "items": [
        { "ref": "#46807", "state": "claimed", "claimed_by": "tester", "attempts": 2 },
        { "ref": "#9", "state": "claimed", "claimed_by": "someone-else", "attempts": 5 },
    ] });
    for tool in ["WorkUpdate", "WorkHeartbeat"] {
        let err = missing_attempt_says(tool, json!({ "id": "46807" }), listed.clone()).await;
        assert!(err.contains("You hold #46807 as attempt 2: pass id=46807 attempt=2"), "{tool}: {err}");
    }
}

#[tokio::test]
async fn a_call_without_an_attempt_for_a_task_it_does_not_hold_lists_what_it_holds() {
    let listed = json!({ "items": [
        { "ref": "#1", "state": "claimed", "claimed_by": "tester", "attempts": 1 },
        { "num": 2, "state": "claimed", "claimed_by": "tester", "attempts": 3 },
    ] });
    let err = missing_attempt_says("WorkUpdate", json!({ "id": "7" }), listed).await;
    assert!(err.contains("You hold: #1 (attempt 1), #2 (attempt 3)"), "{err}");
    assert!(!err.contains("attempt=") , "it does not pick one for the agent: {err}");
}

#[tokio::test]
async fn a_call_without_an_attempt_for_a_task_it_asked_about_is_told_the_attempt_it_holds() {
    // After `Tasks ask` the task is blocked, still held, and update and submit still take it.
    let listed = json!({ "items": [{ "ref": "#47380", "state": "blocked", "claimed_by": "tester", "attempts": 1 }] });
    for tool in ["WorkUpdate", "WorkSubmit"] {
        let err = missing_attempt_says(tool, json!({ "id": "47380", "result": "R" }), listed.clone()).await;
        assert!(err.contains("You hold #47380 as attempt 1: pass id=47380 attempt=1"), "{tool}: {err}");
    }
}

#[tokio::test]
async fn a_call_without_an_attempt_and_no_claimed_task_is_told_to_claim() {
    let err = missing_attempt_says("WorkUpdate", json!({ "id": "7" }), json!({ "items": [] })).await;
    assert!(err.contains("You hold no claimed task: claim one first"), "{err}");
}

#[tokio::test]
async fn a_refusal_from_srv_names_the_tool_and_the_status() {
    let failures = [
        ("WorkEnqueue", json!({ "title": "T", "payload": "P" }), "Tasks enqueue failed: HTTP 500 Internal Server Error"),
        ("WorkClaim", json!({}), "Tasks claim failed: HTTP 500 Internal Server Error"),
        ("WorkUpdate", json!({ "id": "7", "attempt": 1 }), "Tasks update failed: HTTP 500 Internal Server Error"),
        ("WorkHeartbeat", json!({ "id": "7", "attempt": 1 }), "WorkHeartbeat failed: HTTP 500 Internal Server Error"),
        ("WorkHold", json!({ "id": "7" }), "Tasks hold:"),
        ("WorkAccept", json!({ "id": "7" }), "Tasks accept:"),
        ("WorkEdit", json!({ "id": "7" }), "Tasks edit:"),
        ("WorkList", json!({}), "Tasks list failed: HTTP 500 Internal Server Error"),
    ];
    for (tool, args, wanted) in failures {
        let srv = stub_srv("500 Internal Server Error", json!({ "error": "boom" }));
        let fx = Fixture::new(&srv);
        let err = call(tool, &args, &fx.cx()).await.expect_err(&format!("{tool} should fail")).to_string();
        assert!(err.contains(wanted), "{tool} {args}: {err:?}, wanted {wanted:?}");
    }
}

#[tokio::test]
async fn a_conflict_means_the_task_is_no_longer_the_agents() {
    for (tool, args, wanted) in [
        ("WorkUpdate", json!({ "id": "7", "attempt": 1 }), "This task is no longer yours"),
        ("WorkHeartbeat", json!({ "id": "7", "attempt": 1 }), "This claim is no longer yours"),
    ] {
        let srv = stub_srv("409 Conflict", json!({}));
        let fx = Fixture::new(&srv);
        let err = call(tool, &args, &fx.cx()).await.unwrap_err().to_string();
        assert!(err.contains(wanted), "{tool}: {err}");
    }
}

#[tokio::test]
async fn a_name_that_is_not_a_tasks_tool_is_not_in_the_family() {
    let srv = stub_srv("200 OK", json!({}));
    let fx = Fixture::new(&srv);
    assert!(call("WorkNope", &json!({}), &fx.cx()).await.is_err());
    assert!(srv.seen.lock().unwrap().is_empty());
}
