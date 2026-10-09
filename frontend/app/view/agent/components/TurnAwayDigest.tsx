// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Show, createEffect, createSignal, type Accessor, type JSX } from "solid-js";
import { IconButton } from "@/app/element/ui";
import { awaySummary, markTurnsSeen, unseenTurnsFor } from "@/app/store/turn-awareness";

/**
 * When you come back to a pane where turns you didn't start finished unseen,
 * one line above the composer says what happened: "While you were away: 3
 * turns · 2 jekts (AgentX, Korp) · 1 finished task". Looking clears the
 * pane's dot; the line stays until dismissed or the next turn starts.
 * SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md §5.1 item 4, §5.3.
 */
export function TurnAwayDigest(props: {
    blockId: string;
    /** The user is looking at this pane now (window focused, pane focused). */
    looking: Accessor<boolean>;
    /** A turn is running: the summary is history once the next one starts. */
    busy: Accessor<boolean>;
}): JSX.Element {
    const [line, setLine] = createSignal<string | null>(null);
    createEffect(() => {
        if (!props.looking()) return;
        const seen = unseenTurnsFor(props.blockId) ? markTurnsSeen(props.blockId) : null;
        if (seen) setLine(awaySummary(seen));
    });
    let wasBusy = props.busy();
    createEffect(() => {
        const busy = props.busy();
        if (busy && !wasBusy) setLine(null);
        wasBusy = busy;
    });
    return (
        <Show when={line()}>
            {(text) => (
                <div class="agent-away-digest" role="status">
                    <span class="agent-away-digest-text">{text()}</span>
                    <IconButton icon="xmark" label="Dismiss" class="agent-away-digest-dismiss" onClick={() => setLine(null)} />
                </div>
            )}
        </Show>
    );
}
