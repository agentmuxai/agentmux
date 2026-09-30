// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Split out of agent-view.tsx (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §3.5 step 3).

import { Show, type Accessor, type JSX } from "solid-js";
import { Portal } from "solid-js/web";

/**
 * Gradient progress bar — marching-ants shimmer traced around the full pane
 * perimeter while working, hidden at rest. Color matches the pane's own
 * selection-ring color (not a fixed --accent-color) via --progress-bar-color,
 * set on .agent-pane-stack (agent-view.scss). Portaled into a slot
 * AgentPaneChrome owns, between the tab strip and the content (its own row,
 * never overlapping either), bridged through the AgentViewModel instance's
 * progressBarMount signal — see that field's own doc comment (agent-model.ts)
 * for why chrome and content, now separate component trees, need that
 * indirection. The agent view's state (turnPhase, launch activity) is what
 * drives the bar, but .agent-view (nested inside .agent-pane-stack-content,
 * itself BELOW the tab strip in DOM order) can't reach a position above the
 * tab strip through CSS alone; every ancestor between here and there clips
 * overflow before an absolutely-positioned escape could ever become visible.
 * See SPEC_AGENT_PANE_STATUS_GRADIENT_2026_06_14.md §4 and
 * SPEC_AGENT_PANE_PROGRESS_BAR_ABOVE_TAB_STRIP_2026_08_10.md. Renders nothing
 * until the slot ref is assigned (one frame, first mount only).
 */
export const AgentProgressBar = (props: {
    mount: Accessor<HTMLDivElement | null | undefined>;
    active: boolean;
    stopping: boolean;
}): JSX.Element => (
    <Show when={props.mount()}>
        <Portal mount={props.mount()!}>
            <div
                class="agent-pane-progress-bar"
                classList={{
                    "agent-pane-progress-bar--active": props.active,
                    "agent-pane-progress-bar--stopping": props.stopping,
                }}
                role="progressbar"
                aria-label="Agent working"
                aria-valuemin={0}
                aria-valuemax={100}
            />
        </Portal>
    </Show>
);
