// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::storage::work_board::WorkCard;
use crate::backend::storage::work_prs::WorkPr;
use crate::backend::storage::work_queue::{ClaimFilter, WorkItem};

fn pr(repo: &str, number: u64) -> Option<PrRef> {
    Some(PrRef { repo: repo.into(), number })
}

#[test]
fn a_link_names_a_pr_as_an_address_a_reference_or_a_number() {
    assert_eq!(parse_pr_ref("https://github.com/AgentMuxAI/agentmux/pull/5248", ""), pr("agentmuxai/agentmux", 5248));
    assert_eq!(parse_pr_ref("https://github.com/o/r/pull/12/files?x=1", ""), pr("o/r", 12));
    assert_eq!(parse_pr_ref("github.com/o/r/pull/12#discussion", ""), pr("o/r", 12));
    assert_eq!(parse_pr_ref("o/r#7", "x/y"), pr("o/r", 7));
    assert_eq!(parse_pr_ref("#7", "o/r"), pr("o/r", 7));
    assert_eq!(parse_pr_ref(" 7 ", "o/r"), pr("o/r", 7));
    assert_eq!(parse_pr_ref("#7", "local:C:/work/x"), None, "a folder isn't a GitHub repository");
    assert_eq!(parse_pr_ref("#7", ""), None);
    assert_eq!(parse_pr_ref("https://github.com/o/r/issues/7", ""), None, "an issue isn't a PR");
    assert_eq!(parse_pr_ref("feature/x", "o/r"), None);
    assert_eq!(parse_pr_ref("#0", "o/r"), None);
}

#[test]
fn a_result_links_the_one_pr_it_names() {
    assert_eq!(pr_in_text("Opened PR #5248, waiting on review.", "o/r"), pr("o/r", 5248));
    assert_eq!(pr_in_text("Done in https://github.com/o/r/pull/9.", "x/y"), pr("o/r", 9));
    assert_eq!(pr_in_text("pr 12 merged; PR #12 again", "o/r"), pr("o/r", 12), "the same PR twice is one");
    assert_eq!(pr_in_text("PR #1 and PR #2", "o/r"), None, "two PRs: which one is unclear");
    assert_eq!(pr_in_text("fixed issue #12", "o/r"), None, "an issue number isn't a PR");
    assert_eq!(pr_in_text("SPR #12", "o/r"), None);
    assert_eq!(pr_in_text("PR #12", ""), None, "no repository to find it in");
}

#[test]
fn githubs_answer_reads_as_open_merged_or_closed() {
    use serde_json::json;
    assert_eq!(pr_state(&json!({"state": "open", "merged_at": null})), Some(PrState::Open));
    assert_eq!(pr_state(&json!({"state": "closed", "merged_at": "2026-10-20T22:27:20Z"})), Some(PrState::Merged));
    assert_eq!(pr_state(&json!({"state": "closed", "merged": true})), Some(PrState::Merged));
    assert_eq!(pr_state(&json!({"state": "closed", "merged_at": null})), Some(PrState::Closed));
    assert_eq!(pr_state(&json!({"message": "Not Found"})), None);
}

fn task(id: &str, state: &str, pr_url: &str, repo: &str) -> BoardTask {
    BoardTask {
        item: WorkItem { id: id.into(), state: state.into(), ..item(id) },
        card: WorkCard { pr_url: pr_url.into(), repo: repo.into(), ..Default::default() },
    }
}

#[test]
fn tasks_with_a_pr_are_checked_until_merged_and_not_too_often() {
    let merged = |mut t: BoardTask| {
        t.card.pr = Some(WorkPr { merged_at: Some(5), ..Default::default() });
        t
    };
    let tasks = [
        task("a", "review", "#1", "o/r"),
        task("b", "claimed", "#2", "o/r"),
        task("c", "review", "", "o/r"),
        task("d", "review", "#4", "p/private"),
        task("e", "review", "#5", "o/r"),
        merged(task("f", "done", "#6", "o/r")),
        task("g", "done", "#7", "o/r"),
        task("h", "open", "#8", "o/r"),
        merged(task("i", "claimed", "#9", "o/r")),
    ];
    let mut checked = Checked::default();
    checked.private_repos.insert("p/private".into(), 0);
    checked.at.insert("e".into(), (1000, Some(PrState::Open)));
    let ids = |v: Vec<(String, PrRef)>| v.into_iter().map(|(id, _)| id).collect::<Vec<_>>();
    assert_eq!(
        ids(due(&tasks, &checked, 60_000)),
        ["a", "b", "g"],
        "e was just checked; d's repository is private; f and i are known merged; h waits"
    );
    assert_eq!(ids(due(&tasks, &checked, 1000 + CHECK_EVERY_MS)), ["a", "b", "g", "e"], "never looked at first");
    checked.at.insert("e".into(), (1000, Some(PrState::Closed)));
    assert_eq!(ids(due(&tasks, &checked, 1000 + CHECK_EVERY_MS)), ["a", "b", "g"], "a closed PR is looked at less often");
    assert_eq!(ids(due(&tasks, &checked, PRIVATE_FOR_MS)), ["a", "b", "d", "g", "e"]);
    checked.at.insert("e".into(), (1000, Some(PrState::Missing)));
    assert_eq!(ids(due(&tasks, &checked, 1000 + CHECK_EVERY_MS)), ["a", "b", "g"], "a mistyped PR is looked at hourly");
}

#[test]
fn a_merge_is_in_the_first_release_made_at_or_after_it() {
    use serde_json::json;
    let releases = json!([
        {"tag_name": "v3", "draft": false, "created_at": "2026-10-24T10:00:00Z", "published_at": "2026-10-24T11:00:00Z"},
        {"tag_name": "draft", "draft": true, "created_at": "2026-10-23T20:00:00Z"},
        {"tag_name": "v2", "draft": false, "created_at": "2026-10-23T16:41:01Z", "published_at": "2026-10-23T17:35:12Z"},
        {"tag_name": "v1", "draft": false, "created_at": "2026-10-23T00:11:01Z", "published_at": "2026-10-23T01:04:40Z"}
    ]);
    let ms = |s: &str| chrono::DateTime::parse_from_rfc3339(s).unwrap().timestamp_millis();
    assert_eq!(
        release_containing(&releases, ms("2026-10-23T12:00:00Z")),
        Some(("v2".to_string(), ms("2026-10-23T17:35:12Z")))
    );
    assert_eq!(
        release_containing(&releases, ms("2026-10-23T18:00:00Z")).map(|r| r.0),
        Some("v3".to_string()),
        "the draft doesn't count"
    );
    assert_eq!(release_containing(&releases, ms("2026-10-24T12:00:00Z")), None, "not released yet");
    assert_eq!(merged_at(&json!({"merged_at": "2026-10-23T12:00:00Z"})), Some(ms("2026-10-23T12:00:00Z")));
    assert_eq!(merged_at(&json!({"merged_at": null})), None);
}

#[test]
fn releases_are_asked_about_per_repository_for_merged_unreleased_tasks() {
    let with_pr = |id: &str, repo: &str, merged: Option<i64>, released: &str| {
        let mut t = task(id, "done", "#1", repo);
        t.card.pr = Some(WorkPr { merged_at: merged, released_in: released.into(), released_at: None });
        t
    };
    let tasks = [
        with_pr("a", "o/r", Some(10), ""),
        with_pr("b", "o/r", Some(20), ""),
        with_pr("c", "o/r", Some(5), "v1"),
        with_pr("d", "o/s", None, ""),
        with_pr("e", "o/s", Some(30), ""),
    ];
    let mut checked = Checked::default();
    let mut by_repo: Vec<(String, Vec<String>)> = awaiting_release(&tasks, &checked, 1000)
        .into_iter()
        .map(|(r, v)| (r, v.into_iter().map(|(id, _)| id).collect()))
        .collect();
    by_repo.sort();
    assert_eq!(by_repo, [("o/r".to_string(), vec!["a".to_string(), "b".to_string()]), ("o/s".to_string(), vec!["e".to_string()])]);
    checked.releases_at.insert("o/r".into(), 1000);
    assert_eq!(awaiting_release(&tasks, &checked, 2000).keys().collect::<Vec<_>>(), ["o/s"], "o/r was just asked");
}

#[test]
fn a_named_pr_is_the_tasks_only_when_it_is_from_the_agents_branch() {
    use serde_json::json;
    let pr = json!({"number": 5200, "head": {"ref": "loap-3/x", "repo": {"full_name": "AgentMuxAI/agentmux"}}});
    assert!(is_from_branch(&pr, "agentmuxai/agentmux", "loap-3/x"));
    assert!(!is_from_branch(&pr, "agentmuxai/agentmux", "loap-3/other-work"), "a PR the result only mentions");
    assert!(!is_from_branch(&pr, "bob/agentmux", "loap-3/x"), "a fork's same-named branch");
    assert!(!is_from_branch(&pr, "agentmuxai/agentmux", ""), "no branch known: nothing linked");
    assert!(!is_from_branch(&json!({"number": 1, "head": {"ref": "x"}}), "o/r", "x"), "no head repository: nothing linked");
}

#[test]
fn the_budget_stops_at_the_hourly_limit_and_while_github_says_wait() {
    let b = Budget { calls: Mutex::new(Vec::new()), wait_until: AtomicI64::new(0) };
    for _ in 0..HOURLY_BUDGET {
        assert!(b.take(1000));
    }
    assert!(!b.take(2000), "the hour's calls are spent");
    assert!(b.take(1000 + 3_600_000), "an hour later they are back");
    b.wait_until.store(10_000_000, Ordering::Relaxed);
    assert!(!b.take(9_000_000));
}


fn item(id: &str) -> WorkItem {
    WorkItem {
        id: id.into(),
        title: format!("task {id}"),
        payload: "do it".into(),
        kind: String::new(),
        target_agent: String::new(),
        target_group: String::new(),
        priority: 0,
        state: String::new(),
        claimed_by: String::new(),
        target_agent_uid: String::new(),
        claimed_by_uid: String::new(),
        claim_expires: None,
        attempts: 0,
        max_attempts: 3,
        created_by: "tester".into(),
        created_by_uid: String::new(),
        created_at: 1000,
        updated_at: 1000,
        not_before: None,
        result: String::new(),
    }
}

#[test]
fn a_merged_pr_accepts_its_task_once_and_only_from_review() {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open_identity_store(&dir.path().join("identity-store.db")).unwrap();
    s.work_queue_enqueue(&item("t")).unwrap();
    s.work_card_ensure("t", 1000).unwrap();
    let pr = PrRef { repo: "o/r".into(), number: 12 };
    let f = ClaimFilter { kind: None, agent_id: "Loap-3".into(), agent_uid: String::new(), groups: vec![] };
    let c = s.work_queue_claim(&f, 2000, 60_000).unwrap().unwrap();
    assert!(!close_merged(&s, "t", &pr, 3000), "still in progress: not closed");

    assert!(link(&s, "t", &pr, 3000));
    assert!(!link(&s, "t", &PrRef { repo: "o/r".into(), number: 13 }, 3100), "an existing link is kept");
    assert_eq!(s.work_board_get("t").unwrap().unwrap().card.pr_url, "https://github.com/o/r/pull/12");

    s.work_submit("t", "Loap-3", "", c.attempts, "PR #12", 4000).unwrap().unwrap();
    assert!(close_merged(&s, "t", &pr, 5000));
    assert_eq!(s.work_queue_get("t").unwrap().unwrap().state, work_state::DONE);
    assert!(!close_merged(&s, "t", &pr, 6000), "another srv got there first: nothing twice");
    let events = s.work_events("t", 50).unwrap();
    let accepted: Vec<_> = events.iter().filter(|e| e.kind == "accepted").collect();
    assert_eq!(accepted.len(), 1);
    assert_eq!((accepted[0].actor.as_str(), accepted[0].body.as_str()), ("GitHub", "PR #12 merged"));
}

#[test]
fn closing_on_merge_is_on_unless_turned_off() {
    assert!(close_on_merge_enabled(&HashMap::new()));
    let off = HashMap::from([(SETTING_CLOSE_ON_MERGE.to_string(), serde_json::json!(false))]);
    assert!(!close_on_merge_enabled(&off));
}

#[test]
fn a_just_opened_pr_is_the_one_created_since_the_command_not_an_older_one() {
    use serde_json::json;
    let seen = chrono::DateTime::parse_from_rfc3339("2026-10-21T05:00:00Z").unwrap().timestamp_millis();
    let list = json!([
        {"number": 90, "created_at": "2026-09-01T10:00:00Z", "head": {"ref": "x"}},
        {"number": 120, "created_at": "2026-10-21T05:00:20Z", "head": {"ref": "x"}},
    ]);
    assert_eq!(pick_opened(&list, None, seen), Some(120), "the merged PR of last month isn't this command's");
    assert_eq!(pick_opened(&json!([list[0]]), None, seen), None);
    let skewed = json!([{"number": 7, "created_at": "2026-10-21T04:58:00Z", "head": {"ref": "x"}}]);
    assert_eq!(pick_opened(&skewed, None, seen), Some(7), "a clock a little behind is allowed for");
    let newest = json!([
        {"number": 130, "created_at": "2026-10-21T05:01:00Z", "head": {"ref": "someone-else"}},
        {"number": 129, "created_at": "2026-10-21T05:00:30Z", "head": {"ref": "x"}},
    ]);
    assert_eq!(pick_opened(&newest, Some("x"), seen), Some(129), "a fork's PR found by its branch among the newest");
    assert_eq!(pick_opened(&json!({"message": "x"}), None, seen), None);
}

/// The loop's own list holds a task its agent completed (done) within the
/// board's window, so its merge and release are still followed; not one
/// finished before it.
#[test]
fn the_watch_list_follows_finished_tasks_for_the_boards_window() {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open_identity_store(&dir.path().join("identity-store.db")).unwrap();
    let now = 10 * super::super::FINISHED_WINDOW_MS;
    for (id, updated) in [("recent", now - 1000), ("old", now - super::super::FINISHED_WINDOW_MS - 1000)] {
        let mut w = item(id);
        w.created_at = updated;
        w.updated_at = updated;
        s.work_queue_enqueue(&w).unwrap();
        s.work_card_ensure(id, updated).unwrap();
        s.work_card_update(id, &CardUpdate { pr_url: Some("https://github.com/o/r/pull/1".into()), ..Default::default() }, updated)
            .unwrap();
        let conn = s.conn().lock().unwrap();
        conn.execute("UPDATE db_work_queue SET state = 'done', updated_at = ?2 WHERE id = ?1", rusqlite::params![id, updated])
            .unwrap();
    }
    let ids: Vec<String> = watch_list(&s, now).unwrap().into_iter().map(|t| t.item.id).collect();
    assert_eq!(ids, ["recent"]);
    let tasks = watch_list(&s, now).unwrap();
    assert!(watched(&tasks[0]), "a finished task whose merge isn't known yet is watched");
}
