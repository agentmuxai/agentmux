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
import { Show, createEffect, createSignal, onCleanup, type JSX } from "solid-js";
import { createBlock } from "@/store/global";
import type { ToolNode } from "../types";
import { ToolBlockOverlay } from "./ToolBlockOverlay";
import {
    findScrollContainerRect,
    maxOverlayHeight,
    pickExpandDirection,
    type ExpandDirection,
} from "./hover-anchor";

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
    canceled: "⏹",
};

// 150ms enter delay prevents accidental expansions while scrolling past
// tool rows. Leave is instant — the inline panel is inside the same
// .agent-tool-block bounding box, so mouseleave only fires when the cursor
// truly exits the whole block. No dead space, no grace window needed on leave.
// (SPEC_TOOL_BLOCK_UX_POLISH_2026_05_23.md §Change 1)

const HOVER_ENTER_DELAY_MS = 150;

// Tool overlay body estimate — log views can be much bigger than user
// messages, but the cap is just a hint to `pickExpandDirection`; the
// "fits-neither" tie-break handles oversized cases gracefully.
const TOOL_BODY_ESTIMATE_PX = 320;

export const ToolBlock = (props: ToolBlockProps): JSX.Element => {
    const [hovering, setHovering] = createSignal(false);
    // Hover-anchor — direction and per-hover max-height for the
    // overlay panel when in hover-only (not pinned, not auto-
    // expanded) mode. Mirrors UserMessageBlock's mechanic
    // (SPEC_STARTUP_HOVER_EXPANSION_ANCHOR_2026_05_24.md).
    const [expandDirection, setExpandDirection] = createSignal<ExpandDirection>("below");
    const [overlayMaxHeight, setOverlayMaxHeight] = createSignal<number | null>(null);
    let enterTimer: ReturnType<typeof setTimeout> | undefined;
    let rootEl: HTMLDivElement | undefined;
    // Codex P1 on #988: both the block container AND the inline panel
    // bind these handlers. A fast cursor traversal (container → panel)
    // fires enter twice without an intervening leave; without clearing
    // the previous timer, multiple stale timeouts accumulate and
    // `handleMouseLeave` only clears the most recent. Clearing at the
    // top of enter makes the timer a single-active-armed value.
    const handleMouseEnter = () => {
        clearTimeout(enterTimer);
        enterTimer = setTimeout(() => {
            // Capture summary geometry + scroll-container bounds
            // ONCE at expand-time. Direction stays fixed for the
            // hover; next mouseenter re-evaluates. Same pattern as
            // UserMessageBlock — see hover-anchor.ts.
            if (rootEl) {
                const summaryEl = rootEl.querySelector<HTMLElement>(".agent-tool-summary");
                if (summaryEl) {
                    const rect = summaryEl.getBoundingClientRect();
                    const container = findScrollContainerRect(summaryEl);
                    const summaryV = { top: rect.top, bottom: rect.bottom };
                    const dir = pickExpandDirection(
                        summaryV,
                        container,
                        TOOL_BODY_ESTIMATE_PX,
                    );
                    setExpandDirection(dir);
                    setOverlayMaxHeight(maxOverlayHeight(summaryV, container, dir));
                }
            }
            setHovering(true);
        }, HOVER_ENTER_DELAY_MS);
    };
    const handleMouseLeave = () => {
        clearTimeout(enterTimer);
        setHovering(false);
        setOverlayMaxHeight(null);
    };
    onCleanup(() => clearTimeout(enterTimer));

    // Stays true for POST_COMPLETION_HOLD_MS after a running tool
    // completes so the user can read the final output line before the
    // panel collapses.
    // - Originally 1s (#988).
    // - Bumped to 5s in #1006 — too tight to finish reading.
    // - Dropped to 3s 2026-05-26 — 5s felt too long during live
    //   conversation; user wants it punchier.
    const POST_COMPLETION_HOLD_MS = 3000;
    const [postCompletionHold, setPostCompletionHold] = createSignal(false);
    // Gate the post-completion hold on a real active → inactive
    // TRANSITION (not on a status-value snapshot). The earlier draft
    // simply checked `s !== "running" && ...` which fired on mount
    // for already-completed tools — loaded transcripts would briefly
    // auto-expand every completed tool row on initial render
    // (codex P1 round 2 on #988).
    //
    // Background on the older self-loop bug (round 1): reading
    // `postCompletionHold()` inside the same effect that wrote to it
    // made the effect a subscriber of its own write; the synchronous
    // re-run disposed the previous owner and ran the just-registered
    // `onCleanup(() => clearTimeout(t))` BEFORE the timer could fire,
    // leaving the panel auto-expanded forever after the first
    // completion. Both bugs are fixed here: track only
    // `props.node.status`, and gate on a transition by comparing
    // against `prevStatus` captured outside the reactive scope.
    let prevStatus: string = props.node.status;
    // ACTIVE states are the ones that keep the panel auto-expanded
    // continuously. Terminal states (success, failed) trigger the
    // 5s post-completion hold once, then collapse. Failed used to
    // be in the active set (kept open forever) — removed per user
    // feedback that failed panels cluttered the pane; the ✗ icon
    // and red border-left already signal failure at a glance, and
    // hover-to-peek covers the rare case where the user wants to
    // re-read the output later.
    const isActive = (s: string): boolean =>
        s === "running" || s === "pending_approval";
    createEffect(() => {
        const s = props.node.status;
        if (isActive(prevStatus) && !isActive(s)) {
            setPostCompletionHold(true);
            const t = setTimeout(() => setPostCompletionHold(false), POST_COMPLETION_HOLD_MS);
            onCleanup(() => clearTimeout(t));
        }
        prevStatus = s;
    });

    // Auto-expand while the tool is actively running (or awaiting
    // approval). Terminal states (success OR failure) get the 5s
    // post-completion hold then collapse. Pin still wins as an
    // explicit override (so the user can keep a completed tool
    // expanded). Hover keeps working as a peek affordance for
    // collapsed (completed) tools — successes and failures alike.
    //
    // Per SPEC_TOOL_AUTO_EXPAND_PANEL_2026_05_16.md §4.2 — Phase B.
    // The 2026-05-24 user feedback removed `failed` from the
    // always-expanded set; failed-collapses-after-5s mirrors the
    // success path, and the ✗ icon + red border-left at the
    // collapsed row continue to flag the failure.
    const autoExpanded = (): boolean => {
        const s = props.node.status;
        return s === "running" || s === "pending_approval"
            || postCompletionHold();
    };
    const expanded = () => props.pinned || autoExpanded() || hovering();

    // Render mode for the panel — mirrors UserMessageBlock:
    //
    //   - `hidden`  : not expanded.
    //   - `overlay` : expanded by hover ONLY (no pin, no
    //                 auto-expand by status). Uses absolute
    //                 positioning so the row doesn't change height
    //                 and the panel can flip above/below based on
    //                 available container space. The natural fit
    //                 for "the user is just peeking."
    //   - `flow`    : expanded by pin OR auto-status (running,
    //                 pending_approval, post-completion hold).
    //                 Renders in normal flow under the summary —
    //                 the long-form persistent commitment.
    //
    // Per the user request for tool calls near the bottom of the
    // pane to expand UPWARD instead of being clipped: hover state
    // now uses the same hover-anchor design as the startup row
    // (SPEC_STARTUP_HOVER_EXPANSION_ANCHOR_2026_05_24.md §5.10
    // contemplated exactly this generalization).
    const panelMode = (): "hidden" | "overlay" | "flow" => {
        if (!expanded()) return "hidden";
        if (props.pinned || autoExpanded()) return "flow";
        return "overlay";
    };

    const statusIcon = (): string => STATUS_ICON[props.node.status] || "•";

    return (
        <div
            ref={(el) => (rootEl = el)}
            onMouseEnter={handleMouseEnter}
            onMouseLeave={handleMouseLeave}
            class={clsx("agent-tool-block", {
                collapsed: !expanded(),
                expanded: expanded(),
                pinned: props.pinned,
                running: props.node.status === "running",
                success: props.node.status === "success",
                failed: props.node.status === "failed",
                canceled: props.node.status === "canceled",
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
            {/* Panel — three render modes per `panelMode()`:
             *
             *   hidden  → `.agent-tool-panel--hidden` (off).
             *   flow    → in-flow under the summary (default DOM
             *             layout — pinned / running / post-hold).
             *   overlay → absolute positioning above OR below the
             *             summary so a hover near the pane's bottom
             *             expands upward instead of being clipped.
             *
             * Always rendered in the DOM so CSS transitions can
             * animate the off→on shift; `inert` + `aria-hidden`
             * remove it from the focus/a11y tree when hidden.
             */}
            <div
                class={clsx("agent-tool-panel", {
                    "agent-tool-panel--hidden": panelMode() === "hidden",
                    "agent-tool-panel--flow": panelMode() === "flow",
                    "agent-tool-panel--overlay-below":
                        panelMode() === "overlay" && expandDirection() === "below",
                    "agent-tool-panel--overlay-above":
                        panelMode() === "overlay" && expandDirection() === "above",
                })}
                style={
                    panelMode() === "overlay" && overlayMaxHeight() !== null
                        ? { "max-height": `${overlayMaxHeight()}px` }
                        : undefined
                }
                // Codex P2 on #988: with the always-rendered markup, the
                // collapsed panel was visually hidden via max-height /
                // opacity but still in the focusable + a11y tree, so
                // keyboard users could tab into action buttons that
                // aren't visible. `inert` removes the entire subtree
                // from focus + accessibility while collapsed (Chrome 102+,
                // supported in the bundled CEF runtime).
                inert={!expanded()}
                aria-hidden={!expanded()}
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
        </div>
    );
};

ToolBlock.displayName = "ToolBlock";
