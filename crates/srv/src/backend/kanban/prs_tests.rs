// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::storage::work_board::WorkCard;
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
fn only_tasks_in_review_with_a_pr_are_checked_and_not_too_often() {
    let tasks = [
        task("a", "review", "#1", "o/r"),
        task("b", "claimed", "#2", "o/r"),
        task("c", "review", "", "o/r"),
        task("d", "review", "#4", "p/private"),
        task("e", "review", "#5", "o/r"),
    ];
    let mut checked = Checked::default();
    checked.private_repos.insert("p/private".into(), 0);
    checked.at.insert("e".into(), (1000, Some(PrState::Open)));
    let ids = |v: Vec<(String, PrRef)>| v.into_iter().map(|(id, _)| id).collect::<Vec<_>>();
    assert_eq!(ids(due(&tasks, &checked, 60_000)), ["a"], "e was just checked; d's repository is private");
    assert_eq!(ids(due(&tasks, &checked, 1000 + CHECK_EVERY_MS)), ["a", "e"], "never looked at first");
    checked.at.insert("e".into(), (1000, Some(PrState::Closed)));
    assert_eq!(ids(due(&tasks, &checked, 1000 + CHECK_EVERY_MS)), ["a"], "a closed PR is looked at less often");
    assert_eq!(ids(due(&tasks, &checked, PRIVATE_FOR_MS)), ["a", "d", "e"]);
    checked.at.insert("e".into(), (1000, Some(PrState::Missing)));
    assert_eq!(ids(due(&tasks, &checked, 1000 + CHECK_EVERY_MS)), ["a"], "a mistyped PR is looked at hourly, its repository still checked");
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
