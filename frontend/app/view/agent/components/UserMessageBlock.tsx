// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * UserMessageBlock — agent-pane row for user input.
 *
 * Render shapes:
 *
 *   - **Regular user input** (`isStartup` false / undefined): always
 *     expanded inline. `<pre>` content uses `white-space: pre` so
 *     long lines scroll horizontally inside the bubble. Highest-
 *     contrast user-input color via `--user-input-color`. Also gets a
 *     hover-to-peek time/estimate overlay (PeekOverlay), same as tool
 *     calls and thinking clumps — 2026-08-03 user request.
 *
 *   - **Startup injection, not pinned, not hovering**: collapsed.
 *     Only the summary `<button>` renders.
 *
 *   - **Startup injection, hovering (transient)**: body renders via
 *     PeekOverlay — Portal-rendered at document.body, anchored to the
 *     TOP edge of the summary row (2026-08-03 user feedback: "when it
 *     appears we need it to appear at the top of the entry"). This
 *     used to be a plain `position: absolute` child of `.agent-user-
 *     message` picking above/below via `pickExpandDirection()`; that
 *     had the same virtualized-row stacking-context bug PeekOverlay.tsx
 *     was built to fix for ToolBlock/MarkdownBlock (confirmed live via
 *     CDP — a later row's own content painted over this overlay too).
 *     See `docs/specs/SPEC_STARTUP_HOVER_EXPANSION_ANCHOR_2026_05_24.md`
 *     for the original anchor-to-summary design this superseded.
 *
 *   - **Startup injection, pinned**: body renders in normal
 *     document flow below the summary. Persistent commitment to
 *     the expanded form; the virtualizer remeasures the row to
 *     its new height (estimated by `estimateUnwrappedTextHeight`
 *     in `renderers.ts`). Option B from the spec §4.2.
 *
 * SolidJS reactivity note: props are accessed via `props.X`
 * (never destructured). Pin toggles mutate
 * `documentState.pinnedNodes` without re-triggering the parent's
 * render of the document array; destructuring would lose the
 * reactive read (PR #346 ToolBlock fix; same shape here).
 */

import clsx from "clsx";
import { Show, createMemo, createSignal, onCleanup, type JSX } from "solid-js";
import type { UserMessageNode } from "../types";
import { LinkifiedText } from "@/app/element/linkified-text";
import { estimateTokenCount, formatCompactNumber } from "@/util/format-count";
import { formatExactTime, formatTimeAgo } from "@/util/format-time";
import { useTick } from "@/app/hook/useTick";
import { PeekOverlay } from "./PeekOverlay";

interface UserMessageBlockProps {
    node: UserMessageNode;
    /** User has clicked to pin a startup row open. Has no effect
     * for regular user input (always expanded). */
    pinned: boolean;
    /** Toggle the pin. Wired by the parent through
     * `documentState.pinnedNodes`. */
    onTogglePin: () => void;
}

// 150ms enter-delay matches ToolBlock — prevents accidental
// expansions during fast scroll-throughs.
const HOVER_ENTER_DELAY_MS = 150;

export const UserMessageBlock = (props: UserMessageBlockProps): JSX.Element => {
    const [hovering, setHovering] = createSignal(false);
    let enterTimer: ReturnType<typeof setTimeout> | undefined;
    let rootEl: HTMLDivElement | undefined;

    const handleMouseEnter = () => {
        clearTimeout(enterTimer);
        enterTimer = setTimeout(() => setHovering(true), HOVER_ENTER_DELAY_MS);
    };
    const handleMouseLeave = () => {
        clearTimeout(enterTimer);
        setHovering(false);
    };
    onCleanup(() => clearTimeout(enterTimer));

    // Only the startup variant is collapsible. Regular input is
    // always fully visible — hover/pin are no-ops there.
    const collapsible = (): boolean => props.node.isStartup === true;
    const expanded = (): boolean =>
        !collapsible() || props.pinned || hovering();

    /** Render mode for the body:
     *
     *   - `flow`    — normal document flow (regular input + pinned startup).
     *   - `overlay` — PeekOverlay, Portal-rendered, top-anchored to the row.
     *   - `hidden`  — body not rendered.
     */
    const bodyMode = (): "flow" | "overlay" | "hidden" => {
        if (!collapsible()) return "flow";
        if (props.pinned) return "flow";
        if (hovering()) return "overlay";
        return "hidden";
    };

    // Peek metadata (time + token estimate) for regular (non-startup) user
    // input — same treatment ToolBlock.tsx / MarkdownBlock.tsx already have.
    // Startup-injection rows show their full body on hover instead (via
    // bodyMode() === "overlay" above) rather than a metadata summary, so
    // this is gated to the non-collapsible case only.
    const peekTick = useTick(1000);
    const [isPeeking, setIsPeeking] = createSignal(false);
    const peekTimeText = createMemo(() => {
        if (collapsible() || !isPeeking()) return null;
        peekTick(); // re-run every second so "ago" stays live while hovered
        const ts = props.node.timestamp;
        if (ts == null) return null;
        return `${formatExactTime(ts)} · ${formatTimeAgo(ts)}`;
    });
    const peekEstimateText = createMemo(() => {
        if (collapsible()) return null;
        const count = estimateTokenCount(props.node.message);
        return count > 0 ? `~${formatCompactNumber(count)} tok (est.)` : null;
    });
    let peekEnterTimer: ReturnType<typeof setTimeout> | undefined;
    const handlePeekEnter = () => {
        clearTimeout(peekEnterTimer);
        peekEnterTimer = setTimeout(() => setIsPeeking(true), HOVER_ENTER_DELAY_MS);
    };
    const handlePeekLeave = () => {
        clearTimeout(peekEnterTimer);
        setIsPeeking(false);
    };
    onCleanup(() => clearTimeout(peekEnterTimer));

    // Shared between the flow-mode in-DOM render and the Portal-rendered
    // PeekOverlay — same pin/unpin button + message body in both.
    const bodyContent = () => (
        <>
            {/* Top-right action button. Two glyphs:
             *    📌 — pin (hovered, not yet pinned).
             *    ✕  — unpin (currently pinned).
             *
             * stopPropagation so the click doesn't bubble
             * to ancestors (no outer handlers today, but
             * defensive for future additions). */}
            <Show when={collapsible()}>
                <button
                    type="button"
                    class={clsx({
                        "agent-user-message-unpin": props.pinned,
                        "agent-user-message-pin": !props.pinned,
                    })}
                    title={
                        props.pinned
                            ? "Collapse session context"
                            : "Pin session context open"
                    }
                    aria-label={
                        props.pinned
                            ? "Collapse session context"
                            : "Pin session context open"
                    }
                    onClick={(e) => {
                        e.stopPropagation();
                        props.onTogglePin();
                    }}
                >
                    {props.pinned ? "✕" : "📌"}
                </button>
            </Show>
            <pre><LinkifiedText text={props.node.message} /></pre>
        </>
    );

    return (
        <div
            ref={(el) => (rootEl = el)}
            class={clsx("agent-user-message", {
                "agent-user-message--startup": collapsible(),
                "agent-user-message--collapsed": collapsible() && !expanded(),
                "agent-user-message--expanded": collapsible() && expanded(),
                "agent-user-message--pinned": collapsible() && props.pinned,
            })}
            onMouseEnter={collapsible() ? handleMouseEnter : handlePeekEnter}
            onMouseLeave={collapsible() ? handleMouseLeave : handlePeekLeave}
        >
            {/* Summary is always present (when collapsible) so the
             *  ARIA/keyboard surface is stable. We hide it via CSS
             *  when bodyMode is "flow" + pinned, since the body
             *  takes over the row's identity in that mode.
             *
             *  When in overlay mode, the summary stays visible at
             *  its normal 32px height — the body floats above it. */}
            <Show when={collapsible()}>
                <button
                    type="button"
                    class="agent-user-message-summary"
                    onClick={props.onTogglePin}
                    aria-expanded={expanded()}
                    aria-label="Session context — click to expand and pin"
                >
                    <span class="agent-user-message-icon">⓵</span>
                    <span class="agent-user-message-label">Session context</span>
                    <span class="agent-user-message-hint">
                        {props.pinned
                            ? "(pinned · click ✕ to collapse)"
                            : "(hover to peek · click to pin)"}
                    </span>
                </button>
            </Show>
            <Show when={bodyMode() === "flow"}>
                <div class="agent-user-message-content agent-user-message-content--flow">
                    {bodyContent()}
                </div>
            </Show>
            <PeekOverlay
                show={bodyMode() === "overlay"}
                rowEl={() => rootEl}
                class="agent-user-message-peek-overlay"
            >
                <div class="agent-user-message-content">
                    {bodyContent()}
                </div>
            </PeekOverlay>
            {/* Peek metadata overlay for regular (non-startup) input —
                time + token estimate, no body (the message is already
                always visible in flow above). */}
            <Show when={!collapsible()}>
                <PeekOverlay
                    show={isPeeking() && (peekTimeText() != null || peekEstimateText() != null)}
                    rowEl={() => rootEl}
                >
                    <Show when={peekTimeText()}>
                        <div class="agent-node-peek-tooltip-meta">{peekTimeText()}</div>
                    </Show>
                    <Show when={peekEstimateText()}>
                        <div class="agent-node-peek-tooltip-meta">{peekEstimateText()}</div>
                    </Show>
                </PeekOverlay>
            </Show>
        </div>
    );
};

UserMessageBlock.displayName = "UserMessageBlock";
