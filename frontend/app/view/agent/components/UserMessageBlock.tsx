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
            onClick={collapsible() ? props.onTogglePin : undefined}
        >
            <Show when={collapsible() && !expanded()}>
                <div class="agent-user-message-summary">
                    <span class="agent-user-message-icon">⓵</span>
                    <span class="agent-user-message-label">Session context</span>
                    <span class="agent-user-message-hint">
                        (hover to peek · click to pin)
                    </span>
                </div>
            </Show>
            <Show when={!collapsible() || expanded()}>
                <div class="agent-user-message-content">
                    <pre>{props.node.message}</pre>
                </div>
            </Show>
        </div>
    );
};

UserMessageBlock.displayName = "UserMessageBlock";
