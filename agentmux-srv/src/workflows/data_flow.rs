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

/// Holds the per-run scope. Maps:
///   * `outputs[block_id]` — the JSON output of a completed block
///   * `vars[name]` — workflow-scope variables (set by Variables block)
#[derive(Debug, Default)]
pub struct ExecutionScope {
    pub outputs: HashMap<String, Value>,
    pub vars: HashMap<String, Value>,
}

impl ExecutionScope {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve `{{...}}` tokens in `input` against this scope. Unknown
    /// tokens are left as-is (Phase 1 — Phase 2 will surface them as
    /// validation errors).
    pub fn resolve(&self, input: &str) -> String {
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
                        Some(v) => out.push_str(&value_to_string(&v)),
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
        } else if head == "env" {
            let name = rest.first()?;
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
    fn resolves_workflow_var() {
        let mut scope = ExecutionScope::new();
        scope.vars.insert("name".to_string(), json!("world"));
        assert_eq!(scope.resolve("hi {{var.name}}"), "hi world");
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
}
