// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Show, type Accessor, type JSX } from "solid-js";

/**
 * "↓ New activity" over the bottom of the transcript while the reader is scrolled up and the content has
 * grown below them (SPEC_AGENT_PANE_SCROLL_FOLLOW_STATE_MACHINE_2026_09_24.md §5.7). Clicking it follows the
 * bottom again. Out of flow, so nothing in the transcript moves when it comes and goes.
 */
export const NewActivityPill = (props: { visible: Accessor<boolean>; onJump: () => void }): JSX.Element => (
    <Show when={props.visible()}>
        <button type="button" class="agent-new-activity-pill" onClick={() => props.onJump()}>
            <span aria-hidden="true">↓</span> New activity
        </button>
    </Show>
);
