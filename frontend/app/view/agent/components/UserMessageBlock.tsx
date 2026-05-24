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
 *     contrast user-input color via `--user-input-color`.
 *
 *   - **Startup injection, not pinned, not hovering**: collapsed.
 *     Only the summary `<button>` renders.
 *
 *   - **Startup injection, hovering (transient)**: body renders as
 *     a `position: absolute` overlay anchored to the summary,
 *     EITHER above (`bottom: 100%`) OR below (`top: 100%`),
 *     depending on `pickExpandDirection()`. The summary's screen-Y
 *     never changes — the cursor stays exactly where it was when
 *     the hover timer fired. See
 *     `docs/specs/SPEC_STARTUP_HOVER_EXPANSION_ANCHOR_2026_05_24.md`.
 *
 *   - **Startup injection, pinned**: body renders in normal
 *     document flow below the summary. Persistent commitment to
 *     the expanded form; the virtualizer remeasures the row to
 *     its new height (estimated by `estimateUnwrappedTextHeight`
 *     in `renderers.ts`). Option B from the spec §4.2.
 *
 * Why mouseleave doesn't fire when the cursor crosses from
 * summary into the absolute body: per the MDN spec, `mouseleave`
 * fires only when the pointer has exited the element AND ALL OF
 * ITS DESCENDANTS — and "descendants" is the DOM tree, NOT visual
 * containment. The absolute body is still a DOM child of
 * `.agent-user-message`, so cursor moves between summary and
 * body keep `hovering` true with no jitter and no `safePolygon`.
 *
 * SolidJS reactivity note: props are accessed via `props.X`
 * (never destructured). Pin toggles mutate
 * `documentState.pinnedNodes` without re-triggering the parent's
 * render of the document array; destructuring would lose the
 * reactive read (PR #346 ToolBlock fix; same shape here).
 */

import clsx from "clsx";
import { Show, createSignal, onCleanup, type JSX } from "solid-js";
import type { UserMessageNode } from "../types";
import { pickExpandDirection, type ExpandDirection } from "./hover-anchor";

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

// Conservative estimate of the rendered body height for direction
// selection. The startup payload is typically 4-12kB of Markdown;
// at 24px line-height + average line density, ~400px is realistic.
// The pure-function `pickExpandDirection` handles the "fits-neither"
// case gracefully if this is wrong; this is just a hint for the
// flip decision. Over-estimating slightly biases toward "above" in
// near-bottom rows, which is the desirable conservative direction.
const STARTUP_BODY_ESTIMATE_PX = 400;

export const UserMessageBlock = (props: UserMessageBlockProps): JSX.Element => {
    const [hovering, setHovering] = createSignal(false);
    const [expandDirection, setExpandDirection] = createSignal<ExpandDirection>("below");
    let enterTimer: ReturnType<typeof setTimeout> | undefined;
    let rootEl: HTMLDivElement | undefined;

    const handleMouseEnter = () => {
        clearTimeout(enterTimer);
        enterTimer = setTimeout(() => {
            // Capture the summary's position and the viewport size
            // ONCE at expand-time. Direction stays fixed for the
            // duration of this hover (no resize listener, no
            // re-evaluation — per spec §5.2). Re-evaluated on the
            // next mouseenter.
            if (rootEl) {
                const summaryEl = rootEl.querySelector<HTMLElement>(
                    ".agent-user-message-summary",
                );
                if (summaryEl) {
                    const rect = summaryEl.getBoundingClientRect();
                    setExpandDirection(
                        pickExpandDirection(
                            { top: rect.top, bottom: rect.bottom },
                            window.innerHeight,
                            STARTUP_BODY_ESTIMATE_PX,
                        ),
                    );
                }
            }
            setHovering(true);
        }, HOVER_ENTER_DELAY_MS);
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

    /** Render mode for the body — drives DOM positioning + CSS classes:
     *
     *   - `flow`    — normal document flow (regular input + pinned startup).
     *   - `overlay` — `position: absolute`, anchored to summary, direction
     *                 from `expandDirection()`.
     *   - `hidden`  — body not rendered.
     */
    const bodyMode = (): "flow" | "overlay" | "hidden" => {
        if (!collapsible()) return "flow";
        if (props.pinned) return "flow";
        if (hovering()) return "overlay";
        return "hidden";
    };

    return (
        <div
            ref={(el) => (rootEl = el)}
            class={clsx("agent-user-message", {
                "agent-user-message--startup": collapsible(),
                "agent-user-message--collapsed": collapsible() && !expanded(),
                "agent-user-message--expanded": collapsible() && expanded(),
                "agent-user-message--pinned": collapsible() && props.pinned,
            })}
            onMouseEnter={collapsible() ? handleMouseEnter : undefined}
            onMouseLeave={collapsible() ? handleMouseLeave : undefined}
        >
            {/* Summary is always present (when collapsible) so the
             *  ARIA/keyboard surface is stable. We hide it via CSS
             *  when bodyMode is "flow" + pinned, since the body
             *  takes over the row's identity in that mode.
             *
             *  When in overlay mode, the summary stays visible at
             *  its normal 32px height — the body floats above or
             *  below it. */}
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
            <Show when={bodyMode() !== "hidden"}>
                <div
                    class={clsx("agent-user-message-content", {
                        "agent-user-message-content--flow": bodyMode() === "flow",
                        "agent-user-message-content--overlay-below":
                            bodyMode() === "overlay" && expandDirection() === "below",
                        "agent-user-message-content--overlay-above":
                            bodyMode() === "overlay" && expandDirection() === "above",
                    })}
                >
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
                    <pre>{props.node.message}</pre>
                </div>
            </Show>
        </div>
    );
};

UserMessageBlock.displayName = "UserMessageBlock";
