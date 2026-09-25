// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! srv-side notification sources that need no mounted pane —
//! `SPEC_OS_NOTIFICATIONS_SYSTEM_2026_09_24.md` §10 Phase 5.
//!
//! The persistent Claude controller already receives every AskUserQuestion as
//! a control-protocol `can_use_tool` request (it parks it in
//! `pending_questions`, `persistent/input.rs`). Detecting it here — rather
//! than in the renderer's reducer — is what lets "an agent needs your input"
//! reach you with zero windows open (background mode).

/// An `AskUserQuestion` call: the first question's text (when readable) and
/// how many questions the call asks.
#[derive(Debug, PartialEq, Eq)]
pub struct AskedQuestion {
    pub text: Option<String>,
    pub count: usize,
}

/// If `frame` is a `control_request` asking to run `AskUserQuestion`, return
/// what it asks (`text: None` when the frame has no readable text). `None`
/// for every other frame.
pub fn ask_user_question(frame: &serde_json::Value) -> Option<AskedQuestion> {
    if frame.get("type").and_then(|v| v.as_str()) != Some("control_request") {
        return None;
    }
    let req = frame.get("request")?;
    if req.get("subtype").and_then(|v| v.as_str()) != Some("can_use_tool") {
        return None;
    }
    if req.get("tool_name").and_then(|v| v.as_str()) != Some("AskUserQuestion") {
        return None;
    }
    let questions = req.get("input").and_then(|i| i.get("questions")).and_then(|q| q.as_array());
    Some(AskedQuestion {
        text: questions
            .and_then(|q| q.first())
            .and_then(|q| q.get("question"))
            .and_then(|t| t.as_str())
            .map(str::to_string),
        count: questions.map_or(0, Vec::len),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn detects_ask_user_question_and_extracts_text() {
        let f = json!({"type":"control_request","request_id":"r1","request":{
            "subtype":"can_use_tool","tool_name":"AskUserQuestion","tool_use_id":"t1",
            "input":{"questions":[{"question":"Which branch?","header":"Branch","options":[]}]}}});
        assert_eq!(ask_user_question(&f), Some(AskedQuestion { text: Some("Which branch?".to_string()), count: 1 }));
    }

    #[test]
    fn counts_every_question_in_the_call() {
        let f = json!({"type":"control_request","request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion",
            "input":{"questions":[{"question":"Which branch?"},{"question":"Squash?"},{"question":"Tag it?"}]}}});
        assert_eq!(ask_user_question(&f), Some(AskedQuestion { text: Some("Which branch?".to_string()), count: 3 }));
    }

    #[test]
    fn tolerates_missing_text_and_ignores_other_frames() {
        let no_text = json!({"type":"control_request","request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion","input":{}}});
        assert_eq!(ask_user_question(&no_text), Some(AskedQuestion { text: None, count: 0 }));
        let bash = json!({"type":"control_request","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{}}});
        assert_eq!(ask_user_question(&bash), None);
        let other = json!({"type":"control_request","request":{"subtype":"interrupt"}});
        assert_eq!(ask_user_question(&other), None);
        assert_eq!(ask_user_question(&json!({"type":"control_response"})), None);
        assert_eq!(ask_user_question(&json!({"type":"assistant"})), None);
    }
}
