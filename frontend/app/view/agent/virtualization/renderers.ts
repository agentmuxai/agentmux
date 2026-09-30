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
    ContextDeliveryNode,
    DocumentNode,
    DocumentState,
    JektMessageNode,
    MarkdownNode,
    ShellNode,
    ToolNode,
    UserMessageNode,
} from "../types";
import { isContentFirstTool } from "../tool-meta/tool-descriptors";
import { rowDisclosureIn } from "./disclosure";

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
    max = TEXT_MAX_ESTIMATE_PX,
): number {
    if (!content) return TEXT_MIN_HEIGHT_PX;
    const lines = Math.ceil(content.length / chars);
    return Math.min(Math.max(lines * lineHeight, TEXT_MIN_HEIGHT_PX), max);
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
const COLLAPSED_MESSAGE_PX = 32;

/**
 * The CSS preview cap, `$transcript-preview-max-height: calc(50vh / 3)`, in
 * unzoomed CSS px (≈ 233 on a 1400 px window). Every capped row's expanded
 * estimate is this plus that row's chrome, so the estimates follow the window
 * height the way the rendered boxes do
 * (SPEC_AGENT_PANE_PREVIEW_CLEANUPS_2026_09_26.md §3). Falls back to a
 * 1400 px window where there is none.
 */
export function previewCapPx(): number {
    const h = typeof window !== "undefined" && window.innerHeight > 0 ? window.innerHeight : 1400;
    return h / 6;
}

/** A pinned tool's panel: its typical 200 px, but never more than the cap
 *  plus the header allows (a small window). */
export function toolExpandedPx(): number {
    return Math.min(200, Math.round(previewCapPx() + 40));
}

// ── Per-kind estimator functions ────────────────────────────────────────────
//
// Exported individually so they can be unit-tested without needing the
// real components. The full registry is constructed via
// buildRendererRegistry() at view-mount time — registry binding to
// concrete components is Phase 2's job.

export function estimateMarkdown(node: MarkdownNode): number {
    return estimateTextHeight(node.content);
}

/**
 * A content-first tool (WebSearch) renders expanded by default: the capped
 * body plus the header row and padding (280 at a 1400 px window). The
 * measured height replaces it on render.
 */
export function contentFirstToolEstimatePx(): number {
    return Math.round(previewCapPx() + 47);
}

/**
 * An expanded jekt's body is capped by CSS and scrolls inside its box, so a
 * long message no longer needs `TEXT_MAX_ESTIMATE_PX`: the cap plus the
 * summary and metadata lines around the body (290 at a 1400 px window). An
 * approximation is enough; the measured height replaces it once the row
 * renders.
 */
export function jektExpandedMaxEstimatePx(): number {
    return Math.round(previewCapPx() + 57);
}

export function estimateExpandedJekt(message: string): number {
    // The jekt cap bounds it, not the text estimate's own 320 px ceiling —
    // on a tall window the cap is higher (Codex P2 on #3934).
    return estimateTextHeight(message, TEXT_CHARS_PER_LINE, TEXT_LINE_HEIGHT_PX, jektExpandedMaxEstimatePx());
}

const SHELL_COLLAPSED_PX = 32;

const CONTEXT_DELIVERY_TITLE_PX = 30;
const CONTEXT_DELIVERY_EXCERPT_PX = 22;
const CONTEXT_DELIVERY_ITEM_HEAD_PX = 24;
const CONTEXT_DELIVERY_ADVICE_PX = 40;

/**
 * A compaction summary card: title plus excerpt. A memory card: title plus a
 * row per item (always shown), plus the size advice when Personal Memory is large.
 */
function estimateCollapsedContextDelivery(node: ContextDeliveryNode): number {
    const summaryCard = node.items.length === 1 && node.items[0].kind === "compaction_summary";
    const rows = summaryCard ? CONTEXT_DELIVERY_EXCERPT_PX : node.items.length * CONTEXT_DELIVERY_ITEM_HEAD_PX;
    const advice = node.sizeBand === "high" || node.sizeBand === "critical" ? CONTEXT_DELIVERY_ADVICE_PX : 0;
    return CONTEXT_DELIVERY_TITLE_PX + rows + advice;
}

/** Collapsed, then each item with text: its head row and its body, capped like a jekt's. */
function estimateExpandedContextDelivery(node: ContextDeliveryNode): number {
    return node.items.reduce(
        (sum, item) => sum + (item.body ? CONTEXT_DELIVERY_ITEM_HEAD_PX + estimateExpandedJekt(item.body) : 0),
        estimateCollapsedContextDelivery(node),
    );
}
const SHELL_EXPANDED_PX = 200;

/** Per-kind streaming capability — straightforward map. */
export const STREAMING_CAPABLE: Record<NodeKind, boolean> = {
    markdown: true,
    agent_message: true,
    tool: false,
    user_message: false,
    shell: false,
    agent_error: false, // fixed-content inline error — not a streaming node
    context_compacted: false,
    // One-shot announcement (the PreCompact hook fires once) — not chunked.
    compaction_started: false,
    // One-shot label built from a completed compact_boundary — never chunked, and never carries content to stream in the first place (§3.2 of its spec).
    memory_reinjection: false,
    // Built whole from one frame (the compaction summary) — never chunked.
    context_delivery: false,
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
    // Complete one-line narration delivered as a single broadcast — not chunked.
    ambient_narration: false,
};

/**
 * Dispatch helper — pick the right estimator for a given node. Used
 * by Phase 2's virtualizer config and by Phase 3's perf-probe miss
 * detector.
 */
export function estimateNode(node: DocumentNode, state: DocumentState): number {
    // The same open/closed answer the rows render and the layout slice uses
    // (disclosure.ts), not a per-kind copy of it.
    return estimateNodeForState(node, rowDisclosureIn(node, state).open ? "expanded" : "collapsed", state);
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
            case "ambient_narration": return estimateTextHeight(node.text);
            case "context_delivery":  return estimateCollapsedContextDelivery(node);
        }
    }
    // expanded
    switch (node.type) {
        // A content-first tool's body is capped at the preview height, like a
        // jekt's; everything else pinned open uses the generic tool size.
        case "tool":              return isContentFirstTool(node) ? contentFirstToolEstimatePx() : toolExpandedPx();
        case "agent_message":     return estimateTextHeight(node.message);
        case "jekt_message":      return estimateExpandedJekt(node.message);
        case "user_message":      return estimateUnwrappedTextHeight(node.message);
        case "markdown":          return estimateTextHeight(node.content);
        case "shell":             return SHELL_EXPANDED_PX;
        case "agent_error":       return 64;
        case "context_compacted": return 48;
        case "compaction_started": return 32;
        case "session_outcome":   return 48;
        case "day_divider":       return 32;
        case "history_link":      return 40;
        case "resume_preflight":  return 56;
        case "ambient_narration": return estimateTextHeight(node.text);
        case "context_delivery":  return estimateExpandedContextDelivery(node);
    }
}
