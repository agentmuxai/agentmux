// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Per-kind renderer registry — each DocumentNode kind declares its
 * component, size estimator, and streaming behavior. Phase 2 builds
 * the virtualizer config from this registry; Phase 3's perf probe
 * compares each renderer's `estimatedSize` against actual measured
 * heights to flag misses.
 *
 * See docs/specs/SPEC_AGENT_PANE_VIRTUALIZATION_REDESIGN.md
 * §"Render contract".
 */

import type {
    AgentMessageNode,
    DocumentNode,
    DocumentState,
    JektMessageNode,
    MarkdownNode,
    SectionNode,
    ShellNode,
    ToolNode,
    UserMessageNode,
} from "../types";

export type NodeKind = DocumentNode["type"];

// ── Estimator helpers ───────────────────────────────────────────────────────

/**
 * Approximate text-block height. Calibrated against the existing
 * agent-pane CSS at 14px font / 1.5 line-height ≈ 21px per line +
 * 3px gutter. Caps at 320px so a single huge message doesn't blow
 * out the initial total-size estimate.
 */
const TEXT_LINE_HEIGHT_PX = 24;
const TEXT_CHARS_PER_LINE = 80;
const TEXT_MIN_HEIGHT_PX = 32;
const TEXT_MAX_ESTIMATE_PX = 320;

export function estimateTextHeight(
    content: string,
    chars = TEXT_CHARS_PER_LINE,
    lineHeight = TEXT_LINE_HEIGHT_PX,
): number {
    if (!content) return TEXT_MIN_HEIGHT_PX;
    const lines = Math.ceil(content.length / chars);
    return Math.min(Math.max(lines * lineHeight, TEXT_MIN_HEIGHT_PX), TEXT_MAX_ESTIMATE_PX);
}

/**
 * Height estimate for unwrapped content — rows that render with
 * `white-space: pre` and horizontal scroll on overflow (no soft
 * wrap). One visual line per explicit `\n`; long single lines do
 * NOT bloat the estimate.
 *
 * Used by `estimateUserMessage` since PR #1020 — user input
 * switched to `white-space: pre` in
 * `_document-nodes.scss`, so the char-count heuristic in
 * `estimateTextHeight` would over-allocate for long URLs / paths
 * (300-char URL → 4 estimated lines vs 1 actual line) and cause
 * blank gaps / scroll jumps in the virtualized list until the
 * measure RO caught up. Codex P2 round 4 on PR #1020.
 */
export function estimateUnwrappedTextHeight(
    content: string,
    lineHeight = TEXT_LINE_HEIGHT_PX,
): number {
    if (!content) return TEXT_MIN_HEIGHT_PX;
    // Each `\n` introduces a new visual line; content with no
    // newlines is exactly 1 visual line.
    let newlines = 0;
    for (let i = 0; i < content.length; i++) {
        if (content.charCodeAt(i) === 10 /* '\n' */) newlines++;
    }
    const lines = newlines + 1;
    return Math.min(Math.max(lines * lineHeight, TEXT_MIN_HEIGHT_PX), TEXT_MAX_ESTIMATE_PX);
}

// Per-kind constants — tuned empirically. Phase 3 perf-probe HUD
// flags any kind whose p50 actual diverges > 30% from estimate.
const TOOL_COLLAPSED_PX = 32;
const TOOL_EXPANDED_PX = 200;
const SECTION_PX = 48;
const COLLAPSED_MESSAGE_PX = 32;

// ── Per-kind estimator functions ────────────────────────────────────────────
//
// Exported individually so they can be unit-tested without needing the
// real components. The full registry is constructed via
// buildRendererRegistry() at view-mount time — registry binding to
// concrete components is Phase 2's job.

export function estimateMarkdown(node: MarkdownNode): number {
    return estimateTextHeight(node.content);
}

export function estimateSection(_node: SectionNode): number {
    return SECTION_PX;
}

export function estimateTool(node: ToolNode, state: DocumentState): number {
    return state.pinnedNodes.has(node.id) ? TOOL_EXPANDED_PX : TOOL_COLLAPSED_PX;
}

export function estimateAgentMessage(node: AgentMessageNode, state: DocumentState): number {
    if (state.collapsedNodes.has(node.id)) return COLLAPSED_MESSAGE_PX;
    return estimateTextHeight(node.message);
}

export function estimateJektMessage(node: JektMessageNode, state: DocumentState): number {
    if (state.collapsedNodes.has(node.id)) return COLLAPSED_MESSAGE_PX;
    return estimateTextHeight(node.message);
}

export function estimateUserMessage(node: UserMessageNode, state: DocumentState): number {
    // Per SPEC_USER_INPUT_VISIBILITY_AND_STARTUP_COLLAPSE_2026_05_24.md,
    // user messages collapse on `isStartup` + `pinnedNodes`, NOT
    // `collapsedNodes` (which is unused for user_message nodes
    // post-PR-#1020). Mirror the rule in `estimateTool`: startup
    // payload is the one-line summary unless pinned.
    if (node.isStartup && !state.pinnedNodes.has(node.id)) {
        return COLLAPSED_MESSAGE_PX;
    }
    // Use newline-count estimation — user_message <pre> is
    // `white-space: pre` (no soft wrap), so line count comes
    // from explicit `\n`. Char-count heuristic from
    // `estimateTextHeight` would over-estimate long single lines.
    return estimateUnwrappedTextHeight(node.message);
}

const SHELL_COLLAPSED_PX = 32;
const SHELL_EXPANDED_PX = 200;

export function estimateShell(node: ShellNode, state: DocumentState): number {
    return state.pinnedNodes.has(node.id) ? SHELL_EXPANDED_PX : SHELL_COLLAPSED_PX;
}

/** Per-kind streaming capability — straightforward map. */
export const STREAMING_CAPABLE: Record<NodeKind, boolean> = {
    markdown: true,
    agent_message: true,
    section: false,
    tool: false,
    user_message: false,
    shell: false,
    agent_error: false, // fixed-content inline error — not a streaming node
    context_compacted: false,
    // One-shot announcement (the PreCompact hook fires once) — not chunked.
    compaction_started: false,
    // Arrives as a single complete user_message event, not chunk-by-chunk.
    jekt_message: false,
    // One-shot marker, same as context_compacted — not chunked.
    session_outcome: false,
    // Render-time synthetic calendar separator (Agent History view) — static.
    day_divider: false,
    // Render-time synthetic link row (live view) — static.
    history_link: false,
    // Render-time synthetic continuity notice (live view) — static. Its
    // pending/resolved swap replaces the node wholesale, never streams.
    resume_preflight: false,
};

/**
 * Dispatch helper — pick the right estimator for a given node. Used
 * by Phase 2's virtualizer config and by Phase 3's perf-probe miss
 * detector.
 */
export function estimateNode(node: DocumentNode, state: DocumentState): number {
    switch (node.type) {
        case "markdown": return estimateMarkdown(node);
        case "section": return estimateSection(node);
        case "tool": return estimateTool(node, state);
        case "agent_message": return estimateAgentMessage(node, state);
        case "jekt_message": return estimateJektMessage(node, state);
        case "user_message": return estimateUserMessage(node, state);
        case "shell": return estimateShell(node, state);
        case "agent_error":       return 64;
        case "context_compacted": return 48;
        case "compaction_started": return 32;
        case "session_outcome":   return 48;
        case "day_divider":       return 32;
        case "history_link":      return 40;
        case "resume_preflight":  return 56;
    }
}

/**
 * Per-state height estimate for a given node, independent of the
 * document's current open/collapsed signals. Used by the layout
 * slice-feeding effect to push `EstimateSet` for BOTH expansion states when a
 * node first enters the slice (INV-3: measurements are keyed by (nodeId,
 * state), so the slice needs an estimate for each state until the measure RO
 * settles a real height for it).
 *
 * Intentionally does not receive `DocumentState` — the whole point is to
 * give a size for the GIVEN state, not the rendered state. `_docState` is
 * accepted only for call-site symmetry with `estimateNode`.
 */
export function estimateNodeForState(
    node: DocumentNode,
    expansionState: "collapsed" | "expanded",
    _docState: DocumentState,
): number {
    if (expansionState === "collapsed") {
        switch (node.type) {
            case "tool":          return TOOL_COLLAPSED_PX;
            case "agent_message": return COLLAPSED_MESSAGE_PX;
            case "jekt_message":  return COLLAPSED_MESSAGE_PX;
            case "user_message":
                // Startup messages collapse; normal user input doesn't.
                return node.isStartup
                    ? COLLAPSED_MESSAGE_PX
                    : estimateUnwrappedTextHeight(node.message);
            case "section":       return SECTION_PX;
            case "markdown":
                // Canceled-thinking collapses; normal markdown stays full.
                return node.metadata?.canceled
                    ? COLLAPSED_MESSAGE_PX
                    : estimateTextHeight(node.content);
            case "shell":         return SHELL_COLLAPSED_PX;
            case "agent_error":       return 64;
            case "context_compacted": return 48;
            case "compaction_started": return 32;
            case "session_outcome":   return 48;
            case "day_divider":       return 32;
        case "history_link":      return 40;
        case "resume_preflight":  return 56;
        }
    }
    // expanded
    switch (node.type) {
        case "tool":              return TOOL_EXPANDED_PX;
        case "agent_message":     return estimateTextHeight(node.message);
        case "jekt_message":      return estimateTextHeight(node.message);
        case "user_message":      return estimateUnwrappedTextHeight(node.message);
        case "section":           return SECTION_PX;
        case "markdown":          return estimateTextHeight(node.content);
        case "shell":             return SHELL_EXPANDED_PX;
        case "agent_error":       return 64;
        case "context_compacted": return 48;
        case "compaction_started": return 32;
        case "session_outcome":   return 48;
        case "day_divider":       return 32;
        case "history_link":      return 40;
        case "resume_preflight":  return 56;
    }
}
