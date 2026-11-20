// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Mutex;

use super::*;
use crate::backend::storage::background_tasks::BackgroundTaskStatus;

const NOW: i64 = 1_000_000_000;

struct Fake<F: Fn(&str, u32) -> Verdict + Send + Sync> {
    answer: F,
    asked: Mutex<Vec<(String, u32, i64)>>,
}

impl<F: Fn(&str, u32) -> Verdict + Send + Sync> ContainerProbe for Fake<F> {
    fn probe<'a>(&'a self, container: &'a str, pid: u32, task_started_ms: i64) -> BoxFuture<'a, Verdict> {
        self.asked.lock().unwrap().push((container.to_string(), pid, task_started_ms));
        Box::pin(async move { (self.answer)(container, pid) })
    }
}

fn fake<F: Fn(&str, u32) -> Verdict + Send + Sync>(answer: F) -> Fake<F> {
    Fake { answer, asked: Mutex::new(Vec::new()) }
}

fn store() -> Arc<Store> {
    Arc::new(Store::open(std::path::Path::new(":memory:")).unwrap())
}

/// A running row for the call `id` of `block`, whose process the container reported as `pid`.
fn task(s: &Store, block: &str, id: &str, container: &str, pid: Option<i64>) {
    s.background_task_observe(id, block, "Bash", NOW - 5_000, NOW - 1_000).unwrap();
    s.background_container_call_file(block, id, container, pid, None, NOW - 4_000).unwrap();
}

fn status(s: &Store, id: &str) -> (BackgroundTaskStatus, Option<i64>) {
    let t = s.background_task_get(id).unwrap().unwrap();
    (t.status, t.ended_at_ms)
}

#[test]
fn the_printed_answer_is_alive_gone_or_nothing_readable() {
    assert_eq!(verdict_of("gone\n", NOW), Verdict::Gone);
    assert_eq!(verdict_of("alive\n", NOW), Verdict::Alive, "no start time: alive on kill -0 alone");
    assert_eq!(verdict_of("noise\nalive 999000\n", NOW), Verdict::Alive, "started before the task");
    let after = (NOW / 1000) + 3_600;
    assert_eq!(verdict_of(&format!("alive {after}\n"), NOW), Verdict::Gone, "a newer process has the pid");
    let within = (NOW / 1000) + 30;
    assert_eq!(verdict_of(&format!("alive {within}"), NOW), Verdict::Alive, "inside the slack: the pid is reported as the command starts");
    assert_eq!(verdict_of("", NOW), Verdict::Unknown);
    assert_eq!(verdict_of("sh: getconf: not found\n", NOW), Verdict::Unknown);
    assert_eq!(verdict_of("alive x\n", NOW), Verdict::Alive, "an unreadable start time is not evidence of a new process");
}

#[tokio::test]
async fn a_container_row_with_a_live_process_stays_and_one_whose_process_is_gone_ends() {
    let s = store();
    task(&s, "b1", "dead", "agentmux-a", Some(111));
    task(&s, "b2", "live", "agentmux-a", Some(222));
    let probe = fake(|_, pid| if pid == 222 { Verdict::Alive } else { Verdict::Gone });
    let changed = sweep(&s, NOW, &probe).await;
    assert_eq!(changed.into_iter().collect::<Vec<_>>(), vec!["b1".to_string()]);
    assert_eq!(status(&s, "dead"), (BackgroundTaskStatus::Stopped, Some(NOW)));
    assert_eq!(status(&s, "live"), (BackgroundTaskStatus::Running, None));
    let asked = probe.asked.lock().unwrap();
    assert!(asked.contains(&("agentmux-a".to_string(), 111, NOW - 5_000)), "asked in the pane's own container, with the task's start: {asked:?}");
}

#[tokio::test]
async fn a_probe_that_cannot_say_ends_nothing_and_a_row_with_no_container_pid_is_not_probed() {
    let s = store();
    task(&s, "b1", "unsure", "agentmux-a", Some(5));
    task(&s, "b1", "nopid", "agentmux-a", None);
    let probe = fake(|_, _| Verdict::Unknown);
    assert!(sweep(&s, NOW, &probe).await.is_empty());
    assert_eq!(status(&s, "unsure").0, BackgroundTaskStatus::Running);
    assert_eq!(probe.asked.lock().unwrap().len(), 1, "only the row with a pid is checked");
    // A pid that cannot be one is never sent to the container.
    task(&s, "b2", "zero", "agentmux-a", Some(0));
    task(&s, "b2", "negative", "agentmux-a", Some(-4));
    sweep(&s, NOW, &probe).await;
    assert_eq!(probe.asked.lock().unwrap().len(), 2, "one more ask, for the first row again");
}

#[tokio::test]
async fn a_container_that_is_not_running_ends_every_row_of_its_panes_at_once() {
    let s = store();
    task(&s, "b1", "one", "agentmux-down", Some(10));
    task(&s, "b1", "two", "agentmux-down", Some(11));
    task(&s, "b1", "no-pid", "agentmux-down", None);
    task(&s, "b3", "other", "agentmux-up", Some(12));
    let probe = fake(|container, _| if container == "agentmux-down" { Verdict::ContainerDown } else { Verdict::Alive });
    let changed = sweep(&s, NOW, &probe).await;
    assert_eq!(changed.into_iter().collect::<Vec<_>>(), vec!["b1".to_string()]);
    for id in ["one", "two", "no-pid"] {
        assert_eq!(status(&s, id), (BackgroundTaskStatus::Stopped, Some(NOW - 1_000)), "{id}: ended at its last sighting");
    }
    assert_eq!(status(&s, "other").0, BackgroundTaskStatus::Running, "another container's row stays");
    assert_eq!(probe.asked.lock().unwrap().iter().filter(|(c, ..)| c == "agentmux-down").count(), 1, "one ask finds the container down");
}

#[tokio::test]
async fn a_row_that_ended_meanwhile_is_not_touched_and_is_not_reported() {
    let s = store();
    task(&s, "b1", "done", "agentmux-a", Some(111));
    s.background_task_complete("done", BackgroundTaskStatus::Done, NOW - 500).unwrap();
    let probe = fake(|_, _| Verdict::Gone);
    assert!(sweep(&s, NOW, &probe).await.is_empty());
    assert_eq!(status(&s, "done"), (BackgroundTaskStatus::Done, Some(NOW - 500)));
}

#[test]
fn a_description_filed_before_or_after_the_row_labels_the_row_but_never_replaces_the_clis() {
    let s = store();
    // Before: the row is made later and finds it.
    s.background_container_call_file("b1", "early", "agentmux-a", Some(1), Some("Run the migrations"), NOW).unwrap();
    s.background_task_observe("early", "b1", "Bash", NOW, NOW).unwrap();
    assert_eq!(s.background_task_get("early").unwrap().unwrap().label, "Run the migrations");
    // After: the report finds a row that still has the generic label.
    s.background_task_observe("late", "b1", "Bash", NOW, NOW).unwrap();
    assert!(s.background_task_describe_if_generic("b1", "late", "Watch the tests").unwrap());
    assert_eq!(s.background_task_get("late").unwrap().unwrap().label, "Watch the tests");
    // The CLI's own description stays, and another pane's row with the same call id is not touched.
    s.background_task_observe("cli", "b1", "task dev", NOW, NOW).unwrap();
    assert!(!s.background_task_describe_if_generic("b1", "cli", "Something else").unwrap());
    s.background_task_observe("same-id", "b2", "Bash", NOW, NOW).unwrap();
    assert!(!s.background_task_describe_if_generic("b1", "same-id", "Not yours").unwrap());
    assert_eq!(s.background_task_get("same-id").unwrap().unwrap().label, "Bash");
}
