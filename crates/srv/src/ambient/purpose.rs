// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Every ambient purpose, in one place: its tag (the second half of an
//! `AmbientCallKey`, the outcome-log purpose and the token-usage category) and how
//! long its CLI call may run. Each caller keeps its OWN purpose: two callers under
//! one purpose cancel each other.
//!
//! docs/reports/REPORT_AMBIENT_FRAMEWORK_REASSESSMENT_2026_10_08.md section 6.3.

use std::time::Duration;

/// One kind of ambient call.
#[derive(Debug, PartialEq, Eq)]
pub struct Purpose {
    pub tag: &'static str,
    /// The CLI call's time limit, spawn to exit.
    pub timeout: Duration,
}

/// The limit for a one-line reply on a user-facing path.
const LINE_TIMEOUT: Duration = Duration::from_secs(15);

/// The per-turn session title the pane requests (`session:activity_summary`).
pub const ACTIVITY_SUMMARY: Purpose = Purpose { tag: "activity_summary", timeout: LINE_TIMEOUT };

/// A missing session title, recovered by the background sweep
/// (`backend::reactive::activity_watcher`). Its own purpose so it never cancels,
/// or is cancelled by, the pane's own title request.
pub const ACTIVITY_SUMMARY_PUSHED: Purpose = Purpose { tag: "activity_summary_pushed", timeout: LINE_TIMEOUT };

/// The once-per-definition conversation preview for the AgentPicker's "My Agents"
/// list (`tasks::generate_definition_activity_summary`).
pub const DEFINITION_SUMMARY: Purpose = Purpose { tag: "definition_summary", timeout: LINE_TIMEOUT };

/// A subagent's display name (`tasks::generate_subagent_name`). One-shot: the name
/// is cached on `SubAgent.display_name`, so its generation is always `1`.
pub const SUBAGENT_NAME: Purpose = Purpose { tag: "subagent_name", timeout: LINE_TIMEOUT };

/// A Workflow dispatch's display name. Separate from `SUBAGENT_NAME` so the two
/// are counted apart. docs/specs/SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19.md.
pub const DISPATCH_NAME: Purpose = Purpose { tag: "dispatch_name", timeout: LINE_TIMEOUT };

/// The composer's ghost-text next-prompt suggestion.
/// docs/specs/SPEC_AMBIENT_GHOST_TEXT_NEXT_PROMPT_2026_07_03.md.
pub const NEXT_PROMPT_SUGGESTION: Purpose = Purpose { tag: "next_prompt_suggestion", timeout: LINE_TIMEOUT };

/// A line narrating an autonomous AgentMux action in the pane's conversation.
pub const NARRATION: Purpose = Purpose { tag: "ambient_narration", timeout: LINE_TIMEOUT };

/// The rolling continuity state (`backend::continuity_state`): off any
/// user-facing path, and a long multi-section reply, so it may take longer.
pub const CONTINUITY_STATE: Purpose = Purpose { tag: "continuity_state", timeout: Duration::from_secs(90) };

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
}
