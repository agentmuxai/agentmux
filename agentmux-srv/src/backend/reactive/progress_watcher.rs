// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Pushed per-agent **progress**: the agent's current todo checklist and the
//! tool it is running right now, published as an `agent:progress` WaveEvent so
//! the Swarm pane can list them under the agent's name.
//!
//! Sibling of [`super::activity_watcher`], and deliberately a separate loop:
//! that one spends a Haiku call per agent to write an English one-liner and is
//! throttled accordingly. This one only reads NEW bytes of a block's `output`
//! file and parses JSON, so it costs nothing per tick and can run far more
//! often — which is the point, since a checklist that lags 20s behind the agent
//! is worse than no checklist.
//!
//! Why the backend and not the frontend: `agent-document-store` already holds
//! every tool call, but only for panes that are currently MOUNTED. The Swarm
//! pane's whole value is seeing agents you are not looking at, so the data has
//! to come from the same place the shell/cron/summary rows come from.
//!
//! **Why state is carried across ticks** (reagent P1 on PR #2952): the
//! checklist cannot be rebuilt from a fixed tail window. `TodoWrite`-style
//! calls are immune — one call carries the whole list — but the vocabulary
//! AgentMux actually ships against (`TaskCreate`/`TaskUpdate`) describes ONE
//! item per call, so reconstructing the list needs every still-open task's
//! original `TaskCreate` to be inside the window. A verbose `Bash` or a large
//! `Read` pushes those out within seconds, and the task would then vanish from
//! the checklist while still open, silently. So each block keeps an accumulated
//! [`BlockState`] and each tick only consumes the bytes appended since the last
//! one.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::time::interval;

use crate::backend::blockcontroller::{get_block_controller_status, STATUS_RUNNING};
use crate::backend::storage::filestore::FileStore;
use crate::backend::wps::{Broker, WaveEvent};

use super::get_global_handler;

/// How often to sweep registered agents. Much tighter than the summary loop's
/// 20s: this is an incremental read + JSON parse, with no model call to bill.
const SWEEP_INTERVAL_SECS: u64 = 3;

/// Most bytes consumed in one tick, and the most read when first catching up on
/// a block. A block seen from its first byte is exact; one that already had
/// more than this when we first saw it (srv restarted mid-session) is seeded
/// from its tail and flagged — see [`BlockState::partial`].
const MAX_READ_BYTES: i64 = 4 * 1024 * 1024;

/// Cap on rows published per agent, so one pathological checklist can't flood
/// the Swarm tree. Truncation is reported rather than silent — see
/// [`AgentProgress::todos_truncated`].
const MAX_TODOS: usize = 24;

/// Bound on remembered in-flight tool calls. Real concurrency is a handful; a
/// larger number means results are being missed, and the oldest entries are the
/// ones least likely to still be running.
const MAX_OPEN_TOOLS: usize = 32;

pub const EVENT_AGENT_PROGRESS: &str = "agent:progress";

/// Republish an unchanged payload every N ticks (~15s at a 3s sweep).
///
/// The change-suppression below is what keeps this loop quiet, but on its own
/// it strands a LATE subscriber (codex P1 on PR #2952): open the Swarm after an
/// agent's checklist has settled and there is no further change to deliver, so
/// the pane shows nothing indefinitely for a perfectly active agent. The event
/// is also published with `persist: 1` so the broker replays the latest one to
/// a freshly-subscribed route; this periodic republish is the belt to that
/// suspenders, and covers any subscriber the replay path does not.
const REPUBLISH_EVERY_TICKS: u64 = 5;

/// Tool names whose **call arguments** carry checklist state.
///
/// Deliberately a list rather than the single `TodoWrite` everyone expects:
/// Claude Code 2.1.177 — the version AgentMux ships against today — advertises
/// `TaskCreate`/`TaskUpdate` and no `TodoWrite` at all, and other providers
/// differ again. Matching by a known set plus the `Todo` substring below means
/// a version bump that renames the tool degrades to "no rows" rather than to
/// wrong rows.
///
/// READ-style tools (`TaskList`, `TodoRead`) are deliberately absent. Their
/// task data lives in the tool RESULT, not the call's `input`, and this parser
/// only reads `input` — so listing them would advertise support that silently
/// contributes nothing (reagent P2 on PR #2952). Supporting them means parsing
/// tool_result payloads, which is its own change.
const TODO_TOOL_NAMES: &[&str] = &["TodoWrite", "TaskCreate", "TaskUpdate"];

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

/// What we currently believe about one agent.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct AgentProgress {
    pub todos: Vec<TodoItem>,
    /// How many rows [`MAX_TODOS`] dropped, so the UI can say "+N more"
    /// instead of quietly showing a partial list as if it were the whole one.
    pub todos_truncated: usize,
    /// True when this block was first seen mid-stream, so item-at-a-time
    /// (`TaskCreate`) calls from before that point were never observed and the
    /// checklist may be missing entries. Distinct from `todos_truncated`,
    /// which is a cap we chose; this is history we could not see.
    pub todos_partial: bool,
    /// The tool this agent is running right now — the oldest `tool_use` with no
    /// matching `tool_result` yet. `None` when the agent is between tools.
    pub current_tool: Option<String>,
}

impl AgentProgress {
    /// Nothing worth publishing — avoids a WaveEvent per tick per idle agent.
    fn is_empty(&self) -> bool {
        self.todos.is_empty() && self.current_tool.is_none()
    }
}

/// Everything we accumulate for one block across ticks.
#[derive(Debug, Default)]
struct BlockState {
    /// Byte offset of the next unread line in the block's `output`.
    next_offset: i64,
    /// Item-at-a-time accumulation (`TaskCreate`/`TaskUpdate`), keyed so an
    /// update revises its own row. Insertion order is the agent's own creation
    /// order, the only ordering it ever expressed.
    tasks: Vec<(String, TodoItem)>,
    /// Last whole-list call. Supersedes `tasks` outright when present, because
    /// that is the tool's own replace-everything semantics.
    list: Option<Vec<TodoItem>>,
    /// In-flight `tool_use` calls, oldest first.
    open: Vec<(String, String)>,
    /// See [`AgentProgress::todos_partial`].
    partial: bool,
}

impl BlockState {
    fn progress(&self) -> AgentProgress {
        let mut todos = self
            .list
            .clone()
            .unwrap_or_else(|| self.tasks.iter().map(|(_, i)| i.clone()).collect());
        let todos_truncated = todos.len().saturating_sub(MAX_TODOS);
        todos.truncate(MAX_TODOS);
        AgentProgress {
            todos,
            todos_truncated,
            // A whole-list call is self-contained, so once one has been seen
            // the checklist is complete regardless of what we missed earlier.
            todos_partial: self.partial && self.list.is_none(),
            current_tool: self.open.first().map(|(_, name)| name.clone()),
        }
    }
}

/// Fold newly-appended transcript lines into `state`.
///
/// Pure w.r.t. I/O so it can be unit-tested against real transcript shapes
/// without a FileStore. Called once per tick with only the lines appended since
/// the previous call — never the whole file.
fn apply_lines(state: &mut BlockState, lines: &[&str]) {
    for line in lines {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };

        // Resolve finished calls first: a tool_use and its result never share a
        // line, so ordering within the line is not a concern.
        for id in tool_result_ids(&v) {
            state.open.retain(|(open_id, _)| *open_id != id);
        }

        for frame in tool_frames(&v) {
            if frame.get("type").and_then(|t| t.as_str()) != Some("tool_use") {
                continue;
            }
            let Some(name) = frame_tool_name(frame) else { continue };
            if let Some(id) = frame_tool_id(frame) {
                state.open.push((id.to_string(), name.to_string()));
                // Drop the oldest if we're clearly missing results, rather than
                // letting this grow for the life of the block.
                if state.open.len() > MAX_OPEN_TOOLS {
                    state.open.remove(0);
                }
            }

            if !is_todo_tool(name) {
                continue;
            }
            let Some(input) = frame_input(frame) else { continue };

            if let Some(items) = parse_todo_array(input) {
                state.list = Some(items);
                state.tasks.clear();
                continue;
            }
            // A whole-list call, once seen, owns the checklist — an
            // item-at-a-time call after it would be describing a different
            // vocabulary than the one currently in force.
            if state.list.is_none() {
                merge_single_task(&mut state.tasks, input);
            }
        }
    }
}

/// Every frame in a transcript line that could be a tool call or result.
///
/// Covers two genuinely different provider shapes (codex P2 on PR #2952):
///   * Claude stream-json — blocks inside `message.content` (or a bare
///     top-level `content` array).
///   * Gemini and friends — the LINE ITSELF is the frame:
///     `{"type":"tool_use","tool_name":…,"tool_id":…,"parameters":{…}}`
///     (see `frontend/app/view/agent/providers/gemini-translator.ts`).
///
/// Without the second, every non-Claude agent silently showed no in-flight
/// tool and no todos at all, since the array lookup just yielded nothing.
fn tool_frames(v: &serde_json::Value) -> Vec<&serde_json::Value> {
    let mut frames: Vec<&serde_json::Value> = v
        .get("message")
        .and_then(|m| m.get("content"))
        .or_else(|| v.get("content"))
        .and_then(|c| c.as_array())
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    // A bare frame is only a frame if it declares one of the two types we
    // care about — otherwise every ordinary line would look like one.
    if matches!(v.get("type").and_then(|t| t.as_str()), Some("tool_use") | Some("tool_result")) {
        frames.push(v);
    }
    frames
}

/// Tool name under either vocabulary (`name` for Claude, `tool_name` for
/// Gemini-shaped frames).
fn frame_tool_name(frame: &serde_json::Value) -> Option<&str> {
    frame
        .get("name")
        .or_else(|| frame.get("tool_name"))
        .and_then(|n| n.as_str())
}

/// Call id under either vocabulary.
fn frame_tool_id(frame: &serde_json::Value) -> Option<&str> {
    frame
        .get("id")
        .or_else(|| frame.get("tool_id"))
        .and_then(|i| i.as_str())
}

/// Call arguments under either vocabulary.
fn frame_input(frame: &serde_json::Value) -> Option<&serde_json::Value> {
    frame.get("input").or_else(|| frame.get("parameters"))
}

fn tool_result_ids(v: &serde_json::Value) -> Vec<String> {
    let mut ids = Vec::new();
    if let Some(id) = v.get("tool_use_id").and_then(|i| i.as_str()) {
        ids.push(id.to_string());
    }
    for frame in tool_frames(v) {
        if frame.get("type").and_then(|t| t.as_str()) != Some("tool_result") {
            continue;
        }
        // `tool_use_id` (Claude) or `tool_id` (Gemini) — a result names the
        // call it belongs to under whichever key its provider uses.
        if let Some(id) = frame
            .get("tool_use_id")
            .or_else(|| frame.get("tool_id"))
            .and_then(|i| i.as_str())
        {
            ids.push(id.to_string());
        }
    }
    ids
}

/// `input.todos` (or `input.items`) as a whole checklist.
fn parse_todo_array(input: &serde_json::Value) -> Option<Vec<TodoItem>> {
    let arr = input
        .get("todos")
        .or_else(|| input.get("items"))
        .and_then(|t| t.as_array())?;
    // An empty array is a real state ("checklist cleared"), so return it as
    // such rather than falling through to the single-item path.
    Some(arr.iter().filter_map(parse_todo_entry).collect())
}

fn parse_todo_entry(entry: &serde_json::Value) -> Option<TodoItem> {
    let text = first_string(entry, &["content", "title", "text", "activeForm", "description"])?;
    let status = first_string(entry, &["status", "state"]).unwrap_or_else(|| "pending".to_string());
    Some(TodoItem { text, status })
}

/// Fold a Task-style call describing a SINGLE item into the accumulated rows.
///
/// Keyed by whatever id the tool uses so a later call revises the same row,
/// falling back to the text for tools that hand back no id.
///
/// Fields are merged, not replaced (codex P1 on PR #2952). A `TaskUpdate`
/// legitimately carries only `task_id` + the changed `status` — it has no
/// reason to resend text that did not change. Requiring a complete item meant
/// such a call parsed to nothing and the row sat at `pending` forever, which is
/// precisely the case a checklist exists to show moving.
fn merge_single_task(items: &mut Vec<(String, TodoItem)>, input: &serde_json::Value) {
    let text = first_string(input, &["content", "title", "text", "activeForm", "description"]);
    let status = first_string(input, &["status", "state"]);
    let Some(key) = first_string(input, &["task_id", "taskId", "id"]).or_else(|| text.clone()) else {
        // Neither an id nor any text — nothing identifies a row to touch.
        return;
    };

    if let Some(slot) = items.iter_mut().find(|(k, _)| *k == key) {
        if let Some(text) = text {
            slot.1.text = text;
        }
        if let Some(status) = status {
            slot.1.status = status;
        }
        return;
    }

    // Creating a row still needs something to display; an update for a task we
    // never saw created (seeded mid-stream) has no text to show.
    let Some(text) = text else { return };
    items.push((key, TodoItem { text, status: status.unwrap_or_else(|| "pending".to_string()) }));
}

fn first_string(v: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|k| v.get(*k).and_then(|s| s.as_str()))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Consume whatever has been appended to this block's `output` since the last
/// tick, folding it into `state`.
///
/// Returns false when there was nothing to do, so the caller can skip the rest
/// of the work for this block.
fn consume_new_output(filestore: &FileStore, block_id: &str, state: &mut BlockState) -> bool {
    let size = match filestore.stat(block_id, "output") {
        Ok(Some(wf)) if wf.size > 0 => wf.size,
        _ => return false,
    };

    // The file shrank — a new session reusing the block, or a truncation. Our
    // accumulated state describes a transcript that no longer exists, so start
    // over rather than mixing two conversations' checklists.
    if size < state.next_offset {
        *state = BlockState::default();
    }

    if state.next_offset == 0 && size > MAX_READ_BYTES {
        // First sight of a block that is already long (srv restarted
        // mid-session). Seed from the tail and admit the list may be missing
        // item-at-a-time entries from before this point.
        state.next_offset = size - MAX_READ_BYTES;
        state.partial = true;
    }

    let available = size - state.next_offset;
    if available <= 0 {
        return false;
    }
    let want = available.min(MAX_READ_BYTES);
    let Ok((_, bytes)) = filestore.read_at(block_id, "output", state.next_offset, want) else {
        return false;
    };
    if bytes.is_empty() {
        return false;
    }

    // Only consume up to the last complete line: a read boundary lands
    // anywhere, and half a JSON object would be dropped as malformed and then
    // never seen again in its complete form.
    let consumed = match bytes.iter().rposition(|b| *b == b'\n') {
        Some(idx) => idx + 1,
        // No newline anywhere in the chunk — an unterminated line longer than
        // the read. Skip it wholesale rather than stalling forever on it.
        None if want == MAX_READ_BYTES => bytes.len(),
        None => return false,
    };
    state.next_offset += consumed as i64;

    let text = String::from_utf8_lossy(&bytes[..consumed]);
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        return false;
    }
    apply_lines(state, &lines);
    true
}

/// Run the progress sweep loop. Never returns.
pub async fn run_agent_progress_loop(filestore: Arc<FileStore>, broker: Arc<Broker>) {
    let mut ticker = interval(Duration::from_secs(SWEEP_INTERVAL_SECS));
    let mut tick: u64 = 0;
    let mut states: HashMap<String, BlockState> = HashMap::new();
    // Last payload published per block, so an unchanged checklist doesn't
    // re-publish every 3s.
    let mut last_published: HashMap<String, AgentProgress> = HashMap::new();

    loop {
        ticker.tick().await;
        tick += 1;
        let force_republish = tick % REPUBLISH_EVERY_TICKS == 0;

        let agents = get_global_handler().list_agents();
        let registered: std::collections::HashSet<String> =
            agents.iter().map(|a| a.block_id.clone()).collect();
        // Both maps are bounded by the live agent count, not by every block_id
        // ever seen in the process's lifetime.
        states.retain(|block_id, _| registered.contains(block_id));
        last_published.retain(|block_id, _| registered.contains(block_id));

        for agent in agents {
            let block_id = agent.block_id.clone();

            // Same gate as the summary loop: an idle or non-agent pane has no
            // progress to report and shouldn't be read every tick.
            let Some(status) = get_block_controller_status(&block_id) else { continue };
            if status.shellprocstatus != STATUS_RUNNING || !status.is_agent_pane {
                continue;
            }

            let state = states.entry(block_id.clone()).or_default();
            let had_new = consume_new_output(&filestore, &block_id, state);
            let progress = state.progress();

            // Nothing new AND nothing already published for this block — an
            // agent that has never had progress costs no events at all.
            if !had_new && !last_published.contains_key(&block_id) {
                continue;
            }
            if progress.is_empty() && !last_published.contains_key(&block_id) {
                continue;
            }
            if !force_republish && last_published.get(&block_id) == Some(&progress) {
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
                // Replayed to a freshly-subscribed route, so a Swarm opened
                // mid-session sees the current checklist rather than waiting
                // for the agent's next change — see REPUBLISH_EVERY_TICKS.
                persist: 1,
                data: Some(serde_json::json!({
                    "agentId": agent.agent_id,
                    "blockId": block_id,
                    "todos": progress.todos,
                    "todosTruncated": progress.todos_truncated,
                    "todosPartial": progress.todos_partial,
                    "currentTool": progress.current_tool,
                    "ts": ts,
                })),
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

    fn task_call(id: &str, tool: &str, tid: &str, title: &str, status: &str) -> String {
        format!(
            r#"{{"type":"assistant","message":{{"content":[{{"type":"tool_use","id":"{id}","name":"{tool}","input":{{"task_id":"{tid}","title":"{title}","status":"{status}"}}}}]}}}}"#
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

    /// One tick's worth of lines.
    fn feed(state: &mut BlockState, lines: &[String]) {
        let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        apply_lines(state, &refs);
    }

    fn run(lines: &[String]) -> AgentProgress {
        let mut st = BlockState::default();
        feed(&mut st, lines);
        st.progress()
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

    #[test]
    fn the_latest_full_list_supersedes_earlier_ones() {
        let p = run(&[
            todo_write("t1", r#"[{"content":"A","status":"pending"}, {"content":"B","status":"pending"}]"#),
            todo_write("t2", r#"[{"content":"A","status":"completed"}]"#),
        ]);
        assert_eq!(p.todos, vec![TodoItem { text: "A".into(), status: "completed".into() }]);
    }

    #[test]
    fn task_style_calls_accumulate_and_update_in_place() {
        let p = run(&[
            task_call("c1", "TaskCreate", "k1", "First", "pending"),
            task_call("c2", "TaskCreate", "k2", "Second", "pending"),
            task_call("u1", "TaskUpdate", "k1", "First", "completed"),
        ]);
        assert_eq!(
            p.todos,
            vec![
                TodoItem { text: "First".into(), status: "completed".into() },
                TodoItem { text: "Second".into(), status: "pending".into() },
            ],
            "an update revises its own row and preserves creation order",
        );
    }

    /// THE reagent P1 regression: a TaskCreate seen in an EARLIER tick must
    /// survive once its line has scrolled out of any fixed tail window. A
    /// stateless, tail-only parser drops it silently while the task is still
    /// open — the failure this whole accumulate-across-ticks design exists for.
    #[test]
    fn a_task_created_in_an_earlier_tick_survives_later_unrelated_output() {
        let mut st = BlockState::default();
        feed(&mut st, &[task_call("c1", "TaskCreate", "k1", "Long-lived task", "pending")]);

        // A later tick carrying only unrelated chatter — no todo tool in sight.
        let noise: Vec<String> = (0..50).map(|i| tool_use(&format!("n{i}"), "Read")).collect();
        feed(&mut st, &noise);

        assert_eq!(
            st.progress().todos,
            vec![TodoItem { text: "Long-lived task".into(), status: "pending".into() }],
            "the task is still open and must still be listed",
        );
    }

    #[test]
    fn an_update_arriving_a_tick_later_still_finds_its_row() {
        let mut st = BlockState::default();
        feed(&mut st, &[task_call("c1", "TaskCreate", "k1", "Task", "pending")]);
        feed(&mut st, &[tool_use("n1", "Bash"), tool_result("n1")]);
        feed(&mut st, &[task_call("u1", "TaskUpdate", "k1", "Task", "completed")]);

        assert_eq!(st.progress().todos, vec![TodoItem { text: "Task".into(), status: "completed".into() }]);
    }

    #[test]
    fn current_tool_is_the_outstanding_tool_use() {
        let p = run(&[tool_use("a", "Read"), tool_result("a"), tool_use("b", "Bash")]);
        assert_eq!(p.current_tool.as_deref(), Some("Bash"));
    }

    #[test]
    fn no_current_tool_once_every_call_has_returned() {
        let p = run(&[tool_use("a", "Read"), tool_result("a")]);
        assert_eq!(p.current_tool, None);
    }

    /// A result arriving in a later tick than its call must still clear it —
    /// the common case now that ticks are 3s and tools are slower than that.
    #[test]
    fn a_result_in_a_later_tick_clears_the_call_from_an_earlier_one() {
        let mut st = BlockState::default();
        feed(&mut st, &[tool_use("a", "Bash")]);
        assert_eq!(st.progress().current_tool.as_deref(), Some("Bash"));
        feed(&mut st, &[tool_result("a")]);
        assert_eq!(st.progress().current_tool, None);
    }

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

    /// `partial` is about history we could not see; `todos_truncated` is a cap
    /// we chose. They are different claims and must not be conflated.
    #[test]
    fn a_tail_seeded_block_reports_its_checklist_as_partial() {
        let mut st = BlockState { partial: true, ..Default::default() };
        feed(&mut st, &[task_call("c1", "TaskCreate", "k1", "Only what we saw", "pending")]);
        let p = st.progress();
        assert!(p.todos_partial, "item-at-a-time rows may be missing earlier entries");
        assert_eq!(p.todos_truncated, 0, "nothing was dropped by the cap");
    }

    /// …but a whole-list call is self-contained, so seeing one makes the
    /// checklist complete regardless of what came before.
    #[test]
    fn a_full_list_call_clears_the_partial_flag() {
        let mut st = BlockState { partial: true, ..Default::default() };
        feed(&mut st, &[todo_write("t1", r#"[{"content":"Whole list","status":"pending"}]"#)]);
        assert!(!st.progress().todos_partial);
    }

    #[test]
    fn ignores_malformed_lines_instead_of_giving_up_on_the_whole_batch() {
        let p = run(&[
            "not json at all".to_string(),
            String::new(),
            todo_write("t1", r#"[{"content":"Survives","status":"pending"}]"#),
        ]);
        assert_eq!(p.todos.len(), 1);
    }

    #[test]
    fn a_non_todo_tool_never_contributes_checklist_rows() {
        let p = run(&[tool_use("a", "Read"), tool_use("b", "Bash")]);
        assert!(p.todos.is_empty());
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

    /// Read-style tools return their data in the RESULT, which this parser
    /// never reads — so claiming to support them would be advertising a
    /// no-op (reagent P2 on PR #2952).
    #[test]
    fn read_style_task_tools_are_not_claimed_as_supported() {
        assert!(
            !TODO_TOOL_NAMES.contains(&"TaskList"),
            "TaskList's tasks live in its tool_result, not its input",
        );
        // TodoRead matches only via the deliberate `Todo` substring fallback,
        // and still contributes nothing because its input carries no items —
        // which is correct behavior, not a supported path.
        assert!(parse_todo_array(&serde_json::json!({})).is_none());
    }

    #[test]
    fn the_open_tool_list_stays_bounded_when_results_go_missing() {
        let mut st = BlockState::default();
        let calls: Vec<String> = (0..MAX_OPEN_TOOLS + 20)
            .map(|i| tool_use(&format!("t{i}"), "Read"))
            .collect();
        feed(&mut st, &calls);
        assert_eq!(st.open.len(), MAX_OPEN_TOOLS);
    }

    /// codex P1: a TaskUpdate legitimately carries only the id and the changed
    /// status. Requiring a complete item meant the row sat at `pending`
    /// forever — precisely the transition a checklist exists to show.
    #[test]
    fn a_status_only_update_moves_the_row_without_resending_its_text() {
        let mut st = BlockState::default();
        feed(&mut st, &[task_call("c1", "TaskCreate", "k1", "Ship it", "pending")]);
        feed(
            &mut st,
            &[format!(
                r#"{{"type":"assistant","message":{{"content":[{{"type":"tool_use","id":"u1","name":"TaskUpdate","input":{{"task_id":"k1","status":"completed"}}}}]}}}}"#
            )],
        );
        assert_eq!(
            st.progress().todos,
            vec![TodoItem { text: "Ship it".into(), status: "completed".into() }],
            "text is preserved from the create, status taken from the update",
        );
    }

    /// The mirror: a text-only edit must not reset a status back to pending.
    #[test]
    fn a_text_only_update_preserves_the_existing_status() {
        let mut st = BlockState::default();
        feed(&mut st, &[task_call("c1", "TaskCreate", "k1", "Old wording", "in_progress")]);
        feed(
            &mut st,
            &[format!(
                r#"{{"type":"assistant","message":{{"content":[{{"type":"tool_use","id":"u1","name":"TaskUpdate","input":{{"task_id":"k1","title":"New wording"}}}}]}}}}"#
            )],
        );
        assert_eq!(
            st.progress().todos,
            vec![TodoItem { text: "New wording".into(), status: "in_progress".into() }],
        );
    }

    /// An update for a task we never saw created (seeded mid-stream) has no
    /// text to display, so it must not invent a blank row.
    #[test]
    fn a_status_only_update_for_an_unknown_task_creates_no_row() {
        let mut st = BlockState::default();
        feed(
            &mut st,
            &[format!(
                r#"{{"type":"assistant","message":{{"content":[{{"type":"tool_use","id":"u1","name":"TaskUpdate","input":{{"task_id":"ghost","status":"completed"}}}}]}}}}"#
            )],
        );
        assert!(st.progress().todos.is_empty());
    }

    /// codex P2: Gemini-shaped transcripts put the frame at the TOP LEVEL with
    /// `tool_name`/`tool_id`/`parameters`. Before this, every non-Claude agent
    /// showed no in-flight tool and no todos at all.
    #[test]
    fn parses_provider_native_top_level_tool_frames() {
        let mut st = BlockState::default();
        feed(
            &mut st,
            &[r#"{"type":"tool_use","tool_name":"Bash","tool_id":"g1","parameters":{}}"#.to_string()],
        );
        assert_eq!(st.progress().current_tool.as_deref(), Some("Bash"));

        feed(
            &mut st,
            &[r#"{"type":"tool_result","tool_id":"g1","status":"success"}"#.to_string()],
        );
        assert_eq!(st.progress().current_tool, None, "a tool_id result clears its call");
    }

    #[test]
    fn parses_a_todo_checklist_from_a_top_level_frame() {
        let mut st = BlockState::default();
        feed(
            &mut st,
            &[r#"{"type":"tool_use","tool_name":"TodoWrite","tool_id":"g2","parameters":{"todos":[{"content":"From Gemini","status":"pending"}]}}"#.to_string()],
        );
        assert_eq!(
            st.progress().todos,
            vec![TodoItem { text: "From Gemini".into(), status: "pending".into() }],
        );
    }

    /// An ordinary transcript line must not be mistaken for a bare tool frame.
    #[test]
    fn a_plain_line_is_not_treated_as_a_tool_frame() {
        let mut st = BlockState::default();
        feed(&mut st, &[r#"{"type":"system","subtype":"init","name":"TodoWrite"}"#.to_string()]);
        assert!(st.progress().todos.is_empty());
        assert_eq!(st.progress().current_tool, None);
    }

    #[test]
    fn empty_progress_is_recognized_so_idle_agents_publish_nothing() {
        assert!(AgentProgress::default().is_empty());
        let p = run(&[tool_use("a", "Read")]);
        assert!(!p.is_empty(), "an in-flight tool is worth publishing on its own");
    }
}
