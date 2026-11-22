// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Show, createEffect, createSignal, onCleanup, type Accessor, type JSX } from "solid-js";
import { t } from "@/app/i18n/i18n";
import { CloseButton } from "@/app/element/ui";
import { markTurnsSeen, unseenTurnsFor } from "@/app/store/turn-awareness";
import { awaySummary } from "../turn-trigger-text";

/** How long the "While you were away" line stays before it hides itself. */
export const AWAY_DIGEST_SHOW_SECONDS = 10;

/**
 * When you come back to a pane where turns you didn't start finished unseen,
 * one line above the composer says what happened: "While you were away: 3
 * turns · 2 jekts (AgentX, Korp) · 1 finished task". Looking clears the
 * pane's dot; the line hides itself after 10 seconds (the seconds left show
 * beside it), or sooner when dismissed or the next turn starts.
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
    const [secondsLeft, setSecondsLeft] = createSignal(AWAY_DIGEST_SHOW_SECONDS);
    // The line hides itself after AWAY_DIGEST_SHOW_SECONDS and shows the seconds left. A new summary
    // restarts the count. The end is a clock time, so a throttled timer in a background tab does not
    // stretch it.
    createEffect(() => {
        if (line() === null) return;
        const endsAt = Date.now() + AWAY_DIGEST_SHOW_SECONDS * 1000;
        setSecondsLeft(AWAY_DIGEST_SHOW_SECONDS);
        const timer = setInterval(() => {
            const left = Math.ceil((endsAt - Date.now()) / 1000);
            if (left <= 0) setLine(null);
            else setSecondsLeft(left);
        }, 250);
        onCleanup(() => clearInterval(timer));
    });
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
    // Keyed: the line is cleared from effects that a window-focus change runs
    // in the same flush, and a keyed Show hands the child the value, not an
    // accessor that throws "Stale read from <Show>" once the line is gone.
    return (
        <Show when={line()} keyed>
            {(text) => (
                <div class="agent-away-digest" role="status">
                    <span class="agent-away-digest-text">{text}</span>
                    <span class="agent-away-digest-timer" aria-hidden="true">
                        {t("agentAway.secondsLeft", { count: secondsLeft() })}
                    </span>
                    <CloseButton label="Dismiss" class="agent-away-digest-dismiss" onClick={() => setLine(null)} />
                </div>
            )}
        </Show>
    );
}
