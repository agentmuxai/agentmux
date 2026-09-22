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
import { PEEK_ENTER_DELAY_MS } from "./hover-anchor";
import { useNodePeek } from "../hooks/useNodePeek";
import { isPrimaryButtonDown, onPrimaryButtonRelease } from "@/app/util/pointer-drag-state";
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

export const UserMessageBlock = (props: UserMessageBlockProps): JSX.Element => {
    const [hovering, setHovering] = createSignal(false);
    let enterTimer: ReturnType<typeof setTimeout> | undefined;

    // Gated exactly like `useNodePeek`'s sibling path below, and for the same
    // reason. This component has TWO hover paths — collapsible (startup) rows
    // use this private one, everything else uses the hook — and only the hook
    // was gated. `onMouseEnter={collapsible() ? handleMouseEnter : ...}` meant
    // a text-selection drag sweeping across a COLLAPSED STARTUP ROW still
    // scheduled `setHovering(true)`, flipped `bodyMode()` to "overlay", and
    // mounted the Portal-rendered body preview under the cursor mid-drag.
    //
    // That is the exact flicker this PR exists to remove, surviving in the
    // precise scenario its own title names ("dragging left across collapsible
    // header rows") — reagentx P1 on #3470.
    const handleMouseEnter = () => {
        if (isPrimaryButtonDown()) return;
        clearTimeout(enterTimer);
        // Re-checked when the delay elapses, not just at call time: the button
        // can go down during the delay window (enter fired just before
        // mousedown), and without this the timeout would still mount the
        // overlay mid-drag.
        enterTimer = setTimeout(() => {
            if (isPrimaryButtonDown()) return;
            setHovering(true);
        }, PEEK_ENTER_DELAY_MS);
    };
    // Leave is deliberately NOT gated — see tooltip.tsx's note: gating unmount
    // too strands an overlay open forever when the leave fires mid-drag and
    // the button is released elsewhere.
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
    // `setRowEl` is wired to the row below and `rowEl` is read by both
    // PeekOverlays. This component used to keep its own private `rootEl` and
    // never register it, so the hook's own `rowEl` signal stayed permanently
    // undefined — and the drag-release hover resync in useNodePeek could
    // never fire for this one call site, silently reproducing the very
    // stuck-closed defect that resync exists to fix (reagentx P1 on #3470).
    // One element reference, owned by the hook.
    const { isPeeking, rowEl: peekRowEl, setRowEl: setPeekRowEl, handlePeekEnter, handlePeekLeave } =
        useNodePeek();

    // The collapsible path's own drag-release resync. `useNodePeek` already
    // does this for the non-collapsible path, but a collapsed startup row
    // never routes through `handlePeekEnter`, so it needs its own — otherwise
    // a drag that ENDS on a collapsed row leaves it stuck shut until the
    // cursor leaves and returns, which is the mirror-image defect of the one
    // above.
    onCleanup(
        onPrimaryButtonRelease(() => {
            if (collapsible() && peekRowEl()?.matches(":hover")) handleMouseEnter();
        }),
    );
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
            ref={setPeekRowEl}
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
                rowEl={peekRowEl}
                class="agent-user-message-peek-overlay"
                // Full width, left-aligned: this variant renders a real
                // message body (sometimes kilobytes of startup payload),
                // unlike the shrink-wrapped metadata peek every other
                // caller uses.
                align="stretch"
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
                    rowEl={peekRowEl}
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
