// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `{{var}}` interpolation — Mustache-style. Resolves
//! `{{block_id.path}}`, `{{var.name}}`, `{{env.NAME}}` against the
//! execution context (RFC #753 §2 Q5).
//!
//! The Phase 1 resolver is intentionally simple: regex find-replace
//! over the input string. Loop scope (`{{loop.index}}`, `{{loop.item}}`)
//! is added in Phase 2 alongside the Loop block.

use std::collections::HashMap;

use serde_json::Value;

use super::state::{Reducer, RunState};
use super::value::Tv;

/// Only env vars starting with this prefix are exposed to drone
/// templates via `{{env.NAME}}`. See `ExecutionScope::lookup` for
/// the security rationale.
const DRONE_ENV_PREFIX: &str = "AGENTMUX_DR_";

/// Holds the per-run scope. Maps:
///   * `outputs[block_id]` — the JSON output of a completed block
///   * `vars[name]` — drone-scope variables (set by Variables block)
///   * `trigger` — what started the run: the input of `rundrone`, or what
///     the job passed; `Null` when nothing did (`{{trigger.*}}`)
#[derive(Debug, Default)]
pub struct ExecutionScope {
    pub outputs: HashMap<String, Value>,
    /// What `{{var.name}}` reads: a mirror of [`state`](Self::state)'s values.
    pub vars: HashMap<String, Value>,
    pub trigger: Value,
    /// The run's named values with their reducers and taint bits. Write through
    /// [`write_var`](Self::write_var), which keeps `vars` in step.
    pub state: RunState,
    /// Each block's output bits `(tainted, derived)`, set by the scheduler and
    /// by the blocks that compute one. A block with none is clean.
    pub bits: HashMap<String, (bool, bool)>,
    /// `{{item}}` and `{{index}}` while a for-each runs its body, and the bits
    /// of the item.
    pub locals: HashMap<String, Value>,
    pub local_bits: (bool, bool),
}

impl ExecutionScope {
    pub fn new() -> Self {
        Self::default()
    }

    /// A scope for a run started by `trigger`.
    pub fn with_trigger(trigger: Value) -> Self {
        Self { trigger, ..Self::default() }
    }

    /// The value a template stands for: the raw value (a list, an object, a
    /// number) when the template is one `{{...}}` token, else the text with every
    /// token filled in.
    pub fn value_of(&self, template: &str) -> Value {
        let t = template.trim();
        let one = t.strip_prefix("{{").and_then(|r| r.strip_suffix("}}")).filter(|inner| !inner.contains("{{") && !inner.contains("}}"));
        if let Some(v) = one.and_then(|inner| self.lookup(inner.trim())) {
            return v;
        }
        Value::String(self.resolve(template))
    }

    /// Declare how writes to `name` combine (`replace` when never declared).
    pub fn declare_var(&mut self, name: &str, reducer: Reducer) -> Result<(), String> {
        self.state.declare(name, reducer).map_err(|e| e.to_string())
    }

    /// Write a variable under its reducer, keeping the value `{{var.name}}` reads
    /// in step with the state. An error leaves both as they were.
    pub fn write_var(&mut self, name: &str, value: Tv) -> Result<(), String> {
        self.state.write(name, value).map_err(|e| e.to_string())?;
        if let Some(now) = self.state.get(name) {
            self.vars.insert(name.to_string(), now.v.clone());
        }
        Ok(())
    }

    /// The bits `(tainted, derived)` of a text made by filling in `template`:
    /// tainted when any value it reads is, derived when none is tainted and one
    /// is derived. The trigger is outside text when something started the run.
    pub fn bits_of(&self, template: &str) -> (bool, bool) {
        let (mut tainted, mut derived) = (false, false);
        let mut rest = template;
        while let Some(open) = rest.find("{{") {
            let Some(len) = find_close(&rest[open + 2..]) else { break };
            let path = rest[open + 2..open + 2 + len].trim();
            rest = &rest[open + 2 + len + 2..];
            let mut parts = path.split('.');
            let (t, d) = match parts.next().unwrap_or("") {
                "trigger" => (!self.trigger.is_null(), false),
                "env" => (false, false),
                "item" | "index" if self.locals.contains_key(path.split('.').next().unwrap_or("")) => self.local_bits,
                "var" | "vars" => parts.next().and_then(|name| self.state.get(name)).map(|tv| (tv.tainted, tv.derived)).unwrap_or((false, false)),
                node => self.bits.get(node).copied().unwrap_or((false, false)),
            };
            tainted |= t;
            derived |= d;
        }
        (tainted, !tainted && derived)
    }

    /// Resolve `{{...}}` tokens in `input` against this scope. Unknown
    /// tokens are left as-is (Phase 1 — Phase 2 will surface them as
    /// validation errors).
    pub fn resolve(&self, input: &str) -> String {
        self.resolve_with(input, |v| v)
    }

    /// Like [`resolve`](Self::resolve), but each resolved value goes through
    /// `wrap` before it is written, so a node can quote what came from outside
    /// (an event, an API body, an agent's reply) while its own text stays as
    /// the graph wrote it. An unresolved token is left as-is and not wrapped.
    pub fn resolve_with(&self, input: &str, wrap: impl Fn(String) -> String) -> String {
        let mut out = String::with_capacity(input.len());
        let bytes = input.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            // `{` is ASCII so the byte-level scan for `{{` is safe to
            // run at any byte offset — UTF-8 continuation bytes are
            // always >= 0x80 and never equal `{` (0x7B). The non-token
            // emit path below walks one full Unicode scalar at a time
            // to avoid splitting multi-byte sequences into separate
            // Latin-1 `char`s (mojibake).
            if i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'{' {
                // Find the matching `}}`.
                if let Some(end_rel) = find_close(&input[i + 2..]) {
                    let token = &input[i + 2..i + 2 + end_rel];
                    let resolved = self.lookup(token.trim());
                    match resolved {
                        Some(v) => out.push_str(&wrap(value_to_string(&v))),
                        None => {
                            // Leave unresolved tokens visible — easier to debug.
                            out.push_str("{{");
                            out.push_str(token);
                            out.push_str("}}");
                        }
                    }
                    i = i + 2 + end_rel + 2;
                    continue;
                }
            }
            // Copy one full Unicode scalar from `input` starting at
            // byte i. `&str` guarantees a valid char starts here.
            let ch_len = input[i..]
                .chars()
                .next()
                .expect("byte index inside &str must start a valid char")
                .len_utf8();
            out.push_str(&input[i..i + ch_len]);
            i += ch_len;
        }
        out
    }

    fn lookup(&self, path: &str) -> Option<Value> {
        // Splits like `block1.response.text` → ["block1", "response", "text"].
        let mut parts = path.split('.');
        let head = parts.next()?;
        let rest: Vec<&str> = parts.collect();

        let root = if head == "var" || head == "vars" {
            // `{{var.name.path}}` — name is the next part.
            let name = rest.first()?;
            let v = self.vars.get(*name)?;
            return Some(walk(v.clone(), &rest[1..]));
        } else if (head == "item" || head == "index") && self.locals.contains_key(head) {
            // `{{item.path}}` and `{{index}}` inside a for-each body.
            return Some(walk(self.locals.get(head)?.clone(), &rest));
        } else if head == "trigger" {
            // `{{trigger.path}}`: the run input. Missing keys resolve to
            // nothing, so the token stays visible like any unknown one.
            let mut v = &self.trigger;
            for key in &rest {
                v = v.get(*key)?;
            }
            return Some(v.clone());
        } else if head == "env" {
            // Restrict `{{env.NAME}}` to vars under the drone
            // namespace prefix. Without this guard a Response template
            // could read AWS_*, GITHUB_TOKEN, CLAUDE_API_KEY, etc. and
            // surface them via the persisted run output, exfiltrating
            // server-side secrets to any caller with drone access.
            // (reagent P1 on PR #755.) Phase 2 introduces a per-drone
            // configured allowlist; the prefix is the Phase 1 stopgap.
            let name = rest.first()?;
            if !name.starts_with(DRONE_ENV_PREFIX) {
                return None;
            }
            return std::env::var(name).ok().map(Value::String);
        } else {
            // Treat head as a block id; rest is dot-path inside its output.
            self.outputs.get(head)?
        };
        Some(walk(root.clone(), &rest))
    }
}

fn find_close(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'}' && bytes[i + 1] == b'}' {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn walk(mut v: Value, path: &[&str]) -> Value {
    for key in path {
        match v {
            Value::Object(mut m) => {
                v = m.remove(*key).unwrap_or(Value::Null);
            }
            _ => return Value::Null,
        }
    }
    v
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn resolves_block_output_path() {
        let mut scope = ExecutionScope::new();
        scope
            .outputs
            .insert("agent_1".to_string(), json!({ "response": "hello" }));
        assert_eq!(scope.resolve("Got: {{agent_1.response}}"), "Got: hello");
    }

    #[test]
    fn resolves_drone_var() {
        let mut scope = ExecutionScope::new();
        scope.vars.insert("name".to_string(), json!("world"));
        assert_eq!(scope.resolve("hi {{var.name}}"), "hi world");
    }

    #[test]
    fn resolves_trigger_path() {
        let scope = ExecutionScope::with_trigger(json!({ "run": { "conclusion": "failure", "attempt": 2 } }));
        assert_eq!(scope.resolve("{{trigger.run.conclusion}} #{{trigger.run.attempt}}"), "failure #2");
    }

    #[test]
    fn trigger_without_input_or_key_stays_visible() {
        let none = ExecutionScope::new();
        assert_eq!(none.resolve("{{trigger.x}}"), "{{trigger.x}}");
        let scope = ExecutionScope::with_trigger(json!({ "a": 1 }));
        assert_eq!(scope.resolve("{{trigger.b}}"), "{{trigger.b}}");
    }

    #[test]
    fn leaves_unknown_tokens_intact() {
        let scope = ExecutionScope::new();
        assert_eq!(scope.resolve("{{ghost.x}}"), "{{ghost.x}}");
    }

    #[test]
    fn deep_path_into_object() {
        let mut scope = ExecutionScope::new();
        scope
            .outputs
            .insert("api1".to_string(), json!({ "body": { "id": 42 } }));
        assert_eq!(scope.resolve("got id={{api1.body.id}}"), "got id=42");
    }

    #[test]
    fn preserves_text_around_tokens() {
        let mut scope = ExecutionScope::new();
        scope.vars.insert("x".to_string(), json!("X"));
        assert_eq!(scope.resolve("a {{var.x}} b {{var.x}} c"), "a X b X c");
    }

    #[test]
    fn empty_input_is_empty() {
        let scope = ExecutionScope::new();
        assert_eq!(scope.resolve(""), "");
    }

    // ────────────────────────────────────────────────────────────────
    // UTF-8 preservation (reagent P1 + codex P2 on PR #755)
    // ────────────────────────────────────────────────────────────────

    #[test]
    fn preserves_multibyte_chars_around_tokens() {
        let mut scope = ExecutionScope::new();
        scope.vars.insert("name".to_string(), json!("world"));
        // 'Olá' has a 2-byte codepoint (á = 0xC3 0xA1).
        assert_eq!(scope.resolve("Olá {{var.name}}"), "Olá world");
    }

    #[test]
    fn preserves_multibyte_token_value() {
        let mut scope = ExecutionScope::new();
        scope.vars.insert("greeting".to_string(), json!("こんにちは"));
        assert_eq!(scope.resolve("hi: {{var.greeting}}"), "hi: こんにちは");
    }

    #[test]
    fn preserves_emoji_passthrough() {
        let scope = ExecutionScope::new();
        // No tokens — pure passthrough. The pre-fix byte-wise copy
        // would turn each UTF-8 byte of 🚀 (0xF0 0x9F 0x9A 0x80) into
        // four separate Latin-1 chars; this assertion catches that.
        assert_eq!(scope.resolve("ship it 🚀"), "ship it 🚀");
    }

    #[test]
    fn preserves_multibyte_in_unresolved_token_passthrough() {
        let scope = ExecutionScope::new();
        // Unresolved tokens echo their surrounding context — make
        // sure that path doesn't corrupt the surrounding bytes either.
        assert_eq!(scope.resolve("café {{ghost}} ☕"), "café {{ghost}} ☕");
    }

    // ────────────────────────────────────────────────────────────────
    // Env-var allowlist (reagent P1 on PR #755 v0.33.841)
    // ────────────────────────────────────────────────────────────────

    #[test]
    fn env_var_allowed_under_drone_prefix() {
        let key = "AGENTMUX_DR_TEST_PASS_KEY";
        std::env::set_var(key, "hello");
        let scope = ExecutionScope::new();
        assert_eq!(
            scope.resolve(&format!("got: {{{{env.{key}}}}}")),
            "got: hello"
        );
        std::env::remove_var(key);
    }

    #[test]
    fn env_var_outside_prefix_is_blocked() {
        // Setting common secret-shaped names would let drones
        // exfiltrate them via Response output if the lookup wasn't
        // namespaced.
        let secret_key = "AWS_TEST_SECRET_DO_NOT_LEAK";
        std::env::set_var(secret_key, "TOPSECRET");
        let scope = ExecutionScope::new();
        // Without the AGENTMUX_DR_ prefix, lookup returns None —
        // the template emits the unresolved token (passthrough).
        let out = scope.resolve(&format!("leak: {{{{env.{secret_key}}}}}"));
        assert!(!out.contains("TOPSECRET"), "secret value leaked: {out}");
        std::env::remove_var(secret_key);
    }

    #[test]
    fn env_var_path_blocked() {
        // PATH is universally set; if it leaked, attackers could
        // confirm a target's directory layout. Plain `env.PATH`
        // without the prefix is rejected.
        let scope = ExecutionScope::new();
        assert_eq!(scope.resolve("{{env.PATH}}"), "{{env.PATH}}");
    }
}
