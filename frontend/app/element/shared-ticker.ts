// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The one clock behind every ASCII animation on screen (AsciiSpinner's slash and ActivityAnimation's
// buckets): the Swarm can show dozens at once, and independent timers would give each its own phase,
// which reads as a strobe rather than one steady motion. A tick is TICKER_MS; each animation derives
// its frame from the tick, so identical animations always show the same frame. The timer runs only while
// at least one animation is mounted.
// SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md §6.10,
// SPEC_LONG_RUNNING_ACTIVITY_ANIMATIONS_2026_11_02.md §3.

import { createSignal } from "solid-js";

export const TICKER_MS = 120;

const [tick, setTick] = createSignal(0);
let mounted = 0;
let timer: ReturnType<typeof setInterval> | undefined;

/** The current tick: reading it in a Solid scope re-runs the scope every TICKER_MS (while a ticker is held). */
export const sharedTick = tick;

/** Hold the ticker; call the returned function to let go. The last holder stops the timer and resets the tick. */
export function acquireTicker(): () => void {
    if (mounted++ === 0) timer = setInterval(() => setTick((t) => t + 1), TICKER_MS);
    return () => {
        if (--mounted === 0) {
            clearInterval(timer);
            timer = undefined;
            setTick(0);
        }
    };
}
