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
/** A pointer button held this long is taken as released without our hearing of it (the page lost the pointer). */
export const POINTER_HOLD_MAX_MS = 30_000;
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
    /** A gesture (wheel, key, pointer press, touch move) happened on `target`: sample frames for `ACTIVE_MS` more,
     *  counted against the pane around the target. */
    note(target: EventTarget | null): void;
    /** A scroll event. It keeps sampling going only while the person is doing something (a gesture in the last
     *  `ACTIVE_MS`, or a pointer button held, as when dragging a scrollbar); a pane that scrolls itself (a streaming
     *  transcript following its tail) neither starts sampling nor moves the pane the frames are counted against. */
    noteScroll(): void;
    /** A pointer button went down or up. While one is held, scroll events keep sampling going. */
    pointer(down: boolean): void;
    /** The page was hidden: drop the pending frame, so the time it stays hidden is not counted as one frame. */
    suspend(): void;
    /** Write a line for every pane with frames in this period, and start a new period. */
    flush(): void;
    /** Stop sampling and drop the period's counts without writing them. */
    stop(): void;
}

/** The line for one pane's counts over a period. */
function periodLine(windowLabel: string, key: string, seconds: number, c: Counts): string {
    return (
        `${PERF_LOG_PREFIX} frame-sample window=${windowLabel} ${key} ` +
        `period=${seconds}s frames=${c.frames} over17=${c.over[0]} over33=${c.over[1]} over50=${c.over[2]} ` +
        `worst=${Math.round(c.worst)}ms`
    );
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
    let lastGestureAt = Number.NEGATIVE_INFINITY;
    let pointerDownAt: number | null = null;

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

    const startSampling = (): void => {
        activeUntil = now() + ACTIVE_MS;
        if (rafId == null) {
            previousStamp = null;
            rafId = raf(onFrame);
        }
    };

    return {
        note(target) {
            if (target !== lastTarget) {
                lastTarget = target;
                const node = target as Node | null;
                paneKey = describePane(node, opts.viewOf);
            }
            lastGestureAt = now();
            startSampling();
        },
        noteScroll() {
            const t = now();
            const pointerHeld = pointerDownAt != null && t - pointerDownAt < POINTER_HOLD_MAX_MS;
            if (t - lastGestureAt < ACTIVE_MS || pointerHeld) startSampling();
        },
        pointer(down) {
            pointerDownAt = down ? now() : null;
        },
        suspend() {
            if (rafId != null) cancelRaf(rafId);
            rafId = null;
            previousStamp = null;
            activeUntil = Number.NEGATIVE_INFINITY;
        },
        flush() {
            const t = now();
            const seconds = Math.round((t - periodStart) / 1000);
            periodStart = t;
            const lines: string[] = [];
            for (const [key, c] of byPane) {
                if (c.frames === 0) continue;
                lines.push(periodLine(opts.windowLabel, key, seconds, c));
            }
            byPane.clear();
            for (const line of lines) opts.log(line);
        },
        stop() {
            if (rafId != null) cancelRaf(rafId);
            rafId = null;
            previousStamp = null;
            activeUntil = Number.NEGATIVE_INFINITY;
            lastGestureAt = Number.NEGATIVE_INFINITY;
            pointerDownAt = null;
            byPane.clear();
        },
    };
}

/** The gestures that start sampling: a wheel turn, a key, a click or drag, a touch move. A `scroll` event is not one of
 *  them: a pane scrolls itself too, so it only keeps sampling going while a gesture is under way. */
export const SAMPLE_EVENTS = ["wheel", "keydown", "pointerdown", "touchmove"] as const;
const POINTER_UP_EVENTS = ["pointerup", "pointercancel"] as const;

/** Attach the sampler to the page's input and write its lines every `LOG_EVERY_MS`. Returns a function that detaches it. */
export function startFrameSampler(opts: {
    windowLabel: string;
    viewOf: (blockId: string) => string | undefined;
}): () => void {
    const sampler = createFrameSampler({ log: (line) => console.info(line), ...opts });
    const note = (e: Event): void => {
        if (e.type === "pointerdown") sampler.pointer(true);
        sampler.note(e.target);
    };
    const pointerUp = (): void => sampler.pointer(false);
    const scrolled = (): void => sampler.noteScroll();
    const visibility = (): void => {
        if (document.hidden) sampler.suspend();
    };
    const listen: AddEventListenerOptions = { capture: true, passive: true };
    for (const type of SAMPLE_EVENTS) document.addEventListener(type, note, listen);
    for (const type of POINTER_UP_EVENTS) document.addEventListener(type, pointerUp, listen);
    document.addEventListener("scroll", scrolled, listen);
    document.addEventListener("visibilitychange", visibility);
    const timer = setInterval(() => sampler.flush(), LOG_EVERY_MS);
    return () => {
        for (const type of SAMPLE_EVENTS) document.removeEventListener(type, note, listen);
        for (const type of POINTER_UP_EVENTS) document.removeEventListener(type, pointerUp, listen);
        document.removeEventListener("scroll", scrolled, listen);
        document.removeEventListener("visibilitychange", visibility);
        clearInterval(timer);
        sampler.stop();
    };
}
