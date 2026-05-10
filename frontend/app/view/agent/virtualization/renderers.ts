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

import type { Component } from "solid-js";
import type {
    AgentMessageNode,
    DocumentNode,
    DocumentState,
    MarkdownNode,
    SectionNode,
    SubagentLinkNode,
    ToolNode,
    UserMessageNode,
} from "../types";

export type NodeKind = DocumentNode["type"];

/** Map a NodeKind back to its concrete DocumentNode subtype. */
export type NodeOf<K extends NodeKind> = Extract<DocumentNode, { type: K }>;

export interface NodeKindRenderer<K extends NodeKind = NodeKind> {
    /** SolidJS component that renders this kind. Must access props
     *  reactively (no destructuring) so prop changes propagate. */
    component: Component<{ node: NodeOf<K>; state: DocumentState }>;
    /**
     * Initial height estimate in pixels. Used until measureElement
     * settles each row to its real size. Should be close to the p50
     * actual height for the kind — Phase 3's perf probe surfaces
     * estimator misses > 30% in the dev HUD so we can recalibrate.
     */
    estimatedSize: (node: NodeOf<K>, state: DocumentState) => number;
    /**
     * True if this kind receives content chunk-by-chunk during a
     * stream (markdown, agent_message). Drives the streaming-buffer
     * pin so the row isn't recycled mid-stream.
     */
    isStreamingCapable: boolean;
    /**
     * Rare: render in a dedicated layer above/below the virtualized
     * region instead of in the list. Reserved for future affordances
     * (e.g., always-visible auth box). null = normal list item.
     */
    pinnedLayer?: "top" | "bottom";
}

export type NodeRendererRegistry = {
    [K in NodeKind]: NodeKindRenderer<K>;
};

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

// Per-kind constants — tuned empirically. Phase 3 perf-probe HUD
// flags any kind whose p50 actual diverges > 30% from estimate.
const TOOL_COLLAPSED_PX = 32;
const TOOL_EXPANDED_PX = 200;
const SECTION_PX = 48;
const SUBAGENT_LINK_PX = 56;
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

export function estimateUserMessage(node: UserMessageNode, state: DocumentState): number {
    if (state.collapsedNodes.has(node.id)) return COLLAPSED_MESSAGE_PX;
    return estimateTextHeight(node.message);
}

export function estimateSubagentLink(_node: SubagentLinkNode): number {
    return SUBAGENT_LINK_PX;
}

/** Per-kind streaming capability — straightforward map. */
export const STREAMING_CAPABLE: Record<NodeKind, boolean> = {
    markdown: true,
    agent_message: true,
    section: false,
    tool: false,
    user_message: false,
    subagent_link: false,
};

// ── Registry factory ────────────────────────────────────────────────────────

export interface RendererComponents {
    Markdown: Component<{ node: MarkdownNode; state: DocumentState }>;
    Section: Component<{ node: SectionNode; state: DocumentState }>;
    Tool: Component<{ node: ToolNode; state: DocumentState }>;
    AgentMessage: Component<{ node: AgentMessageNode; state: DocumentState }>;
    UserMessage: Component<{ node: UserMessageNode; state: DocumentState }>;
    SubagentLink: Component<{ node: SubagentLinkNode; state: DocumentState }>;
}

/**
 * Build the registry by binding components to their estimators.
 * Phase 2 will wire concrete components from `../components/`; tests
 * pass stub components.
 */
export function buildRendererRegistry(components: RendererComponents): NodeRendererRegistry {
    return {
        markdown: {
            component: components.Markdown,
            estimatedSize: estimateMarkdown,
            isStreamingCapable: STREAMING_CAPABLE.markdown,
        },
        section: {
            component: components.Section,
            estimatedSize: estimateSection,
            isStreamingCapable: STREAMING_CAPABLE.section,
        },
        tool: {
            component: components.Tool,
            estimatedSize: estimateTool,
            isStreamingCapable: STREAMING_CAPABLE.tool,
        },
        agent_message: {
            component: components.AgentMessage,
            estimatedSize: estimateAgentMessage,
            isStreamingCapable: STREAMING_CAPABLE.agent_message,
        },
        user_message: {
            component: components.UserMessage,
            estimatedSize: estimateUserMessage,
            isStreamingCapable: STREAMING_CAPABLE.user_message,
        },
        subagent_link: {
            component: components.SubagentLink,
            estimatedSize: estimateSubagentLink,
            isStreamingCapable: STREAMING_CAPABLE.subagent_link,
        },
    };
}

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
        case "user_message": return estimateUserMessage(node, state);
        case "subagent_link": return estimateSubagentLink(node);
    }
}
