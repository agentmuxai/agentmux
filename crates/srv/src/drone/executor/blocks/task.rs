// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Task block: files a Kanban task for an agent, or for the backlog
//! (the Drone real-world jobs spec §4.2).
//!
//! Block config (`node.data`):
//!   * `title`   required. The graph's words; each `{{...}}` value is quoted data (§6.2).
//!   * `body`    optional, the task's text, quoted the same way.
//!   * `target`  optional, the agent's id; empty is the backlog. Written in the graph, never a
//!               `{{reference}}`: a value from outside must not pick who gets the work (§6.2).
//!   * `dedupe`  optional, "one task per incident": a second run with the same key adds a line to
//!               the open task instead of filing another. The key may be built from a reference
//!               (it only finds a task; it names no one). It is kept per node and target: another
//!               node, or the same node with another target, with the same text has its own task.
//!
//! Output: `{ "filed": bool, "task": <number>, "work_id": "<id>" }`; `filed` is false when the
//! run added a line to an open task with the same key.
//!
//! A task this block files carries the drone's tag (`created_by_uid`, `drone:<id>`), which is what
//! the note mode below finds it by.
//!
//! Note mode: with `note_task` in the config the block adds a line to a task instead of filing one.
//!   * `note_task`  the task's number (`12` or `#12`), which may be a `{{reference}}`: it only finds a
//!                  task, and the block adds to a task **this drone filed** and to no other (a task
//!                  filed by another drone, an agent or a person is refused, whatever number is given).
//!   * `note`       required, the line's text; each `{{...}}` value is quoted data (§6.2).
//!
//! Output: `{ "updated": true, "task": <number>, "work_id": "<id>" }`; any refusal fails the block.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::backend::storage::work_board::WorkEvent;
use crate::backend::storage::work_queue::{work_state, WorkItem};
use crate::drone::data_flow::ExecutionScope;
use crate::drone::executor::blocks::message::{quoted_text, strip_standing};
use crate::drone::types::FlowNode;

/// Who the task's history says filed it.
const FILED_BY: &str = "drone";
/// The tasks' `kind`; the dedupe looks only at these.
const TASK_KIND: &str = "drone";
/// The key line that opens a deduplicated task's text.
const KEY_PREFIX: &str = "Drone key: ";
/// The states in which a task is still open for the dedupe: it is not done, failed or cancelled.
const OPEN_STATES: [&str; 5] = [work_state::OPEN, work_state::CLAIMED, work_state::HELD, work_state::BLOCKED, work_state::REVIEW];
/// Held from looking for an open task with the key to filing one, so two runs with the same key at
/// once can't both find none. Never held across an await.
static DEDUPE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// What the block files.
#[derive(Debug, PartialEq)]
pub(crate) struct TaskSpec {
    pub title: String,
    pub body: String,
    pub target: String,
    /// The hashed dedupe key, when the node has one.
    pub key: Option<String>,
}

pub async fn run(node: &FlowNode, scope: &ExecutionScope) -> Result<Value, String> {
    if is_note(node) {
        let spec = compose_note(node, scope)?;
        let state = crate::drone::host::state()?;
        return add_note(&crate::server::cron::work_store(state), &scope.drone_id, &spec);
    }
    let spec = compose(node, scope)?;
    let state = crate::drone::host::state()?;
    file_or_repeat(state, &spec, &scope.drone_id)
}

/// Whether the node adds a note instead of filing a task: it has a `note_task` with something in it (the editor
/// leaves an empty one behind when the field is cleared).
fn is_note(node: &FlowNode) -> bool {
    match node.data.get("note_task") {
        None | Some(Value::Null) => false,
        Some(Value::String(s)) => !s.trim().is_empty(),
        Some(_) => true,
    }
}

/// The tag a task carries when the drone `drone_id` filed it. A scope outside a saved drone has none.
fn owner_tag(drone_id: &str) -> String {
    if drone_id.is_empty() {
        String::new()
    } else {
        format!("drone:{drone_id}")
    }
}

/// The note a block adds to a task its drone filed.
#[derive(Debug, PartialEq)]
pub(crate) struct NoteSpec {
    pub num: i64,
    pub text: String,
}

/// The note the node describes. Pure, so the number and the quoting are testable without a server.
pub(crate) fn compose_note(node: &FlowNode, scope: &ExecutionScope) -> Result<NoteSpec, String> {
    let raw = match node.data.get("note_task") {
        Some(Value::String(t)) => scope.resolve(t),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    };
    let digits = raw.trim().trim_start_matches('#');
    let num: i64 = digits
        .parse()
        .ok()
        .filter(|n| *n > 0 && digits.bytes().all(|b| b.is_ascii_digit()))
        .ok_or_else(|| "task block: `note_task` is a task number (12 or #12)".to_string())?;
    let note_t = node.data.get("note").and_then(|v| v.as_str()).unwrap_or("");
    if note_t.trim().is_empty() {
        return Err("task block missing `note`".to_string());
    }
    Ok(NoteSpec { num, text: quoted_text(note_t, scope) })
}

/// Add the note to the task, which must be one this drone filed. Every refusal is an error, so a graph that tries to
/// write on another's task fails at that block instead of carrying on.
fn add_note(store: &crate::backend::storage::store::Store, drone_id: &str, spec: &NoteSpec) -> Result<Value, String> {
    let tag = owner_tag(drone_id);
    if tag.is_empty() {
        return Err("task block: a note is added only by a run of a saved drone".to_string());
    }
    let found = store.work_board_get(&format!("#{}", spec.num)).map_err(|e| format!("task block: {e}"))?;
    let Some(task) = found else {
        return Err(format!("task block: there is no task #{}", spec.num));
    };
    if task.item.kind != TASK_KIND || task.item.created_by_uid != tag {
        return Err(format!("task block: task #{} was not filed by this drone, so it takes no note from it", spec.num));
    }
    let line = WorkEvent {
        work_id: task.item.id.clone(),
        at: agentmux_common::time::now_ms(),
        actor: FILED_BY.to_string(),
        actor_kind: "system".to_string(),
        kind: "updated".to_string(),
        body: crate::server::work_tasks::clean(&spec.text),
        ..Default::default()
    };
    store.work_event_add(&line).map_err(|e| format!("task block: {e}"))?;
    Ok(json!({ "updated": true, "task": spec.num, "work_id": task.item.id }))
}

/// File the task, or add a line to the open one with the same key. Synchronous end to end, so the
/// lock covers the lookup and the insert and is never held across an await.
fn file_or_repeat(state: &crate::server::AppState, spec: &TaskSpec, drone_id: &str) -> Result<Value, String> {
    let store = crate::server::cron::work_store(state);
    let _one_at_a_time = DEDUPE_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

    if let Some(key) = &spec.key {
        let found = store.work_queue_find_open_with_prefix(TASK_KIND, &format!("{KEY_PREFIX}{key}")).map_err(|e| format!("task block: {e}"))?;
        if let Some(open) = found.as_ref().and_then(|t| find_open(std::slice::from_ref(t), key)) {
            let now = agentmux_common::time::now_ms();
            let num = store.work_card_ensure(&open.id, now).map_err(|e| format!("task block: {e}"))?;
            let line = WorkEvent {
                work_id: open.id.clone(),
                at: now,
                actor: FILED_BY.to_string(),
                actor_kind: "system".to_string(),
                kind: "updated".to_string(),
                body: crate::server::work_tasks::clean(&line_for_repeat(spec)),
                ..Default::default()
            };
            store.work_event_add(&line).map_err(|e| format!("task block: {e}"))?;
            return Ok(json!({ "filed": false, "task": num, "work_id": open.id }));
        }
    }

    let target = crate::server::job_actions::target_of(state, &spec.target);
    let text = match &spec.key {
        Some(key) => format!("{KEY_PREFIX}{key}\n{}", spec.body),
        None => spec.body.clone(),
    };
    let (work_id, num) = crate::server::job_actions::file_task_by(&store, FILED_BY, &owner_tag(drone_id), TASK_KIND, &spec.title, &text, target, "Filed by a drone.")
        .map_err(|e| format!("task block: couldn't file the task: {e}"))?;
    Ok(json!({ "filed": true, "task": num, "work_id": work_id }))
}

/// The task the node describes. Pure, so the quoting and the rules are testable without a server.
pub(crate) fn compose(node: &FlowNode, scope: &ExecutionScope) -> Result<TaskSpec, String> {
    let title_t = node.data.get("title").and_then(|v| v.as_str()).map(str::trim).unwrap_or("");
    if title_t.is_empty() {
        return Err("task block missing `title`".to_string());
    }
    let target = node.data.get("target").and_then(|v| v.as_str()).map(str::trim).unwrap_or("");
    if target.contains("{{") {
        return Err("task block: `target` must name an agent, not a {{reference}}".to_string());
    }
    let body_t = node.data.get("body").and_then(|v| v.as_str()).unwrap_or("");
    // The title is one line: the standing line belongs to the body, so a title with a value in it gets the value quoted and nothing else.
    let title = if title_t.contains("{{") {
        strip_standing(&quoted_text(title_t, scope)).to_string()
    } else {
        title_t.to_string()
    };
    let body = if body_t.trim().is_empty() { String::new() } else { quoted_text(body_t, scope) };
    let key = node
        .data
        .get("dedupe")
        .and_then(|v| v.as_str())
        .map(|t| scope.resolve(t))
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .map(|k| hash_key(&scope.drone_id, &node.id, target, &k));
    Ok(TaskSpec { title, body, target: target.to_string(), key })
}

/// A short stable hash of the key with the drone, the node and the target, so the stored line carries
/// no text from outside and another drone, another node or another target doesn't share the task
/// (node ids are only unique inside one graph).
fn hash_key(drone_id: &str, node_id: &str, target: &str, key: &str) -> String {
    let mut h = Sha256::new();
    for part in [drone_id, node_id, target, key] {
        h.update((part.len() as u64).to_le_bytes());
        h.update(part.as_bytes());
    }
    hex::encode(&h.finalize()[..8])
}

/// The open task that carries `key`, if there is one.
pub(crate) fn find_open<'a>(items: &'a [WorkItem], key: &str) -> Option<&'a WorkItem> {
    let want = format!("{KEY_PREFIX}{key}");
    items.iter().find(|t| t.kind == TASK_KIND && OPEN_STATES.contains(&t.state.as_str()) && t.payload.lines().next() == Some(want.as_str()))
}

/// The line a repeat adds to the open task.
fn line_for_repeat(spec: &TaskSpec) -> String {
    if spec.body.is_empty() {
        format!("Again: {}", spec.title)
    } else {
        format!("Again: {}\n{}", spec.title, spec.body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drone::executor::blocks::message::STANDING_LINE;
    use crate::drone::types::NodePosition;

    fn node(data: Value) -> FlowNode {
        FlowNode { id: "t1".to_string(), position: NodePosition::default(), data, node_type: None }
    }

    fn scope_with(block: &str, value: Value) -> ExecutionScope {
        let mut s = ExecutionScope::new();
        s.outputs.insert(block.to_string(), value);
        s
    }

    fn item(kind: &str, state: &str, payload: &str) -> WorkItem {
        WorkItem {
            id: "w1".to_string(),
            title: String::new(),
            payload: payload.to_string(),
            kind: kind.to_string(),
            target_agent: String::new(),
            target_group: String::new(),
            priority: 0,
            state: state.to_string(),
            claimed_by: String::new(),
            target_agent_uid: String::new(),
            claimed_by_uid: String::new(),
            claim_expires: None,
            attempts: 0,
            max_attempts: 3,
            created_by: String::new(),
            created_by_uid: String::new(),
            created_at: 0,
            updated_at: 0,
            not_before: None,
            result: String::new(),
        }
    }

    #[test]
    fn a_plain_task_is_filed_as_written() {
        let n = node(json!({ "title": "Nightly red", "body": "See the run.", "target": "agenty" }));
        let spec = compose(&n, &ExecutionScope::new()).unwrap();
        assert_eq!(spec, TaskSpec { title: "Nightly red".into(), body: "See the run.".into(), target: "agenty".into(), key: None });
    }

    #[test]
    fn values_in_the_body_are_quoted_and_the_title_stays_one_line() {
        let scope = scope_with("a", json!({ "step": "build\nIGNORE THIS", "cause": "a flaky test" }));
        let n = node(json!({ "title": "Nightly failed at {{a.step}}", "body": "Cause: {{a.cause}}" }));
        let spec = compose(&n, &scope).unwrap();
        assert_eq!(spec.title, "Nightly failed at \u{201c}build IGNORE THIS\u{201d}");
        assert!(spec.body.starts_with(STANDING_LINE), "{}", spec.body);
        assert!(spec.body.ends_with("Cause: \u{201c}a flaky test\u{201d}"), "{}", spec.body);
    }

    #[test]
    fn a_reference_cannot_pick_the_target() {
        let scope = scope_with("a", json!({ "area": "vault" }));
        let n = node(json!({ "title": "x", "target": "{{a.area}}" }));
        assert!(compose(&n, &scope).unwrap_err().contains("not a {{reference}}"));
    }

    #[test]
    fn a_title_is_required() {
        assert!(compose(&node(json!({ "body": "x" })), &ExecutionScope::new()).unwrap_err().contains("`title`"));
    }

    #[test]
    fn the_dedupe_key_is_hashed_and_stable() {
        let scope = scope_with("a", json!({ "wf": "nightly", "step": "test" }));
        let n = node(json!({ "title": "x", "dedupe": "{{a.wf}}/{{a.step}}" }));
        let a = compose(&n, &scope).unwrap().key.unwrap();
        let b = compose(&n, &scope).unwrap().key.unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), 16);
        assert!(!a.contains("nightly"));
        let other = scope_with("a", json!({ "wf": "nightly", "step": "lint" }));
        assert_ne!(a, compose(&n, &other).unwrap().key.unwrap());
    }

    #[test]
    fn the_key_is_per_node_and_per_target() {
        let scope = scope_with("a", json!({ "wf": "nightly" }));
        let key = |id: &str, target: &str| {
            let mut n = node(json!({ "title": "x", "target": target, "dedupe": "{{a.wf}}" }));
            n.id = id.to_string();
            compose(&n, &scope).unwrap().key.unwrap()
        };
        assert_eq!(key("t1", "agenty"), key("t1", "agenty"));
        assert_ne!(key("t1", "agenty"), key("t2", "agenty"), "another node has its own task");
        assert_ne!(key("t1", "agenty"), key("t1", "loap"), "another target has its own task");
    }

    #[test]
    fn the_key_is_per_drone_when_two_drones_share_a_node_id_target_and_key_text() {
        let n = node(json!({ "title": "x", "target": "agenty", "dedupe": "nightly" }));
        let key = |drone: &str| {
            let mut scope = ExecutionScope::for_drone(drone, Value::Null);
            scope.outputs.insert("a".to_string(), json!({}));
            compose(&n, &scope).unwrap().key.unwrap()
        };
        assert_eq!(key("drone-1"), key("drone-1"), "one drone keeps finding its own task");
        assert_ne!(key("drone-1"), key("drone-2"), "another drone has its own task");
        assert_ne!(key("drone-1"), key(""), "a scope outside a run is not any drone's");
    }

    #[test]
    fn a_title_with_a_value_keeps_its_own_lines() {
        let scope = scope_with("a", json!({ "step": "build" }));
        let n = node(json!({ "title": "Nightly red\nat {{a.step}}" }));
        let spec = compose(&n, &scope).unwrap();
        assert_eq!(spec.title, "Nightly red\nat \u{201c}build\u{201d}");
    }

    #[test]
    fn the_store_finds_the_open_task_with_the_key_however_many_others_there_are() {
        use crate::backend::storage::store::Store;
        // The work queue lives in the identity store, as the jobs tests use it.
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_identity_store(&dir.path().join("identity-store.db")).unwrap();
        let mine = |key: &str| format!("{KEY_PREFIX}{key}\nbody");
        let file = |kind: &str, text: &str| {
            crate::server::job_actions::file_task_as(&store, "drone", kind, "t", text, (String::new(), String::new()), "test").unwrap()
        };
        let (first_id, _) = file(TASK_KIND, &mine("aaaa"));
        // Far more newer tasks than any scan window, of other keys and other kinds.
        for i in 0..600 {
            file(TASK_KIND, &mine(&format!("other{i}")));
            file("job", &mine("aaaa"));
        }
        let found = store.work_queue_find_open_with_prefix(TASK_KIND, &format!("{KEY_PREFIX}aaaa")).unwrap().expect("found");
        assert_eq!(found.id, first_id);
        assert!(store.work_queue_find_open_with_prefix(TASK_KIND, &format!("{KEY_PREFIX}zzzz")).unwrap().is_none());

        store.work_queue_cancel(&first_id, "done with it", 1).unwrap();
        assert!(store.work_queue_find_open_with_prefix(TASK_KIND, &format!("{KEY_PREFIX}aaaa")).unwrap().is_none(), "a cancelled task is not reopened");
    }

    #[test]
    fn a_blank_dedupe_means_none() {
        let n = node(json!({ "title": "x", "dedupe": "  " }));
        assert_eq!(compose(&n, &ExecutionScope::new()).unwrap().key, None);
    }

    #[test]
    fn find_open_matches_the_key_on_an_open_drone_task_only() {
        let key = "abc123";
        let text = format!("{KEY_PREFIX}{key}\nbody");
        assert!(find_open(&[item("drone", "open", &text)], key).is_some());
        assert!(find_open(&[item("drone", "claimed", &text)], key).is_some());
        assert!(find_open(&[item("drone", "done", &text)], key).is_none(), "a finished task is not reopened");
        assert!(find_open(&[item("job", "open", &text)], key).is_none(), "other kinds are not ours");
        assert!(find_open(&[item("drone", "open", &format!("{KEY_PREFIX}other\nbody"))], key).is_none());
        assert!(find_open(&[item("drone", "open", &format!("body\n{KEY_PREFIX}{key}"))], key).is_none(), "the key line opens the text");
    }

    #[test]
    fn a_note_names_its_task_by_number_or_by_an_earlier_blocks_output_and_quotes_its_values() {
        let scope = scope_with("file", json!({ "task": 7, "cause": "a flaky test" }));
        let note = |data: Value| compose_note(&node(data), &scope);
        assert_eq!(note(json!({ "note_task": "12", "note": "x" })).unwrap().num, 12);
        assert_eq!(note(json!({ "note_task": " #12 ", "note": "x" })).unwrap().num, 12);
        assert_eq!(note(json!({ "note_task": "{{file.task}}", "note": "x" })).unwrap().num, 7);
        let quoted = note(json!({ "note_task": "7", "note": "Cause: {{file.cause}}" })).unwrap();
        assert!(quoted.text.starts_with(STANDING_LINE), "{}", quoted.text);
        assert!(quoted.text.ends_with("Cause: \u{201c}a flaky test\u{201d}"), "{}", quoted.text);
    }

    #[test]
    fn a_note_needs_a_task_number_and_a_text() {
        let scope = scope_with("file", json!({ "task": "abc" }));
        for bad in ["", "abc", "0", "-3", "1.5", "+5", "work#12", "12 13", "{{file.task}}", "{{file.nothing}}"] {
            let err = compose_note(&node(json!({ "note_task": bad, "note": "x" })), &scope).unwrap_err();
            assert!(err.contains("`note_task` is a task number"), "{bad:?}: {err}");
        }
        let err = compose_note(&node(json!({ "note_task": "12", "note": "  " })), &scope).unwrap_err();
        assert!(err.contains("`note`"), "{err}");
    }

    #[test]
    fn a_note_goes_only_to_a_task_this_drone_filed() {
        use crate::backend::storage::store::Store;
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_identity_store(&dir.path().join("identity-store.db")).unwrap();
        let file = |uid: &str, kind: &str| {
            crate::server::job_actions::file_task_by(&store, FILED_BY, uid, kind, "t", "body", (String::new(), String::new()), "test").unwrap()
        };
        let (mine_id, mine) = file(&owner_tag("d1"), TASK_KIND);
        let (_, other_drones) = file(&owner_tag("d2"), TASK_KIND);
        let (_, untagged) = file("", TASK_KIND);
        let (_, wrong_kind) = file(&owner_tag("d1"), "job");
        let note = |drone: &str, num: i64| add_note(&store, drone, &NoteSpec { num, text: "the run saw it again".to_string() });

        let done = note("d1", mine).unwrap();
        assert_eq!(done, json!({ "updated": true, "task": mine, "work_id": mine_id }));
        let history = store.work_events(&mine_id, 10).unwrap();
        assert_eq!(history.last().map(|e| e.body.as_str()), Some("the run saw it again"));
        assert_eq!(history.last().map(|e| (e.kind.as_str(), e.actor.as_str())), Some(("updated", FILED_BY)));

        for (what, num) in [("another drone's task", other_drones), ("an untagged task", untagged), ("a task of another kind", wrong_kind)] {
            let err = note("d1", num).unwrap_err();
            assert!(err.contains("was not filed by this drone"), "{what}: {err}");
        }
        assert!(note("d1", 9999).unwrap_err().contains("there is no task #9999"));
        assert!(note("", mine).unwrap_err().contains("only by a run of a saved drone"), "a scope with no drone adds no note, even to a tagged task's number");
    }

    #[test]
    fn a_node_is_a_note_only_when_its_note_task_has_something_in_it() {
        assert!(!is_note(&node(json!({ "title": "x" }))));
        assert!(!is_note(&node(json!({ "title": "x", "note_task": "" }))), "a cleared field is not a note");
        assert!(!is_note(&node(json!({ "title": "x", "note_task": "  " }))));
        assert!(!is_note(&node(json!({ "title": "x", "note_task": null }))));
        assert!(is_note(&node(json!({ "note_task": "12", "note": "x" }))));
        assert!(is_note(&node(json!({ "note_task": 12, "note": "x" }))));
        assert_eq!(compose_note(&node(json!({ "note_task": 12, "note": "x" })), &ExecutionScope::new()).unwrap().num, 12);
    }

    #[test]
    fn a_task_the_block_files_carries_its_drones_tag_and_no_other_does() {
        assert_eq!(owner_tag("d1"), "drone:d1");
        assert_eq!(owner_tag(""), "");
        assert_ne!(owner_tag("d1"), owner_tag("d2"));
    }
}
