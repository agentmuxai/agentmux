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

/// Build the session-goal-title prompt for `session:activity_summary`.
/// Explicitly PR-title-style and stability-biased: the previous "what is
/// currently being worked on" prompt regenerated from a blank slate every
/// call (no memory of the prior title, no access to the original ask), so
/// it thrashed between micro-steps instead of tracking the session's
/// overall goal. Extracted as a pure function so the prompt shape itself
/// (both fields embedded, correct fallback text) is unit-testable without
/// spinning up the full RPC handler. See
/// docs/specs/SPEC_AMBIENT_PANE_TITLE_OVERALL_GOAL_TRACKING_2026_08_17.md.
pub fn build_session_title_prompt(current_title: &str, user_message: Option<&str>, word_target: u32) -> String {
    let current_title_display = if current_title.is_empty() { "(none yet)" } else { current_title };
    let user_message_display =
        user_message.unwrap_or("(no new message — re-evaluate from the title alone)");

    format!(
        "You maintain a short running TITLE for this work session, similar to a git \
         pull-request title — it describes the OVERALL GOAL of the session, not the \
         current micro-step or the most recent tool call.\n\n\
         Current title: {current_title_display}\n\n\
         The user just said:\n{user_message_display}\n\n\
         Decide: does this message represent a genuinely NEW or EXPANDED top-level \
         goal, or is it a continuation, follow-up, clarification, correction, or a \
         step within the SAME goal the current title already describes?\n\n\
         - If the current title still accurately describes the overall goal, repeat \
         it back EXACTLY, unchanged.\n\
         - Otherwise, output an updated title covering the (possibly still-in-progress) \
         overall goal, in {word_target} words or fewer.\n\n\
         Plain text only — no markdown, no code fences, no backticks, no quotes, \
         no punctuation, no preamble."
    )
}

/// The prompt for `session:next_prompt_suggestion`. A pure function so its shape
/// is unit-testable. The activity goes in a tagged block and the model is told
/// when to say nothing: asked to guess with too little to go on, it answered as
/// an assistant, and that text reached the composer.
pub fn build_next_prompt_prompt(digest: &str) -> String {
    let instruction = [
        "Predict the ONE short instruction the user will most likely type next to continue this work, using only the activity below.",
        "Write it the way someone types a task into a prompt box: a direct imperative that begins with the action itself (\"Debug the blank preview bug\", not \"Yeah, let's debug the blank preview bug\"). One line, under 20 words.",
        "Output nothing at all if any of these hold: the activity doesn't show what the work is; the assistant's last message asks the user a question or waits for a decision; the work looks finished with nothing obvious left.",
        "Never suggest deleting data, force-pushing, or touching credentials.",
        crate::ambient::prompt::PLAIN_TEXT_RULES,
    ]
    .join(" ");
    crate::ambient::prompt::with_material(&instruction, "recent_activity", digest)
}

/// Build the prompt for one narration `kind`.
///
/// `kind` is what makes this a general facility rather than a
/// background-task-specific one: adding a narrated action means adding an arm
/// here, not new plumbing. Returns `None` for an unrecognised kind so an
/// unknown caller is a no-op rather than an unconstrained prompt.
pub fn narration_prompt(kind: &str, context: &str) -> Option<String> {
    match kind {
        // The first consumer: a long-running tool call the harness detached.
        // The pane goes quiet at that moment and the user is told nothing.
        "background_task" => Some(with_material(
            &[
                "One sentence, first person, past tense, telling the user you have started a command in the background.",
                "Name the command briefly. Do NOT promise to report back, and do NOT claim it is still running — it may already have finished by the time this is read.",
                PLAIN_TEXT_RULES,
                "Under 20 words.",
            ]
            .join(" "),
            "command",
            context,
        )),
        _ => None,
    }
}

/// Per-turn activity summary pushed by the background sweep.
pub fn build_activity_summary_prompt(word_target: u32, digest: &str) -> String {
    with_material(
        &format!(
            "Summarize in {word_target} words or fewer what is currently being worked on. {PLAIN_TEXT_RULES} No punctuation."
        ),
        "recent_activity",
        digest,
    )
}

/// The one-shot preview for a definition with no structured snapshot.
pub fn build_definition_summary_prompt(digest: &str) -> String {
    with_material(
        &format!(
            "Summarize in 12 words or fewer what this conversation/session was about, based on the activity below. {PLAIN_TEXT_RULES} No punctuation at the end."
        ),
        "recent_activity",
        digest,
    )
}

/// A subagent's display name, from its own task prompt.
pub fn build_subagent_name_prompt(task_prompt: &str) -> String {
    with_material(
        &format!("Give a concise ~5-word name for this task. {PLAIN_TEXT_RULES} No punctuation. Respond with just the name."),
        "task",
        task_prompt,
    )
}

/// A Workflow dispatch's display name, from its first member's task prompt.
pub fn build_dispatch_name_prompt(task_prompt: &str) -> String {
    with_material(
        &format!(
            "Give a concise ~5-word name for this workflow batch, based on its first task. {PLAIN_TEXT_RULES} No punctuation. Respond with just the name."
        ),
        "task",
        task_prompt,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_narration_kind_produces_no_prompt_rather_than_an_open_ended_one() {
        assert!(narration_prompt("no_such_kind", "anything").is_none());
    }

    #[test]
    fn the_background_task_kind_carries_the_command_into_the_prompt() {
        let p = narration_prompt("background_task", "task dev").expect("known kind");
        assert!(p.contains("<command>\ntask dev\n</command>"));
        // The constraints that keep the line short and renderable as plain
        // text in a conversation row.
        assert!(p.contains("One sentence"));
        assert!(p.contains("no markdown"));
        assert!(p.contains("Under 20 words"));
    }

    #[test]
    fn every_summary_and_name_prompt_tags_its_material_and_states_the_plain_text_rules() {
        let prompts = [
            build_activity_summary_prompt(7, "[user] fix login"),
            build_definition_summary_prompt("[user] fix login"),
            build_subagent_name_prompt("fix login"),
            build_dispatch_name_prompt("fix login"),
        ];
        for p in prompts {
            assert!(p.contains(PLAIN_TEXT_RULES), "{p}");
            assert!(p.contains("fix login\n</"), "material not in a tagged block: {p}");
        }
        assert!(build_activity_summary_prompt(7, "x").contains("in 7 words or fewer"));
        assert!(build_definition_summary_prompt("x").contains("12 words or fewer"));
        assert!(build_subagent_name_prompt("x").contains("~5-word name for this task"));
        assert!(build_dispatch_name_prompt("x").contains("workflow batch"));
    }

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

#[cfg(test)]
mod build_session_title_prompt_tests {
    use super::*;

    #[test]
    fn embeds_both_fields_when_present() {
        let prompt = build_session_title_prompt("invert user input styling", Some("also fix the lint warning"), 7);
        assert!(prompt.contains("Current title: invert user input styling"));
        assert!(prompt.contains("The user just said:\nalso fix the lint warning"));
        assert!(prompt.contains("in 7 words or fewer"));
    }

    #[test]
    fn falls_back_to_none_yet_for_an_empty_current_title() {
        let prompt = build_session_title_prompt("", Some("build a login page"), 7);
        assert!(prompt.contains("Current title: (none yet)"));
    }

    #[test]
    fn falls_back_to_a_placeholder_for_a_missing_user_message() {
        let prompt = build_session_title_prompt("invert user input styling", None, 7);
        assert!(prompt.contains("The user just said:\n(no new message — re-evaluate from the title alone)"));
    }

    #[test]
    fn instructs_stability_over_regeneration() {
        // The core behavior change: the prompt must explicitly bias toward
        // KEEPING the current title, not regenerating fresh every call —
        // this is what the old "what is currently being worked on" prompt
        // never asked for. docs/specs/SPEC_AMBIENT_PANE_TITLE_OVERALL_GOAL_TRACKING_2026_08_17.md.
        let prompt = build_session_title_prompt("invert user input styling", Some("continue"), 7);
        assert!(prompt.contains("repeat it back EXACTLY, unchanged"));
        assert!(prompt.contains("OVERALL GOAL"));
        assert!(!prompt.contains("what is currently being worked on"));
    }
}

#[cfg(test)]
mod build_next_prompt_prompt_tests {
    use super::*;

    #[test]
    fn the_activity_is_tagged_material_after_the_instruction() {
        let p = build_next_prompt_prompt("[user] fix the login bug");
        assert!(p.contains("<recent_activity>
[user] fix the login bug
</recent_activity>"));
        assert!(p.find("Predict the ONE").unwrap() < p.find("<recent_activity>").unwrap());
    }

    #[test]
    fn it_says_when_to_stay_silent() {
        let p = build_next_prompt_prompt("[user] x");
        assert!(p.contains("Output nothing at all"));
        assert!(p.contains("asks the user a question"));
        assert!(p.contains("looks finished"));
    }

    #[test]
    fn it_forbids_risky_suggestions_and_filler() {
        let p = build_next_prompt_prompt("[user] x");
        assert!(p.contains("force-pushing"));
        assert!(p.contains("not \"Yeah, let's debug the blank preview bug\""));
    }
}
