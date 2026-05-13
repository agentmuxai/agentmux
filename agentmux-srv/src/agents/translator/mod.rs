// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Provider frame → unified `AgentEvent` translation.
//!
//! Each provider (Claude Code stream-json, ACP, future Aider /
//! Codex / Gemini) implements `Translator` to produce a stream of
//! `AgentEvent`s. The runner is provider-agnostic; only this layer
//! knows the wire format.

use super::types::AgentEvent;

pub mod claude;

/// A streaming translator: feeds raw provider frames (bytes,
/// JSON values, line strings — whatever the provider speaks) and
/// emits zero-or-more `AgentEvent`s per frame.
///
/// PR 0 ships only the trait + Claude skeleton. PR 1 fills in the
/// translation logic and adds the ACP translator (mirror of the
/// frontend `acp-translator.ts`).
pub trait Translator: Send {
    /// Frame type the provider speaks. For Claude Code stream-json
    /// this is `serde_json::Value` (one parsed line per call). For
    /// ACP it's the parsed frame struct.
    type Frame;

    /// Translate a single frame into zero-or-more events. Returning
    /// `Vec` (not `Option`) because some provider frames produce
    /// multiple events (e.g. a `message_start` with embedded
    /// `tool_use` blocks).
    fn translate(&mut self, frame: Self::Frame) -> Vec<AgentEvent>;
}
