// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Global "any agent busy" aggregator.
 *
 * Each agent pane registers its `turnPhase` accessor on mount; the store
 * tracks which pane IDs are currently `Submitting | Streaming |
 * Interrupting` (i.e. `workingFromPhase(turnPhase) === true`) and exposes
 * a reactive count + boolean.
 *
 * Step 1 of `SPEC_OS_TASKBAR_AGENT_ACTIVITY_INDICATOR_2026_05_23.md` —
 * no IPC yet; the count is logged to the console so we can verify
 * aggregation in `task dev` before wiring the host-side taskbar/dock
 * indicators.
 */

import {
    createEffect,
    createRoot,
    createSignal,
    type Accessor,
} from "solid-js";
import {
    workingFromPhase,
    type TurnPhase,
} from "./agent-pane-state/types";

const [busyPanes, setBusyPanes] = createSignal<ReadonlySet<string>>(
    new Set(),
    // Always notify on `set` — Set identity changes but we mutate-then-replace
    // so equality-by-reference would still see the same object if we forgot
    // to clone. `equals: false` makes the contract explicit.
    { equals: false },
);

export const busyCount: Accessor<number> = () => busyPanes().size;
export const anyBusy: Accessor<boolean> = () => busyPanes().size > 0;

/**
 * Register a pane's `turnPhase` accessor with the global tracker.
 *
 * MUST be called from inside a SolidJS reactive owner (the agent
 * component body). The internal `createEffect` is owned by that scope
 * and auto-disposes on unmount, so subscriptions never outlive their
 * pane. Callers should still call {@link unregisterActivity} in
 * `onCleanup` as a belt-and-braces — the effect dispose only stops
 * future updates; it doesn't remove the pane from the busy set if it
 * was working at unmount time.
 */
export function registerActivity(
    blockId: string,
    turnPhase: Accessor<TurnPhase>,
): void {
    createEffect(() => {
        const working = workingFromPhase(turnPhase());
        const cur = new Set(busyPanes());
        if (working) {
            if (!cur.has(blockId)) {
                cur.add(blockId);
                setBusyPanes(cur);
            }
        } else if (cur.delete(blockId)) {
            setBusyPanes(cur);
        }
    });
}

export function unregisterActivity(blockId: string): void {
    const cur = new Set(busyPanes());
    if (cur.delete(blockId)) setBusyPanes(cur);
}

// Step 1 debug. The effect needs a long-lived reactive owner since this
// module is loaded once at app boot; `createRoot` anchors it outside any
// component's lifetime.
createRoot(() => {
    let prev = -1;
    createEffect(() => {
        const n = busyCount();
        if (n !== prev) {
            console.log(
                `[agentActivity] busyCount=${n} panes=[${Array.from(busyPanes()).join(",")}]`,
            );
            prev = n;
        }
    });
});
