// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Gateway purpose tags: the second half of an `AmbientCallKey`, and the
//! token-usage / cost-dashboard category for each call site. Each caller keeps
//! its OWN purpose: two callers under one purpose cancel each other.

/// Ambient-call purpose tag for the per-turn activity summary. Also usable
/// as the token-usage/cost-dashboard category for this call site.
pub const ACTIVITY_SUMMARY: &str = "activity_summary";

/// Purpose tag for the background/pushed activity summary (the swarm-feed
/// sweep in `backend::reactive::activity_watcher`) — distinct from
/// `AMBIENT_PURPOSE_ACTIVITY_SUMMARY` so the two never contend for the same
/// Ambient Model Call gateway slot. A periodic background summary should not
/// cancel, or be cancelled by, a live user-facing pane-header request for
/// the same block.
pub const ACTIVITY_SUMMARY_PUSHED: &str = "activity_summary_pushed";

/// Ambient-call purpose tag for the on-demand, once-per-definition activity
/// summary used as the AgentPicker's "My Agents" conversation-preview
/// fallback — see `generate_definition_activity_summary` below and
/// `db_agent_activity_summaries` (OBJECT_SCHEMA_VERSION v28).
pub const DEFINITION_SUMMARY: &str = "definition_summary";

/// Ambient-call purpose tag for the on-demand subagent display name (see
/// `generate_subagent_name`). One-shot per subagent — `generation` is always
/// the constant `1` since a name, once generated, is cached on
/// `SubAgent.display_name` and never regenerated; there is no "newer
/// turn" to supersede an in-flight naming call the way there is for the
/// per-turn pull RPCs above.
pub const SUBAGENT_NAME: &str = "subagent_name";

/// Ambient-call purpose tag for eager Workflow-dispatch naming — distinct
/// from `AMBIENT_PURPOSE_SUBAGENT_NAME` so cost-dashboard tagging can tell
/// the two apart even though they share the same gateway/semaphore. See
/// docs/specs/SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19.md.
pub const DISPATCH_NAME: &str = "dispatch_name";

/// Ambient-call purpose tag for the ghost-text next-prompt suggestion. See
/// docs/specs/SPEC_AMBIENT_GHOST_TEXT_NEXT_PROMPT_2026_07_03.md.
pub const NEXT_PROMPT_SUGGESTION: &str = "next_prompt_suggestion";

/// Ambient-call purpose for narrating an autonomous AgentMux action back to the
/// user in the pane's own conversation. Its OWN purpose constant, not shared
/// with any summary caller: two callers under one purpose cancel each other
/// (see `AMBIENT_PURPOSE_ACTIVITY_SUMMARY_PUSHED`'s doc comment above for the
/// case that motivated splitting them).
pub const NARRATION: &str = "ambient_narration";
