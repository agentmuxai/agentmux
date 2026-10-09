// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Show, type JSX } from "solid-js";
import { awaySummary, unseenTurnsFor } from "@/app/store/turn-awareness";

/**
 * A dot in a pane's header while turns you didn't start have finished there
 * unseen (store/turn-awareness.ts). Hover says what; looking at the pane
 * clears it. SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md §5.3.
 */
export function UnseenTurnsDot(props: { blockId: string }): JSX.Element {
    const unseen = () => unseenTurnsFor(props.blockId);
    return (
        <Show when={unseen()}>
            {(u) => <span class="block-frame-unseen-turns" role="status" title={awaySummary(u())} aria-label={awaySummary(u())} />}
        </Show>
    );
}
