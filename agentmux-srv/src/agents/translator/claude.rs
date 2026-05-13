// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Claude Code stream-json → `AgentEvent` translator.
//!
//! Claude Code emits one JSON object per line. Top-level shape:
//!
//!   ```json
//!   {"type":"stream_event","event":{...}}
//!   {"type":"message_start","message":{...}}
//!   {"type":"result","cost_usd":0.001,"usage":{...}}
//!   ```
//!
//! The frontend has the reference implementation at
//! `frontend/app/view/agent/providers/claude-translator.ts`. This
//! module mirrors its logic on the backend so the workflow Agent
//! block can consume the same stream without round-tripping through
//! the frontend.
//!
//! PR 0 ships the SKELETON only — `translate()` accepts a frame but
//! returns an empty `Vec`. PR 1 fills in the logic and replaces
//! `agentmux-srv/src/backend/history/claude_adapter.rs` (which
//! currently has the only backend-side parsing).

use serde_json::Value;

use super::super::types::AgentEvent;
use super::Translator;

#[derive(Debug, Default)]
pub struct ClaudeTranslator {
    /// Running buffer for partial events that span multiple frames.
    /// Reserved for PR 1 wiring; not used in PR 0.
    _buffer: Vec<u8>,
}

impl ClaudeTranslator {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Translator for ClaudeTranslator {
    fn translate(&mut self, _frame: Value) -> Vec<AgentEvent> {
        // PR 1 lands the translation logic. The skeleton keeps the
        // surface area visible so PR 1 is a focused fill-in.
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn skeleton_returns_empty_for_any_frame() {
        // Regression guard — PR 1 replaces this with shape-specific
        // tests. Until then, any caller of `translate` gets an empty
        // event list, signalling "skeleton — needs implementation".
        let mut t = ClaudeTranslator::new();
        assert!(t.translate(json!({})).is_empty());
        assert!(t.translate(json!({ "type": "stream_event" })).is_empty());
    }
}
