// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Pushed per-agent **progress**: the agent's current todo checklist and the
//! tool it is running right now, published as an `agent:progress` WaveEvent so
//! the Swarm pane can list them under the agent's name.
//!
//! Sibling of [`super::activity_watcher`], and deliberately a separate loop:
//! that one spends a Haiku call per agent to write an English one-liner and is
//! throttled accordingly. This one only reads the tail of a block's `output`
//! file and parses JSON, so it costs nothing per tick and can run far more
//! often — which is the point, since a checklist that lags 20s behind the agent
//! is worse than no checklist.
//!
//! Why the backend and not the frontend: `agent-document-store` already holds
//! every tool call, but only for panes that are currently MOUNTED. The Swarm
//! pane's whole value is seeing agents you are not looking at, so the data has
//! to come from the same place the shell/cron/summary rows come from.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::time::interval;

use crate::backend::blockcontroller::{get_block_controller_status, STATUS_RUNNING};
use crate::backend::storage::filestore::FileStore;
use crate::backend::wps::{Broker, WaveEvent};

use super::get_global_handler;

/// How often to sweep registered agents. Much tighter than the summary loop's
/// 20s: this is a pure tail-read + JSON parse, with no model call to bill.
const SWEEP_INTERVAL_SECS: u64 = 3;

/// How much of the block's `output` to read. The checklist is republished in
/// full on every `TodoWrite`-style call, so we only need enough tail to catch
/// the most recent one — matching `read_recent_activity_digest`'s window.
const TAIL_BYTES: i64 = 32 * 1024;

/// Cap on rows published per agent, so one pathological checklist can't flood
/// the Swarm tree. Truncation is reported rather than silent — see
/// [`AgentProgress::todos_truncated`].
const MAX_TODOS: usize = 24;

pub const EVENT_AGENT_PROGRESS: &str = "agent:progress";

/// Tool names that carry a todo checklist.
///
/// Deliberately a list rather than the single `TodoWrite` everyone expects:
/// Claude Code 2.1.177 — the version AgentMux ships against today — advertises
/// `TaskCreate`/`TaskUpdate`/`TaskList` and no `TodoWrite` at all, and other
/// providers differ again. Matching by a known set plus the `Todo` substring
/// below means a version bump that renames the tool degrades to "no rows"
/// rather than to wrong rows.
const TODO_TOOL_NAMES: &[&str] = &["TodoWrite", "TodoRead", "TaskCreate", "TaskUpdate", "TaskList"];

fn is_todo_tool(name: &str) -> bool {
    TODO_TOOL_NAMES.contains(&name) || name.contains("Todo")
}

/// One checklist entry as the Swarm renders it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TodoItem {
    pub text: String,
    /// `pending` | `in_progress` | `completed`. Passed through as the provider
    /// wrote it where possible; unknown values are kept verbatim rather than
    /// coerced, so a new provider status shows up as itself instead of
    /// silently becoming "pending".
    pub status: String,
}

/// What one sweep extracted for a single agent.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct AgentProgress {
    pub todos: Vec<TodoItem>,
    /// Number dropped by [`MAX_TODOS`], so the UI can say "+N more" instead of
    /// quietly showing a partial list as if it were the whole one.
    pub todos_truncated: usize,
    /// The tool currently in flight — the last `tool_use` with no matching
    /// `tool_result` yet. `None` when the agent is between tools.
    pub current_tool: Option<String>,
}

impl AgentProgress {
    /// Nothing worth publishing — avoids a WaveEvent per tick per idle agent.
    fn is_empty(&self) -> bool {
        self.todos.is_empty() && self.current_tool.is_none()
    }
}

/// Pull the todo checklist and in-flight tool out of a block's NDJSON output.
///
/// `lines` is the tail of the `output` file, oldest first. Pure and
/// allocation-light so it can be unit-tested against real transcript shapes
/// without a FileStore.
///
/// Two independent passes over the same tool_use stream:
///   * **todos** — last call to a todo-ish tool wins outright when it carries a
///     full `input.todos` array (that is replace-the-whole-list semantics, the
///     same as the tool itself). Task-style `create`/`update` calls, which
///     describe ONE item each, instead accumulate into an ordered map keyed by
///     task id so an update revises the row it belongs to.
///   * **current_tool** — last `tool_use` whose id never appears in a later
///     `tool_result`.
pub fn extract_progress(lines: &[&str]) -> AgentProgress {
    // Ordered accumulation for the Task* shape. Insertion order is the agent's
    // own creation order, which is the only ordering it ever expressed.
    let mut task_items: Vec<(String, TodoItem)> = Vec::new();
    // A full `todos` array supersedes anything accumulated before it.
    let mut list_items: Option<Vec<TodoItem>> = None;

    let mut open_tools: Vec<(String, String)> = Vec::new(); // (tool_use id, name)
    let mut finished: HashSet<String> = HashSet::new();

    for line in lines {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };

        // A tool_result can arrive either as a user-message content block or as
        // a bare frame, depending on provider/version — collect ids from both.
        collect_tool_result_ids(&v, &mut finished);

        for block in content_blocks(&v) {
            if block.get("type").and_then(|t| t.as_str()) != Some("tool_use") {
                continue;
            }
            let Some(name) = block.get("name").and_then(|n| n.as_str()) else {
                continue;
            };
            if let Some(id) = block.get("id").and_then(|i| i.as_str()) {
                open_tools.push((id.to_string(), name.to_string()));
            }

            if !is_todo_tool(name) {
                continue;
            }
            let Some(input) = block.get("input") else { continue };

            if let Some(items) = parse_todo_array(input) {
                // Full-list semantics: this call IS the checklist now.
                list_items = Some(items);
                task_items.clear();
                continue;
            }
            if let Some((key, item)) = parse_single_task(input) {
                // Only meaningful while no full list has superseded it.
                if list_items.is_none() {
                    upsert(&mut task_items, key, item);
                }
            }
        }
    }

    let mut todos = list_items.unwrap_or_else(|| task_items.into_iter().map(|(_, i)| i).collect());
    let todos_truncated = todos.len().saturating_sub(MAX_TODOS);
    todos.truncate(MAX_TODOS);

    let current_tool = open_tools
        .iter()
        .rev()
        .find(|(id, _)| !finished.contains(id))
        .map(|(_, name)| name.clone());

    AgentProgress { todos, todos_truncated, current_tool }
}

/// Content blocks of an assistant/user message, wherever this provider puts
/// them (`message.content` for Claude's stream-json, bare `content` otherwise).
fn content_blocks(v: &serde_json::Value) -> impl Iterator<Item = &serde_json::Value> {
    v.get("message")
        .and_then(|m| m.get("content"))
        .or_else(|| v.get("content"))
        .and_then(|c| c.as_array())
        .map(|a| a.iter())
        .unwrap_or_else(|| [].iter())
}

fn collect_tool_result_ids(v: &serde_json::Value, out: &mut HashSet<String>) {
    if let Some(id) = v.get("tool_use_id").and_then(|i| i.as_str()) {
        out.insert(id.to_string());
    }
    for block in content_blocks(v) {
        if block.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
            if let Some(id) = block.get("tool_use_id").and_then(|i| i.as_str()) {
                out.insert(id.to_string());
            }
        }
    }
}

/// `input.todos` (or `input.items`) as a whole checklist.
fn parse_todo_array(input: &serde_json::Value) -> Option<Vec<TodoItem>> {
    let arr = input
        .get("todos")
        .or_else(|| input.get("items"))
        .and_then(|t| t.as_array())?;
    let items: Vec<TodoItem> = arr.iter().filter_map(parse_todo_entry).collect();
    // An empty array is a real state ("checklist cleared"), so return it as
    // such rather than falling through to the single-task path.
    Some(items)
}

fn parse_todo_entry(entry: &serde_json::Value) -> Option<TodoItem> {
    let text = first_string(entry, &["content", "title", "text", "activeForm", "description"])?;
    let status = first_string(entry, &["status", "state"]).unwrap_or_else(|| "pending".to_string());
    Some(TodoItem { text, status })
}

/// A Task-style call describing a single item. Keyed by whatever id the tool
/// uses so a later update revises the same row; falls back to the text itself,
/// which is stable enough for tools that don't hand back an id.
fn parse_single_task(input: &serde_json::Value) -> Option<(String, TodoItem)> {
    let item = parse_todo_entry(input)?;
    let key = first_string(input, &["task_id", "taskId", "id"]).unwrap_or_else(|| item.text.clone());
    Some((key, item))
}

fn first_string(v: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|k| v.get(*k).and_then(|s| s.as_str()))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn upsert(items: &mut Vec<(String, TodoItem)>, key: String, item: TodoItem) {
    if let Some(slot) = items.iter_mut().find(|(k, _)| *k == key) {
        slot.1 = item;
    } else {
        items.push((key, item));
    }
}

/// Read a block's output tail and extract its progress.
fn progress_for_block(filestore: &FileStore, block_id: &str) -> Option<AgentProgress> {
    let size = match filestore.stat(block_id, "output") {
        Ok(Some(wf)) if wf.size > 0 => wf.size,
        _ => return None,
    };
    let offset = (size - TAIL_BYTES).max(0);
    let (_, bytes) = filestore.read_at(block_id, "output", offset, TAIL_BYTES).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    Some(extract_progress(&lines))
}

/// Run the progress sweep loop. Never returns.
pub async fn run_agent_progress_loop(filestore: Arc<FileStore>, broker: Arc<Broker>) {
    let mut ticker = interval(Duration::from_secs(SWEEP_INTERVAL_SECS));
    // block_id -> last payload published, so an unchanged checklist doesn't
    // re-publish every 3s. Bounded by pruning against the live registration
    // list each tick, same as the summary loop's own bookkeeping.
    let mut last_published: HashMap<String, AgentProgress> = HashMap::new();

    loop {
        ticker.tick().await;

        let agents = get_global_handler().list_agents();
        let registered: HashSet<String> = agents.iter().map(|a| a.block_id.clone()).collect();
        last_published.retain(|block_id, _| registered.contains(block_id));

        for agent in agents {
            let block_id = agent.block_id.clone();

            // Same gate as the summary loop: an idle or non-agent pane has no
            // progress to report and shouldn't be read every tick.
            let Some(status) = get_block_controller_status(&block_id) else { continue };
            if status.shellprocstatus != STATUS_RUNNING || !status.is_agent_pane {
                continue;
            }

            let Some(progress) = progress_for_block(&filestore, &block_id) else { continue };

            // Publish an empty payload ONCE when a previously-non-empty agent
            // goes quiet (so the UI can clear its rows), but never for an agent
            // that has had nothing all along.
            if progress.is_empty() && !last_published.contains_key(&block_id) {
                continue;
            }
            if last_published.get(&block_id) == Some(&progress) {
                continue;
            }
            last_published.insert(block_id.clone(), progress.clone());

            let ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);

            broker.publish(WaveEvent {
                event: EVENT_AGENT_PROGRESS.to_string(),
                scopes: vec![format!("block:{}", block_id)],
                sender: String::new(),
                persist: 0,
                data: serde_json::to_value(serde_json::json!({
                    "agentId": agent.agent_id,
                    "blockId": block_id,
                    "todos": progress.todos,
                    "todosTruncated": progress.todos_truncated,
                    "currentTool": progress.current_tool,
                    "ts": ts,
                }))
                .ok(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn todo_write(id: &str, todos: &str) -> String {
        format!(
            r#"{{"type":"assistant","message":{{"content":[{{"type":"tool_use","id":"{id}","name":"TodoWrite","input":{{"todos":{todos}}}}}]}}}}"#
        )
    }

    fn tool_use(id: &str, name: &str) -> String {
        format!(
            r#"{{"type":"assistant","message":{{"content":[{{"type":"tool_use","id":"{id}","name":"{name}","input":{{}}}}]}}}}"#
        )
    }

    fn tool_result(id: &str) -> String {
        format!(
            r#"{{"type":"user","message":{{"content":[{{"type":"tool_result","tool_use_id":"{id}"}}]}}}}"#
        )
    }

    fn run(lines: &[String]) -> AgentProgress {
        let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        extract_progress(&refs)
    }

    #[test]
    fn extracts_a_todowrite_checklist() {
        let p = run(&[todo_write(
            "t1",
            r#"[{"content":"Fix the bug","status":"in_progress"},{"content":"Write tests","status":"pending"}]"#,
        )]);
        assert_eq!(
            p.todos,
            vec![
                TodoItem { text: "Fix the bug".into(), status: "in_progress".into() },
                TodoItem { text: "Write tests".into(), status: "pending".into() },
            ]
        );
    }

    /// TodoWrite republishes the WHOLE list every call, so the newest call is
    /// the state — an earlier, longer list must not leak through.
    #[test]
    fn the_latest_full_list_supersedes_earlier_ones() {
        let p = run(&[
            todo_write("t1", r#"[{"content":"A","status":"pending"},{"content":"B","status":"pending"}]"#),
            todo_write("t2", r#"[{"content":"A","status":"completed"}]"#),
        ]);
        assert_eq!(p.todos, vec![TodoItem { text: "A".into(), status: "completed".into() }]);
    }

    /// The version AgentMux actually ships against has no TodoWrite at all —
    /// it has TaskCreate/TaskUpdate, one item per call. An update must revise
    /// the row it refers to rather than appending a duplicate.
    #[test]
    fn task_style_calls_accumulate_and_update_in_place() {
        let create = |id: &str, tid: &str, title: &str| {
            format!(
                r#"{{"type":"assistant","message":{{"content":[{{"type":"tool_use","id":"{id}","name":"TaskCreate","input":{{"task_id":"{tid}","title":"{title}","status":"pending"}}}}]}}}}"#
            )
        };
        let update = format!(
            r#"{{"type":"assistant","message":{{"content":[{{"type":"tool_use","id":"u1","name":"TaskUpdate","input":{{"task_id":"k1","title":"First","status":"completed"}}}}]}}}}"#
        );
        let p = run(&[create("c1", "k1", "First"), create("c2", "k2", "Second"), update]);
        assert_eq!(
            p.todos,
            vec![
                TodoItem { text: "First".into(), status: "completed".into() },
                TodoItem { text: "Second".into(), status: "pending".into() },
            ],
            "an update revises its own row and preserves creation order",
        );
    }

    #[test]
    fn current_tool_is_the_last_unresolved_tool_use() {
        let p = run(&[
            tool_use("a", "Read"),
            tool_result("a"),
            tool_use("b", "Bash"),
        ]);
        assert_eq!(p.current_tool.as_deref(), Some("Bash"));
    }

    #[test]
    fn no_current_tool_once_every_call_has_returned() {
        let p = run(&[tool_use("a", "Read"), tool_result("a")]);
        assert_eq!(p.current_tool, None);
    }

    /// An emptied checklist is a real state the UI must be able to show, not a
    /// reason to fall back to stale rows.
    #[test]
    fn an_explicitly_cleared_checklist_reads_as_empty() {
        let p = run(&[
            todo_write("t1", r#"[{"content":"A","status":"pending"}]"#),
            todo_write("t2", "[]"),
        ]);
        assert!(p.todos.is_empty());
    }

    #[test]
    fn truncates_a_pathological_list_and_reports_how_many_were_dropped() {
        let entries: Vec<String> = (0..MAX_TODOS + 5)
            .map(|i| format!(r#"{{"content":"item {i}","status":"pending"}}"#))
            .collect();
        let p = run(&[todo_write("t1", &format!("[{}]", entries.join(",")))]);
        assert_eq!(p.todos.len(), MAX_TODOS);
        assert_eq!(p.todos_truncated, 5);
    }

    #[test]
    fn ignores_malformed_lines_instead_of_giving_up_on_the_whole_tail() {
        let p = run(&[
            "not json at all".to_string(),
            String::new(),
            todo_write("t1", r#"[{"content":"Survives","status":"pending"}]"#),
        ]);
        assert_eq!(p.todos.len(), 1);
    }

    /// A tail window can start mid-conversation, so the first line is often a
    /// fragment. It must not poison the rest.
    #[test]
    fn a_truncated_leading_line_does_not_break_extraction() {
        let mut lines = vec![r#"{"type":"assistant","message":{"conte"#.to_string()];
        lines.push(todo_write("t1", r#"[{"content":"Still parsed","status":"pending"}]"#));
        let p = run(&lines);
        assert_eq!(p.todos, vec![TodoItem { text: "Still parsed".into(), status: "pending".into() }]);
    }

    #[test]
    fn a_non_todo_tool_never_contributes_checklist_rows() {
        let p = run(&[tool_use("a", "Read"), tool_use("b", "Bash")]);
        assert!(p.todos.is_empty());
        assert_eq!(p.current_tool.as_deref(), Some("Bash"));
    }

    #[test]
    fn recognizes_todo_tools_by_name_across_provider_vocabularies() {
        assert!(is_todo_tool("TodoWrite"));
        assert!(is_todo_tool("TaskCreate"));
        assert!(is_todo_tool("TaskUpdate"));
        // Substring fallback for a renamed/namespaced variant.
        assert!(is_todo_tool("mcp__planner__TodoSync"));
        assert!(!is_todo_tool("Read"));
        assert!(!is_todo_tool("Task"), "the subagent-spawning Task tool is not a checklist");
    }

    #[test]
    fn empty_progress_is_recognized_so_idle_agents_publish_nothing() {
        assert!(AgentProgress::default().is_empty());
        let p = run(&[tool_use("a", "Read")]);
        assert!(!p.is_empty(), "an in-flight tool is worth publishing on its own");
    }
}
