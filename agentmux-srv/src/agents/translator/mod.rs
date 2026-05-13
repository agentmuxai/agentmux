// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Provider frame → unified `AgentEvent` translation.
//!
//! Each provider (Claude Code stream-json, ACP, future Aider /
//! Codex / Gemini) implements `Translator` to produce a stream of
//! `AgentEvent`s. The runner is provider-agnostic; only this layer
//! knows the wire format.

use serde_json::Value;

use super::types::AgentEvent;

pub mod claude;

/// A streaming translator: feeds raw provider frames as parsed JSON
/// values and emits zero-or-more `AgentEvent`s per frame.
///
/// Using a concrete `serde_json::Value` frame type (rather than an
/// associated type) keeps the trait `dyn`-dispatchable so the
/// runner can hold `Box<dyn Translator>` and switch providers at
/// run time. All currently-planned providers (Claude Code
/// stream-json, ACP, future Aider/Codex/Gemini) speak JSON over
/// stdout, so the lowest-common-denominator frame fits them.
/// Providers with binary framing would wrap their parser to emit
/// `Value` envelopes before this layer.
///
/// PR 0 ships only the trait + Claude skeleton. PR 1 fills in the
/// translation logic.
pub trait Translator: Send {
    /// Translate a single frame into zero-or-more events. Returning
    /// `Vec` (not `Option`) because some provider frames produce
    /// multiple events (e.g. a `message_start` with embedded
    /// `tool_use` blocks).
    fn translate(&mut self, frame: Value) -> Vec<AgentEvent>;
}
