// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ActivityDock — a strip pinned above the composer listing every long-running
 * activity (shells + subagents spawned by this pane) as a uniform row. Click
 * a row → its live view expands in the dock; click again → collapses.
 *
 * Expansion state is local to the dock (`expandedIds` signal). It is intentionally
 * NOT shared with `documentState.pinnedNodes` — that shared state caused the
 * inline PersistentShellBlock in the conversation to expand simultaneously,
 * showing a duplicate log at whatever scroll position the shell block occupied.
 * Clicking in the dock expands in-dock only; clicking the inline block expands
 * there only. No cross-talk.
 *
 * Spec: docs/specs/SPEC_LONG_RUNNING_SHELL_PINNED_DOCK_2026_06_15.md
 *   (D1 dock vs swarm · D3 ordering · D4 retention · D6 cap/overflow)
 */

import { Show, type JSX } from "solid-js";
import { DockRows, OverflowButton } from "./activity-dock-parts";
import { createActivityDock, type ActivityDockProps } from "./activity-dock-state";

export const ActivityDock = (props: ActivityDockProps): JSX.Element => {
    const s = createActivityDock(props);

    return (
        <Show when={s.ordered().length > 0}>
            <div class="agent-activity-dock">
                <DockRows s={s} />
                <Show when={s.overflow().length > 0}>
                    <OverflowButton s={s} />
                </Show>
            </div>
        </Show>
    );
};

ActivityDock.displayName = "ActivityDock";
