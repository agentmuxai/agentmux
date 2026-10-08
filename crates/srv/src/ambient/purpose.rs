// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Every ambient purpose, in one place: its tag (the second half of an
//! `AmbientCallKey`, the outcome-log purpose and the token-usage category), how
//! long its CLI call may run, and which queue it waits in. Each caller keeps its OWN purpose: two callers under
//! one purpose cancel each other.
//!
//! docs/reports/REPORT_AMBIENT_FRAMEWORK_REASSESSMENT_2026_10_08.md section 6.3.

use std::time::Duration;

use super::limits::Class;

/// One kind of ambient call.
#[derive(Debug, PartialEq, Eq)]
pub struct Purpose {
    pub tag: &'static str,
    /// The CLI call's time limit, spawn to exit.
    pub timeout: Duration,
    /// Whether the outcome log may carry the reply's text. Off for a purpose whose
    /// reply restates the conversation itself (the continuity state), which must
    /// not be copied into the srv log.
    pub logs_reply: bool,
    /// Which queue its calls wait in (`limits`).
    pub class: Class,
}

/// The time limits. A call that answers in about 1.5 s on a quiet account can
/// take much longer when many agents share one login and the CLI retries on
/// overload, and a reply cut off at the limit has been paid for and is thrown
/// away. 15 s timed out a third of suggestion calls. A suggestion or a title
/// that arrives after 25 s is still useful.
const INTERACTIVE_TIMEOUT: Duration = Duration::from_secs(30);
const BACKGROUND_TIMEOUT: Duration = Duration::from_secs(45);

/// The per-turn session title the pane requests (`session:activity_summary`).
pub const ACTIVITY_SUMMARY: Purpose =
    Purpose { tag: "activity_summary", timeout: INTERACTIVE_TIMEOUT, class: Class::Interactive, logs_reply: true };

/// A missing session title, recovered by the background sweep
/// (`backend::reactive::activity_watcher`). Its own purpose so it never cancels,
/// or is cancelled by, the pane's own title request.
pub const ACTIVITY_SUMMARY_PUSHED: Purpose =
    Purpose { tag: "activity_summary_pushed", timeout: BACKGROUND_TIMEOUT, class: Class::Background, logs_reply: true };

/// The once-per-definition conversation preview for the AgentPicker's "My Agents"
/// list (`tasks::generate_definition_activity_summary`).
pub const DEFINITION_SUMMARY: Purpose =
    Purpose { tag: "definition_summary", timeout: BACKGROUND_TIMEOUT, class: Class::Background, logs_reply: true };

/// A subagent's display name (`tasks::generate_subagent_name`), when its parent
/// gave no description. One-shot: the name is cached on `SubAgent.display_name`,
/// so its generation is always `1`. The backlog pass runs it as background work.
pub const SUBAGENT_NAME: Purpose =
    Purpose { tag: "subagent_name", timeout: INTERACTIVE_TIMEOUT, class: Class::Interactive, logs_reply: true };

/// A Workflow dispatch's display name. Separate from `SUBAGENT_NAME` so the two
/// are counted apart. docs/specs/SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19.md.
pub const DISPATCH_NAME: Purpose =
    Purpose { tag: "dispatch_name", timeout: INTERACTIVE_TIMEOUT, class: Class::Interactive, logs_reply: true };

/// The composer's ghost-text next-prompt suggestion.
/// docs/specs/SPEC_AMBIENT_GHOST_TEXT_NEXT_PROMPT_2026_07_03.md.
pub const NEXT_PROMPT_SUGGESTION: Purpose =
    Purpose { tag: "next_prompt_suggestion", timeout: INTERACTIVE_TIMEOUT, class: Class::Interactive, logs_reply: true };

/// A line narrating an autonomous AgentMux action in the pane's conversation.
pub const NARRATION: Purpose =
    Purpose { tag: "ambient_narration", timeout: BACKGROUND_TIMEOUT, class: Class::Background, logs_reply: true };

/// The rolling continuity state (`backend::continuity_state`): off any
/// user-facing path, and a long multi-section reply, so it may take longer.
pub const CONTINUITY_STATE: Purpose =
    Purpose { tag: "continuity_state", timeout: Duration::from_secs(90), class: Class::Background, logs_reply: false };

/// Every purpose, for anything that lists them.
pub const ALL: &[&Purpose] = &[
    &ACTIVITY_SUMMARY,
    &ACTIVITY_SUMMARY_PUSHED,
    &DEFINITION_SUMMARY,
    &SUBAGENT_NAME,
    &DISPATCH_NAME,
    &NEXT_PROMPT_SUGGESTION,
    &NARRATION,
    &CONTINUITY_STATE,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_purpose_has_its_own_tag() {
        let mut tags: Vec<&str> = ALL.iter().map(|p| p.tag).collect();
        tags.sort_unstable();
        tags.dedup();
        assert_eq!(tags.len(), ALL.len(), "two purposes share a tag, so they would cancel each other");
    }

    /// The continuity state restates the conversation (its goal, the latest
    /// request, exact identifiers): its text never goes to the srv log (#4501).
    #[test]
    fn only_the_continuity_state_keeps_its_reply_out_of_the_log() {
        let quiet: Vec<&str> = ALL.iter().filter(|p| !p.logs_reply).map(|p| p.tag).collect();
        assert_eq!(quiet, vec![CONTINUITY_STATE.tag]);
    }
}
