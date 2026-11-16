// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The one clock behind every ASCII animation on screen (AsciiSpinner's slash and ActivityAnimation's
// buckets): the Swarm can show dozens at once, and independent timers would give each its own phase,
// which reads as a strobe rather than one steady motion. A tick is TICKER_MS; each animation derives
// its frame from the tick, so identical animations always show the same frame. The timer runs only while
// at least one animation is mounted.
//
// An animation is a plain callback the clock calls, not a Solid binding on a tick signal. Solid abandons every
// binding queued in a flush that throws, and gives them up for good (the window-display watchdog's doc says the
// same of tab content); a mark bound to a signal stopped on whatever frame it had when some other computation
// threw, each at its own moment (#47754: Swarm rows "working" with a different frozen frame each, until the
// window was restarted). A callback is called by the clock itself, one at a time, each in its own try/catch.
// SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md §6.10,
// SPEC_LONG_RUNNING_ACTIVITY_ANIMATIONS_2026_11_02.md §3.

export const TICKER_MS = 120;

interface Subscriber {
    draw: (tick: number) => void;
    /** The last tick this subscriber drew without throwing. */
    drawn: number;
}

const subscribers = new Set<Subscriber>();
let ticks = 0;
let timer: ReturnType<typeof setInterval> | undefined;
let failures = 0;

/** A mark more than this many ticks behind the clock has stopped following it (about a second). */
const FROZEN_AFTER_TICKS = 8;

/** The current tick, for a mark's first frame. It is not reactive: marks are called, they do not read it. */
export const currentTick = (): number => ticks;

function advance(): void {
    ticks++;
    for (const sub of [...subscribers]) {
        try {
            sub.draw(ticks);
            sub.drawn = ticks;
        } catch (e) {
            // One mark failing (a prop read after its owner went) must not stop the others. The first failure and
            // then one in five hundred are logged: a mark that fails every tick would flood the log.
            if (failures++ % 500 === 0) console.error("[ticker] a mark failed to draw", e);
        }
    }
}

/**
 * Call `draw(tick)` on every tick until the returned function is called. The first subscriber starts the timer;
 * the last one stops it and resets the tick, so the next animation starts from its first frame.
 */
export function subscribeFrames(draw: (tick: number) => void): () => void {
    const sub: Subscriber = { draw, drawn: ticks };
    subscribers.add(sub);
    if (subscribers.size === 1) timer = setInterval(advance, TICKER_MS);
    return () => {
        if (!subscribers.delete(sub)) return;
        if (subscribers.size === 0) {
            clearInterval(timer);
            timer = undefined;
            ticks = 0;
        }
    };
}

/** The marks animating now, how many of them are behind the clock, and the clock's tick: two readings a minute
 *  apart with the same tick and marks above zero mean the timer itself stopped. */
export function markHealth(): { marks: number; frozen: number; tick: number } {
    let frozen = 0;
    for (const sub of subscribers) if (sub.drawn < ticks - FROZEN_AFTER_TICKS) frozen++;
    return { marks: subscribers.size, frozen, tick: ticks };
}
