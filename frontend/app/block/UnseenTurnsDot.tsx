// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Show, type JSX } from "solid-js";
import { unseenTurnsFor } from "@/app/store/turn-awareness";
import { awaySummary } from "@/app/view/agent/turn-trigger-text";
import { tip } from "@/app/element/tip";

/**
 * A dot in a pane's header while turns you didn't start have finished there
 * unseen (store/turn-awareness.ts). Hover says what; looking at the pane
 * clears it. SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md §5.3.
 *
 * Keyed: looking at the pane clears the turns from an effect that runs in the same flush as the window-focus
 * change, and a keyed Show hands the child the value, not an accessor that throws "Stale read from <Show>" once
 * they are gone (the away digest beside it is keyed for the same reason).
 */
export function UnseenTurnsDot(props: { blockId: string }): JSX.Element {
    const unseen = () => unseenTurnsFor(props.blockId);
    return (
        <Show when={unseen()} keyed>
            {(u) => <span class="block-frame-unseen-turns" role="status" {...tip(awaySummary(u))} aria-label={awaySummary(u)} />}
        </Show>
    );
}
