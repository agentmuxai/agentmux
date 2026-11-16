// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Codex's plan as an agent's checklist, for the progress watcher (SPEC_WARDEN_KANBAN_2026_10_20.md §6.12).

use super::{first_string, parse_todo_entry, TodoItem};

/// Codex's plan in a transcript line, as a checklist:
/// - `codex exec`: `{"type":"item.started|item.updated|item.completed","item":{"type":"todo_list","items":[{"text","completed"}]}}`;
/// - `{"item":{"type":"plan_update","plan":[{"step","status"}]}}`;
/// - an `update_plan` function call whose `arguments` (a JSON string or object) carry `plan`.
///
/// `todo_list` items say only whether each step is done, so the first one not
/// done is shown as the step in progress.
pub(super) fn codex_plan(v: &serde_json::Value) -> Option<Vec<TodoItem>> {
    let item = v.get("item")?;
    let kind = item.get("type").and_then(|t| t.as_str())?;
    let entries = match kind {
        "todo_list" => {
            let arr = item.get("items")?.as_array()?;
            let mut current = false;
            return Some(
                arr.iter()
                    .filter_map(|e| {
                        let text = first_string(e, &["text", "step", "content"])?;
                        let done = e.get("completed").and_then(|c| c.as_bool()).unwrap_or(false);
                        let status = if done {
                            "completed"
                        } else if !current {
                            current = true;
                            "in_progress"
                        } else {
                            "pending"
                        };
                        Some(TodoItem { text, status: status.to_string() })
                    })
                    .collect(),
            );
        }
        "plan_update" => item.get("plan")?.as_array()?.clone(),
        "function_call" if item.get("name").and_then(|n| n.as_str()) == Some("update_plan") => {
            let args = item.get("arguments")?;
            let args = match args {
                serde_json::Value::String(s) => serde_json::from_str::<serde_json::Value>(s).ok()?,
                other => other.clone(),
            };
            args.get("plan")?.as_array()?.clone()
        }
        _ => return None,
    };
    Some(entries.iter().filter_map(parse_todo_entry).collect())
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_codex_plans_as_the_checklist() {
        let exec = r#"{"type":"item.updated","item":{"id":"i1","type":"todo_list","items":[{"text":"read","completed":true},{"text":"write","completed":false},{"text":"test","completed":false}]}}"#;
        let todos = codex_plan(&serde_json::from_str(exec).unwrap()).unwrap();
        let rows: Vec<(String, String)> = todos.iter().map(|t| (t.text.clone(), t.status.clone())).collect();
        assert_eq!(
            rows,
            [("read".into(), "completed".into()), ("write".into(), "in_progress".into()), ("test".into(), "pending".into())]
        );

        let update = r#"{"item":{"type":"plan_update","plan":[{"step":"plan","status":"completed"},{"step":"build","status":"in_progress"}]}}"#;
        let todos = codex_plan(&serde_json::from_str(update).unwrap()).unwrap();
        assert_eq!(todos.iter().map(|t| t.text.as_str()).collect::<Vec<_>>(), ["plan", "build"]);

        let call = r#"{"item":{"type":"function_call","name":"update_plan","arguments":"{\"plan\":[{\"step\":\"ship\",\"status\":\"pending\"}]}"}}"#;
        let todos = codex_plan(&serde_json::from_str(call).unwrap()).unwrap();
        assert_eq!((todos[0].text.as_str(), todos[0].status.as_str()), ("ship", "pending"));
        assert!(codex_plan(&serde_json::json!({"item": {"type": "agent_message", "text": "hi"}})).is_none());
    }

}
