// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PeekMetaRow — the hover peek panel's time + token line, shared by every
 * transcript node kind so they all read the same: one line, pinned to the
 * panel's right edge, time first. Renders nothing when neither is known.
 *
 * Spec: docs/specs/SPEC_PEEK_PANEL_META_ROW_AND_MONO_COMMAND_2026_09_27.md §4.1.
 */

import { Show, type JSX } from "solid-js";

interface PeekMetaRowProps {
    /** `"<exact time> · <n>m ago"`, or null/undefined when the node has no timestamp. */
    time?: string | null;
    /** `"~<n> tok (est.)"`, or null/undefined when there is nothing to count. */
    tokens?: string | null;
}

export const PeekMetaRow = (props: PeekMetaRowProps): JSX.Element => (
    <Show when={props.time || props.tokens}>
        <div class="agent-node-peek-tooltip-meta">
            <Show when={props.time}>
                <span class="agent-node-peek-tooltip-time">{props.time}</span>
            </Show>
            <Show when={props.tokens}>
                <span class="agent-node-peek-tooltip-tokens">{props.tokens}</span>
            </Show>
        </div>
    </Show>
);
