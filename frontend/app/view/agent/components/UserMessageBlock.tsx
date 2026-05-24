// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * UserMessageBlock — agent-pane row for user input.
 *
 * Two render shapes, gated by `node.isStartup` (set by the stream
 * parser when the message matches the startup-injection heading):
 *
 *   - **Regular user input** (`isStartup` false / undefined):
 *     always expanded. `<pre>` content rendered with
 *     `white-space: pre` (no soft wrap; long lines scroll
 *     horizontally inside the bubble). High-contrast cyan-blue
 *     tint via `--user-input-color`.
 *
 *   - **Startup injection** (`isStartup === true`): collapsed-by-
 *     default summary row (`⓵ Session context (hover to peek ·
 *     click to pin)`). Hover-expand (150ms enter delay) shows the
 *     full Markdown payload transiently. Clicking the summary
 *     pins the expanded state. Mirrors the ToolBlock pattern in
 *     `docs/specs/tool-collapse.md`.
 *
 * Spec: `docs/specs/SPEC_USER_INPUT_VISIBILITY_AND_STARTUP_COLLAPSE_2026_05_24.md`.
 *
 * SolidJS reactivity note: props are accessed via `props.X` (never
 * destructured). Pin toggles mutate `documentState.pinnedNodes`
 * without triggering a parent re-render of the document array —
 * destructuring `pinned` would lose reactivity (cost AgentMux a
 * bug fix in PR #346 on ToolBlock; same shape here).
 */

import clsx from "clsx";
import { Show, createSignal, onCleanup, type JSX } from "solid-js";
import type { UserMessageNode } from "../types";

interface UserMessageBlockProps {
    node: UserMessageNode;
    /** User has clicked to pin a startup row open. Has no effect
     * for regular user input (which is always expanded). */
    pinned: boolean;
    /** Toggle the pin. Wired by the parent through
     * `documentState.pinnedNodes`. */
    onTogglePin: () => void;
}

// Matches ToolBlock's 150ms — prevents accidental expansions while
// the user scrolls past the row.
const HOVER_ENTER_DELAY_MS = 150;

export const UserMessageBlock = (props: UserMessageBlockProps): JSX.Element => {
    const [hovering, setHovering] = createSignal(false);
    let enterTimer: ReturnType<typeof setTimeout> | undefined;

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

    return (
        <div
            class={clsx("agent-user-message", {
                "agent-user-message--startup": collapsible(),
                "agent-user-message--collapsed": collapsible() && !expanded(),
                "agent-user-message--expanded": collapsible() && expanded(),
                "agent-user-message--pinned": collapsible() && props.pinned,
            })}
            onMouseEnter={collapsible() ? handleMouseEnter : undefined}
            onMouseLeave={collapsible() ? handleMouseLeave : undefined}
        >
            <Show when={collapsible() && !expanded()}>
                {/* Click-to-pin is bound HERE, on the summary row
                 *  only. Binding it on the outer block would let
                 *  clicks inside the expanded <pre> (e.g. placing
                 *  the caret, selecting text to copy) toggle pin and
                 *  immediately collapse the message — codex P2
                 *  round 1 on PR #1020.
                 *
                 *  Rendered as a real <button> so keyboard users get
                 *  Tab focus + Space/Enter activation for free. The
                 *  default button-chrome is reset in SCSS via the
                 *  shared `.agent-user-message-summary` rule.
                 *  Codex P2 round 2 on PR #1020. */}
                <button
                    type="button"
                    class="agent-user-message-summary"
                    onClick={props.onTogglePin}
                    aria-expanded={props.pinned}
                    aria-label="Session context — click to expand and pin"
                >
                    <span class="agent-user-message-icon">⓵</span>
                    <span class="agent-user-message-label">Session context</span>
                    <span class="agent-user-message-hint">
                        (hover to peek · click to pin)
                    </span>
                </button>
            </Show>
            <Show when={!collapsible() || expanded()}>
                <div class="agent-user-message-content">
                    {/* Top-right action button — has two modes:
                     *
                     *   - Hover-expanded but not pinned: shows 📌
                     *     so the user can pin without racing the
                     *     150ms enter-delay. (Codex P2 round 3:
                     *     "Keep click-to-pin available while
                     *      startup preview is expanded.")
                     *   - Pinned: shows ✕ to collapse.
                     *
                     * Both call `onTogglePin` — the parent reducer
                     * flips the boolean and the conditional below
                     * picks the right glyph for the new state. */}
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
                                // Stop propagation so the click doesn't
                                // bubble to the outer block (where no
                                // handler is bound today, but future
                                // outer handlers won't fire either).
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
