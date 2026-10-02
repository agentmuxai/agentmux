// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { isUsableTitle } from "./ambient-title";


/**
 * Shared precedence rule for an agent pane's "what's it doing" label.
 *
 * Two independent sources write to a block's meta:
 *   - `term:ambient_summary` — Haiku-derived, per-turn paraphrase (Ambient
 *     Model Call gateway; see useAgentActivitySummary.ts).
 *   - `term:osc_title` — free, CLI-emitted OSC window-title topic, no LLM
 *     call of our own (see useBlockActivity.ts / termosc.ts).
 *
 * These used to share one meta key (`term:activity`) with no ownership
 * protocol — last write won, regardless of which source was actually more
 * current. This is the single place both readers (agent-model.ts,
 * swarm-model.ts) resolve precedence, so it can't drift between them again.
 * See docs/specs/SPEC_AMBIENT_MODEL_CALLS_FRAMEWORK_2026_07_03.md §3.4.
 */
export function readActivitySummary(meta: Record<string, unknown> | undefined): string | undefined {
    const ambient = meta?.["term:ambient_summary"] as string | undefined;
    // A stored value that is about the ABSENCE of a title (`(none yet)`, the title
    // prompt's own placeholder echoed back and stored by an older build) is not shown:
    // it falls through to the OSC title, or to nothing. See ambient-title.ts.
    if (ambient && isUsableTitle(ambient)) return ambient;
    const oscTitle = meta?.["term:osc_title"] as string | undefined;
    if (oscTitle && oscTitle.length > 0) return oscTitle;
    return undefined;
}

/**
 * The line the Swarm row shows for an agent, and the line a pane tab's tooltip
 * shows: ONE function so the two can never disagree. `null` when there is nothing
 * usable (the swarm hides the line; the tooltip shows just the agent's name).
 */
export function readSwarmSummary(meta: Record<string, unknown> | undefined): string | null {
    return readActivitySummary(meta)?.trim() || null;
}
