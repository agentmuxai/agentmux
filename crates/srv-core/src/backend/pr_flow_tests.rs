// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use serde_json::{json, Value};

use super::*;

const TEXT: &str = "[review] PR #64: a title\n\nbody";

fn notice<'a>(id: &'a str, text: &'a str, genuine: bool) -> Notice<'a> {
    Notice { id, agent: "lazo", text, genuine }
}

fn review() -> Value {
    json!({ "kind": "review", "repo_id": 1234, "pr": 64, "state": "approved", "by": "bot", "head": "88F24B9C93D73B785E293D1291E63CB71450D488" })
}

fn accepted(flow: &Value) -> Option<Accepted> {
    accept_one(Some(flow), &notice("inj-1", TEXT, true), 7)
}

/// `review()` with `field` set to `value`, or removed when `value` is null.
fn with(field: &str, value: Value) -> Value {
    let mut flow = review();
    if value.is_null() {
        flow.as_object_mut().unwrap().remove(field);
    } else {
        flow[field] = value;
    }
    flow
}

#[test]
fn a_review_event_has_the_ids_the_state_and_the_head_and_nothing_else() {
    let Some(Accepted::Review(e)) = accepted(&review()) else { panic!("no review event") };
    assert_eq!(
        serde_json::to_value(&e).unwrap(),
        json!({
            "id": "inj-1", "at_ms": 7, "agent": "lazo",
            "pr": { "repo_id": 1234, "number": 64 },
            "state": "approved", "by": "bot", "head": "88f24b9c93d7",
        })
    );
}

#[test]
fn a_checks_event_and_a_merge_event_have_no_reviewer_kind() {
    let checks = json!({ "kind": "checks", "repo_id": 5, "pr": 64, "state": "failing", "by": "bot" });
    let Some(Accepted::Checks(e)) = accepted(&checks) else { panic!("no checks event") };
    assert_eq!(
        serde_json::to_value(&e).unwrap(),
        json!({ "id": "inj-1", "at_ms": 7, "agent": "lazo", "pr": { "repo_id": 5, "number": 64 }, "state": "failing" })
    );
    let merge = json!({ "kind": "merge", "repo_id": 5, "pr": 64, "state": "merged" });
    let Some(Accepted::Merge(e)) = accepted(&merge) else { panic!("no merge event") };
    assert_eq!(
        serde_json::to_value(&e).unwrap(),
        json!({ "id": "inj-1", "at_ms": 7, "agent": "lazo", "pr": { "repo_id": 5, "number": 64 }, "state": "merged" })
    );
}

#[test]
fn a_state_this_version_does_not_name_is_kept() {
    for state in ["pending", "requested", "closed", "a_b_c", "x"] {
        let Some(Accepted::Review(e)) = accepted(&with("state", json!(state))) else { panic!("{state} dropped") };
        assert_eq!(e.state, state);
    }
}

#[test]
fn a_malformed_state_makes_no_event() {
    let long = "a".repeat(STATE_MAX + 1);
    for state in [json!(""), json!("Approved"), json!("changes-requested"), json!("approved "), json!(long), json!(3), json!(null), json!(["approved"])] {
        assert!(accepted(&with("state", state.clone())).is_none(), "{state} accepted");
    }
    assert!(accepted(&with("state", json!("a".repeat(STATE_MAX)))).is_some());
    assert!(accepted(&with("state", Value::Null)).is_none(), "a missing state");
}

#[test]
fn a_kind_or_reviewer_the_app_does_not_know_makes_no_event() {
    assert!(accepted(&with("kind", json!("closed"))).is_none());
    assert!(accepted(&with("kind", json!("Review"))).is_none());
    assert!(accepted(&with("kind", Value::Null)).is_none());
    assert!(accepted(&with("by", json!("robot"))).is_none());
    assert!(accepted(&with("by", Value::Null)).is_none(), "a review says who reviewed");
    for by in ["bot", "codex", "person"] {
        assert!(accepted(&with("by", json!(by))).is_some(), "{by}");
    }
}

#[test]
fn the_ids_are_positive_whole_numbers_a_script_holds_exactly() {
    for bad in [json!(0), json!(-1), json!(1.5), json!("64"), json!(null), json!(SAFE_MAX + 1), json!(u64::MAX)] {
        assert!(accepted(&with("repo_id", bad.clone())).is_none(), "repo_id {bad}");
    }
    assert!(accepted(&with("repo_id", json!(SAFE_MAX))).is_some());
    assert!(accepted(&with("repo_id", Value::Null)).is_none());
    assert!(accepted(&with("pr", Value::Null)).is_none());
}

#[test]
fn a_head_is_seven_to_forty_hex_characters_kept_to_twelve_and_a_bad_one_drops_the_event() {
    let head = |flow: &Value| match accepted(flow) {
        Some(Accepted::Review(e)) => Some(e.head),
        _ => None,
    };
    assert_eq!(head(&with("head", json!("abcdef0"))), Some(Some("abcdef0".to_string())));
    assert_eq!(head(&with("head", Value::Null)), Some(None));
    let mut explicit_null = review();
    explicit_null["head"] = Value::Null;
    assert_eq!(head(&explicit_null), Some(None), "an explicit null is no head");
    for bad in [json!("abcdef"), json!("g".repeat(12)), json!("a".repeat(41)), json!(""), json!(12345678)] {
        assert_eq!(head(&with("head", bad.clone())), None, "{bad}");
    }
}

#[test]
fn the_pr_must_be_the_one_in_the_first_line_of_the_signed_text() {
    let ok = |text: &str| accept_one(Some(&review()), &notice("inj-1", text, true), 7).is_some();
    assert!(ok("[review] PR #64: a title"));
    assert!(ok("PR #64"), "the number may end the line");
    assert!(!ok("[review] PR #65: a title"), "another PR");
    assert!(!ok("[review] PR #640: a title"), "a longer number is not the PR");
    assert!(!ok("[review] PR #6: a title"), "a shorter one neither");
    assert!(!ok("[review] a title\nPR #64"), "only the first line names it");
    assert!(!ok(""));
    assert!(!ok("[review] pr 64"), "a wording change fails closed");
}

#[test]
fn a_notice_that_is_not_genuine_and_fresh_makes_no_event() {
    assert!(accept_one(Some(&review()), &notice("inj-1", TEXT, false), 7).is_none());
}

#[test]
fn a_notice_with_no_object_or_a_non_object_or_no_id_or_no_agent_makes_no_event() {
    assert!(accept_one(None, &notice("inj-1", TEXT, true), 7).is_none());
    for flow in [json!(null), json!("review"), json!([1]), json!(3)] {
        assert!(accept_one(Some(&flow), &notice("inj-1", TEXT, true), 7).is_none(), "{flow}");
    }
    assert!(accept_one(Some(&review()), &notice("", TEXT, true), 7).is_none());
    assert!(accept_one(Some(&review()), &Notice { id: "inj-1", agent: "", text: TEXT, genuine: true }, 7).is_none());
}

#[test]
fn a_field_the_app_does_not_know_is_ignored_and_never_reaches_the_event() {
    let mut flow = review();
    flow["title"] = json!("Secret title");
    flow["login"] = json!("someone");
    flow["url"] = json!("https://example.invalid/x");
    let Some(Accepted::Review(e)) = accepted(&flow) else { panic!("dropped") };
    let out = serde_json::to_string(&e).unwrap();
    for leaked in ["Secret title", "someone", "example.invalid", "title", "login", "url"] {
        assert!(!out.contains(leaked), "{leaked} in {out}");
    }
}

#[test]
fn a_repeated_id_is_the_same_event_and_the_list_is_bounded() {
    let event = |id: &str, at_ms: u64| match accept_one(Some(&review()), &notice(id, TEXT, true), at_ms) {
        Some(Accepted::Review(e)) => Accepted::Review(e),
        _ => panic!("no event"),
    };
    let mut recent = Recent::default();
    assert!(recent.keep(&event("m-1", 5)));
    assert!(!recent.keep(&event("m-1", 6)), "a redelivery");
    assert_eq!(recent.snapshot().review.len(), 1);
    assert_eq!(recent.snapshot().bot.last_notice_at_ms, Some(5), "a redelivery is not a new notice");
    for n in 0..RECENT_MAX + 5 {
        recent.keep(&event(&format!("n-{n}"), 10 + n as u64));
    }
    let review = recent.snapshot().review;
    assert_eq!(review.len(), RECENT_MAX);
    assert_eq!(review.last().unwrap().id, format!("n-{}", RECENT_MAX + 4));
    assert_eq!(review.first().unwrap().id, "n-5", "oldest first, the oldest dropped");
}

#[test]
fn the_bot_state_is_the_newest_accepted_notice_of_any_kind() {
    let mut recent = Recent::default();
    assert_eq!(recent.snapshot().bot.last_notice_at_ms, None);
    let merge = json!({ "kind": "merge", "repo_id": 5, "pr": 64, "state": "merged" });
    recent.keep(&accept_one(Some(&merge), &notice("m-1", TEXT, true), 20).unwrap());
    recent.keep(&accept_one(Some(&review()), &notice("m-2", TEXT, true), 10).unwrap());
    assert_eq!(recent.snapshot().bot.last_notice_at_ms, Some(20));
    assert_eq!(serde_json::to_value(recent.snapshot().bot).unwrap(), json!({ "last_notice_at_ms": 20 }));
}

#[test]
fn an_empty_answer_has_the_three_lists_and_a_null_time() {
    assert_eq!(
        serde_json::to_value(PrFlowSnapshot::default()).unwrap(),
        json!({ "review": [], "checks": [], "merge": [], "bot": { "last_notice_at_ms": null } })
    );
}

#[test]
fn each_kind_is_published_as_its_own_event_once() {
    let broker = Arc::new(Broker::new());
    let published = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&published);
    broker.add_observer(Arc::new(move |e: &crate::backend::mps::MuxEvent| sink.lock().unwrap().push((e.event.clone(), e.data.clone()))));
    let flow = Flow { broker, recent: Mutex::default() };
    let of = |kind: &str, state: &str, id: &str| {
        let obj = json!({ "kind": kind, "repo_id": 5, "pr": 64, "state": state, "by": "bot" });
        accept_one(Some(&obj), &notice(id, TEXT, true), 9).unwrap()
    };
    flow.record(&of("review", "approved", "m-1"));
    flow.record(&of("checks", "passing", "m-2"));
    flow.record(&of("merge", "merged", "m-3"));
    flow.record(&of("merge", "merged", "m-3"));
    let published = published.lock().unwrap();
    let names: Vec<&str> = published.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["review-flow", "checks-flow", "merge-flow"]);
    assert_eq!(published[0].1.as_ref().unwrap()["pr"]["number"], 64);
    assert_eq!(flow.recent.lock().unwrap().snapshot().merge.len(), 1);
}

#[test]
fn a_list_makes_an_event_for_each_valid_object_and_at_most_three() {
    let checks = json!({ "kind": "checks", "repo_id": 1234, "pr": 64, "state": "passing", "head": "abcdef0" });
    let n = notice("inj-1", TEXT, true);
    let two = accept(Some(&json!([review(), checks])), &n, 7);
    assert_eq!(two.len(), 2);
    assert!(matches!(two[0], Accepted::Review(_)) && matches!(two[1], Accepted::Checks(_)));
    // One bad object does not spoil the other; one for another PR is not drawn.
    let other_pr = with("pr", json!(65));
    let mixed = accept(Some(&json!([with("state", json!("Bad")), checks, other_pr])), &n, 7);
    assert_eq!(mixed.len(), 1);
    // A single object is a list of one, and the list is bounded.
    assert_eq!(accept(Some(&review()), &n, 7).len(), 1);
    assert_eq!(accept(Some(&json!([review(), review(), review(), review()])), &n, 7).len(), OBJECTS_MAX);
    assert!(accept(Some(&json!([])), &n, 7).is_empty());
    assert!(accept(Some(&json!([review()])), &notice("inj-1", TEXT, false), 7).is_empty(), "a notice that is not genuine shows nothing");
}

#[test]
fn the_events_of_one_notice_share_its_id_in_their_own_lists() {
    let checks = json!({ "kind": "checks", "repo_id": 1234, "pr": 64, "state": "passing" });
    let mut recent = Recent::default();
    for accepted in accept(Some(&json!([review(), checks])), &notice("inj-1", TEXT, true), 7) {
        assert!(recent.keep(&accepted));
    }
    let snapshot = recent.snapshot();
    assert_eq!((snapshot.review.len(), snapshot.checks.len()), (1, 1));
}
