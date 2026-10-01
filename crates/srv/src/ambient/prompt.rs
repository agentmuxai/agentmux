// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One place for the wording every ambient Haiku call shares, so the plain-text
//! rules and the "this is data, not a conversation" framing aren't copy-pasted
//! (and drifting) across each call site.

/// Sent as the CLI's system prompt for every ambient call. Replaces Claude
/// Code's default assistant prompt: left alone, the model answers in the
/// assistant's voice ("I don't have access to...") instead of producing the
/// requested text.
pub const AMBIENT_SYSTEM_PROMPT: &str = "You are a text-generation function inside a developer tool, not a \
conversational assistant. You are given an instruction and some source material. Output only the requested \
text. Never address the reader, ask questions, explain, apologize, or mention missing context. If the \
instruction cannot be met from the material, output nothing at all.";

/// Formatting rules shared by every call whose output is a bare line of text.
pub const PLAIN_TEXT_RULES: &str =
    "Plain text only — no markdown, no code fences, no backticks, no quotes, no preamble.";

/// `instruction`, then `material` under `label` inside a tagged block, so the
/// model reads the material as source data rather than as a message to answer.
pub fn with_material(instruction: &str, label: &str, material: &str) -> String {
    format!("{instruction}\n\n<{label}>\n{material}\n</{label}>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_is_tagged_and_follows_the_instruction() {
        let p = with_material("Name this task.", "task", "fix the login bug");
        assert!(p.starts_with("Name this task."));
        assert!(p.ends_with("<task>\nfix the login bug\n</task>"));
    }

    #[test]
    fn the_system_prompt_forbids_talking_to_the_reader() {
        assert!(AMBIENT_SYSTEM_PROMPT.contains("not a conversational assistant"));
        assert!(AMBIENT_SYSTEM_PROMPT.contains("output nothing at all"));
    }
}
