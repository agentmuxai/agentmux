// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ToolBlock - Single-line collapsed-by-default tool display with
 * hover-to-expand and click-to-pin semantics.
 *
 * See docs/specs/tool-collapse.md for the product requirement.
 *
 * Behavior:
 *   - Collapsed (default): one line showing status icon + tool name + ellipsis.
 *     Applies to ALL statuses — running, success, and failed.
 *     Running tools show a ⏳ spinner in the summary; failed tools show ✗.
 *     No content body is rendered in the document flow.
 *   - Click summary or strip Expand button: pins the expanded state. The portal
 *     overlay renders the tool content. Clicking again unpins / collapses.
 *
 * Prior behavior removed in SPEC_AGENT_PANE_FOLLOWUPS items #4 + #5:
 *   running and failed states used to force-expand inline, taking 2+ lines
 *   per tool. That violated the explicit one-line rule in
 *   docs/specs/tool-collapse.md and the user's feedback memory. Now all
 *   statuses collapse by default; progress and error content are still
 *   accessible via hover/pin.
 *
 * SolidJS reactivity note:
 *   Props are accessed via `props.X` (never destructured in the function
 *   signature). Destructuring a SolidJS component's props captures the
 *   value at mount time and breaks reactivity for any prop that changes
 *   without triggering a parent re-render of the component. This bit us
 *   in an earlier version of this file: `pinned` was destructured, and
 *   pin toggles — which mutate `documentState` but not the document
 *   array — never reached the component, so the pin state appeared to
 *   reset on the next render cycle.
 */

import clsx from "clsx";
import { Show, createSignal, type JSX } from "solid-js";
import { createBlock } from "@/store/global";
import type { ToolNode } from "../types";
import { ToolBlockOverlay } from "./ToolBlockOverlay";

interface ToolBlockProps {
    node: ToolNode;
    /** User has clicked to pin this tool block open. */
    pinned: boolean;
    /** Toggle the pinned state (called on click of the collapsed row). */
    onTogglePin: () => void;
    /** Bookmark state + handler — surfaced in the overlay action bar
     *  (SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md §3.4). Optional for
     *  callers that don't surface bookmarking. */
    isBookmarked?: boolean;
    onBookmark?: () => void;
    /** Opens the tool's overlay content in a dedicated pane. */
    onOpenInPane?: () => void;
}

const STATUS_ICON: Record<ToolNode["status"], string> = {
    running: "⏳",
    pending_approval: "⚠",
    success: "✓",
    failed: "✗",
    denied: "⊘",
};

// Hover is instant under the inline-panel layout. The portal overlay
// needed 150ms enter / 300ms leave because it floated outside the
// row's hover boundary — the user had to "travel" through dead space
// to reach the overlay content. With the inline panel
// (SPEC_TOOL_AUTO_EXPAND_PANEL_2026_05_16.md), both the summary row
// and the panel sit inside the same .agent-tool-block element, so
// mouseleave only fires when the cursor truly exits the whole block.
// No dead space, no grace window. See feedback_no_timers_or_delays.md.

export const ToolBlock = (props: ToolBlockProps): JSX.Element => {
    // Hover-expand state. Instant on enter, instant on leave.
    const [hovering, setHovering] = createSignal(false);
    const handleMouseEnter = () => setHovering(true);
    const handleMouseLeave = () => setHovering(false);

    // Auto-expand while the tool is actively running (or awaiting approval,
    // or in a terminal-failure state where the user almost certainly wants
    // to see the output). Pin still wins as an explicit override (so the
    // user can keep a completed tool expanded). Hover keeps working as a
    // peek affordance for collapsed (completed-success) tools.
    //
    // Per SPEC_TOOL_AUTO_EXPAND_PANEL_2026_05_16.md §4.2 — Phase B. This
    // hybrid is the minimal change that delivers the visibility win
    // without introducing `userExpandState` (the three-state map). A
    // follow-up phase can add click-to-collapse-mid-run if needed; for
    // now, click on a running tool is a no-op visually because it's
    // already expanded.
    const autoExpanded = (): boolean => {
        const s = props.node.status;
        return s === "running" || s === "pending_approval" || s === "failed";
    };
    const expanded = () => props.pinned || autoExpanded() || hovering();

    const statusIcon = (): string => STATUS_ICON[props.node.status] || "•";

    return (
        <div
            onMouseEnter={handleMouseEnter}
            onMouseLeave={handleMouseLeave}
            class={clsx("agent-tool-block", {
                collapsed: !expanded(),
                expanded: expanded(),
                pinned: props.pinned,
                running: props.node.status === "running",
                success: props.node.status === "success",
                failed: props.node.status === "failed",
            })}
        >
            <div class="agent-tool-summary" onClick={props.onTogglePin}>
                <span class="agent-tool-status-icon">{statusIcon()}</span>
                <span class="agent-tool-name" title={props.node.summary}>{props.node.summary}</span>
                <Show when={props.node.duration}>
                    <span class="agent-tool-duration">({props.node.duration.toFixed(1)}s)</span>
                </Show>
                {/* Live-tail: while the tool is streaming, show the most
                    recent stdout/stderr line right in the collapsed row
                    so the user can watch progress without expanding
                    the overlay. With auto-expand-while-running, the
                    panel below already shows the full stream — this
                    tail still helps for the user-collapsed-mid-run
                    case (and for tools the user manually pinned-closed
                    while running). */}
                <Show when={
                    props.node.log?.open === true
                    && (props.node.log?.chunks?.length ?? 0) > 0
                }>
                    <span
                        class="agent-tool-live-tail"
                        title={`latest stream output (${props.node.log?.chunks?.length ?? 0} chunks)`}
                    >
                        ↳ {props.node.log!.chunks[props.node.log!.chunks.length - 1].content}
                    </span>
                </Show>
                <Show when={props.node.tool === "Agent"}>
                    <button
                        class="agent-tool-open-pane"
                        title="Open subagent in new pane"
                        onClick={(e) => {
                            e.stopPropagation();
                            const agentId = (props.node.params as any).subagent_id || props.node.id;
                            createBlock({
                                meta: {
                                    view: "subagent",
                                    "subagent:id": agentId,
                                } as any,
                            });
                        }}
                    >
                        ⧉
                    </button>
                </Show>
            </div>
            {/* Phase A: inline panel — replaces the Portal-to-document.body
                positioning hack. The panel renders in normal document
                flow underneath the summary row, so its lifecycle is
                tied to the tool's `expanded()` state (and, in Phase B,
                to the tool's running status). Mouse enter/leave on the
                panel keeps the hover state alive so the user can
                interact with the content. See
                docs/specs/SPEC_TOOL_AUTO_EXPAND_PANEL_2026_05_16.md §4. */}
            <Show when={expanded()}>
                <div
                    class="agent-tool-panel"
                    onClick={(e) => e.stopPropagation()}
                    onMouseEnter={handleMouseEnter}
                    onMouseLeave={handleMouseLeave}
                >
                    <ToolBlockOverlay
                        node={props.node}
                        isBookmarked={props.isBookmarked}
                        onBookmark={props.onBookmark}
                        onOpenInPane={props.onOpenInPane}
                    />
                </div>
            </Show>
        </div>
    );
};

ToolBlock.displayName = "ToolBlock";
