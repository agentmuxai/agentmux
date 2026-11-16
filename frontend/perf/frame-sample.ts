// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Frame times while the person scrolls, types or clicks, in the page log.
 *
 * `[perf] long-frame` is the Long Animation Frames API, which reports a frame only when it ran 50 ms or more. A scroll
 * that runs at 30 ms a frame looks rough and leaves no line, and a line that does exist does not say which pane was
 * being used. This samples the gap between animation frames, only while there is input, and writes one
 * `[perf] frame-sample` line per pane every `LOG_EVERY_MS`: how many frames, how many ran over 17, 33 and 50 ms, and the
 * worst.
 *
 * Cost: nothing while the page is idle (no animation frame is requested, so the page can idle as it did). While there
 * is input, one callback a frame does a subtraction and three comparisons, and the page was drawing those frames
 * anyway. The listeners are passive and capture-phase: they only note the event's target and the time.
 *
 * What is logged is the pane's short block id and view type (the same description as `[perf] slow-interaction`), counts
 * and milliseconds. Never text, a title, a path or an href.
 */
import { describePane } from "./slow-interactions";

const PERF_LOG_PREFIX = "[perf]";
/** The thresholds a frame gap is counted over: one missed frame at 60 Hz, two, three. */
export const FRAME_STEPS_MS = [17, 33, 50] as const;
/** Animation frames are stamped at the display's refresh; a 60 Hz gap of 16.67 ms must not count as over 17. */
const TOLERANCE_MS = 0.7;
/** Frames keep being sampled this long after the last input event. */
export const ACTIVE_MS = 400;
/** One line per pane is written this often. */
export const LOG_EVERY_MS = 300_000;
/** At most this many panes are counted separately in a period; the rest are counted together as `block=- view=-`. */
export const MAX_PANES = 4;
const OTHER = "block=- view=-";

interface Counts {
    frames: number;
    over: [number, number, number];
    worst: number;
}

export interface FrameSampler {
    /** Input happened on `target`: sample frames for `ACTIVE_MS` more, counted against the pane around the target. */
    note(target: EventTarget | null): void;
    /** Write a line for every pane with frames in this period, and start a new period. */
    flush(): void;
    /** Stop sampling and drop the period's counts without writing them. */
    stop(): void;
}

export function createFrameSampler(opts: {
    log: (line: string) => void;
    windowLabel: string;
    viewOf: (blockId: string) => string | undefined;
    now?: () => number;
    raf?: (cb: FrameRequestCallback) => number;
    cancelRaf?: (id: number) => void;
}): FrameSampler {
    const now = opts.now ?? (() => performance.now());
    const raf = opts.raf ?? ((cb) => requestAnimationFrame(cb));
    const cancelRaf = opts.cancelRaf ?? ((id) => cancelAnimationFrame(id));
    let periodStart = now();
    const byPane = new Map<string, Counts>();
    let activeUntil = Number.NEGATIVE_INFINITY;
    let rafId: number | null = null;
    let previousStamp: number | null = null;
    let lastTarget: EventTarget | null = null;
    let paneKey = OTHER;

    const countsFor = (key: string): Counts => {
        let c = byPane.get(key);
        if (c) return c;
        if (byPane.size >= MAX_PANES && key !== OTHER) return countsFor(OTHER);
        c = { frames: 0, over: [0, 0, 0], worst: 0 };
        byPane.set(key, c);
        return c;
    };

    const onFrame = (stamp: number): void => {
        rafId = null;
        if (previousStamp != null) {
            const gap = stamp - previousStamp;
            const c = countsFor(paneKey);
            c.frames++;
            for (let i = 0; i < FRAME_STEPS_MS.length; i++) if (gap > FRAME_STEPS_MS[i] + TOLERANCE_MS) c.over[i]++;
            if (gap > c.worst) c.worst = gap;
        }
        previousStamp = stamp;
        if (now() < activeUntil) rafId = raf(onFrame);
        else previousStamp = null;
    };

    return {
        note(target) {
            if (target !== lastTarget) {
                lastTarget = target;
                const node = target as Node | null;
                paneKey = describePane(node, opts.viewOf);
            }
            activeUntil = now() + ACTIVE_MS;
            if (rafId == null) {
                previousStamp = null;
                rafId = raf(onFrame);
            }
        },
        flush() {
            const t = now();
            const seconds = Math.round((t - periodStart) / 1000);
            periodStart = t;
            const lines: string[] = [];
            for (const [key, c] of byPane) {
                if (c.frames === 0) continue;
                lines.push(
                    `${PERF_LOG_PREFIX} frame-sample window=${opts.windowLabel} ${key} ` +
                        `period=${seconds}s frames=${c.frames} over17=${c.over[0]} over33=${c.over[1]} over50=${c.over[2]} ` +
                        `worst=${Math.round(c.worst)}ms`
                );
            }
            byPane.clear();
            for (const line of lines) opts.log(line);
        },
        stop() {
            if (rafId != null) cancelRaf(rafId);
            rafId = null;
            previousStamp = null;
            activeUntil = Number.NEGATIVE_INFINITY;
            byPane.clear();
        },
    };
}

/** The input that starts sampling: a scroll or wheel turn, a key, a click or drag, a touch move. */
export const SAMPLE_EVENTS = ["wheel", "scroll", "keydown", "pointerdown", "touchmove"] as const;

/** Attach the sampler to the page's input and write its lines every `LOG_EVERY_MS`. Returns a function that detaches it. */
export function startFrameSampler(opts: {
    windowLabel: string;
    viewOf: (blockId: string) => string | undefined;
}): () => void {
    const sampler = createFrameSampler({ log: (line) => console.info(line), ...opts });
    const note = (e: Event): void => sampler.note(e.target);
    const listen: AddEventListenerOptions = { capture: true, passive: true };
    for (const type of SAMPLE_EVENTS) document.addEventListener(type, note, listen);
    const timer = setInterval(() => sampler.flush(), LOG_EVERY_MS);
    return () => {
        for (const type of SAMPLE_EVENTS) document.removeEventListener(type, note, listen);
        clearInterval(timer);
        sampler.stop();
    };
}
