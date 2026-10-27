// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Long Animation Frames (Chromium 123+) in the page log.
 *
 * The long-task line (`observers.ts`) says that a task took N ms, and nothing about what ran. A frame
 * of 50 ms or more, from the Long Animation Frames API, also says which scripts ran in it, who invoked
 * them (a websocket message handler, a timer, a rAF callback) from which file and function, and how
 * much of the frame was rendering and style/layout. One `[perf] long-frame` line per frame answers
 * "what filled it" from the log alone, where before it took a lab run with a probe.
 *
 * Lines are rate limited: a window that is janky for a minute would otherwise write a line per frame
 * and make the log pipe part of the problem; the frames not described are counted in the next line.
 */

const PERF_LOG_PREFIX = "[perf]";
/** At most this many frames are described in each `WINDOW_MS`. */
const MAX_LINES = 5;
const WINDOW_MS = 1000;
/** The scripts named in a line, the slowest first. */
const TOP_SCRIPTS = 3;

/** The parts of a `PerformanceLongAnimationFrameTiming` entry the line uses. */
export interface LongFrame {
    startTime: number;
    duration: number;
    blockingDuration?: number;
    renderStart?: number;
    styleAndLayoutStart?: number;
    scripts?: LongFrameScript[];
}

/** The parts of a `PerformanceScriptTiming` entry the line uses. */
export interface LongFrameScript {
    duration: number;
    invoker?: string;
    invokerType?: string;
    sourceURL?: string;
    sourceFunctionName?: string;
    sourceCharPosition?: number;
    forcedStyleAndLayoutDuration?: number;
}

const ms = (n: number): string => `${Math.round(n)}ms`;
const fileOf = (url: string): string => url.split(/[?#]/)[0].split("/").pop() ?? "";

function describeScript(s: LongFrameScript): string {
    const who = [s.invokerType, s.invoker].filter(Boolean).join(" ");
    const where = s.sourceURL
        ? ` @ ${fileOf(s.sourceURL)}${s.sourceCharPosition != null && s.sourceCharPosition >= 0 ? `:${s.sourceCharPosition}` : ""}`
        : "";
    const fn = s.sourceFunctionName ? ` ${s.sourceFunctionName}` : "";
    const forced = s.forcedStyleAndLayoutDuration ? ` (forced layout ${ms(s.forcedStyleAndLayoutDuration)})` : "";
    return `${ms(s.duration)} ${who}${fn}${where}${forced}`.replace(/\s+/g, " ").trim();
}

/** One line for a long frame: its length, how it divides, and the slowest scripts in it. */
export function describeLongFrame(frame: LongFrame): string {
    const end = frame.startTime + frame.duration;
    const scripts = [...(frame.scripts ?? [])].sort((a, b) => b.duration - a.duration);
    const parts = [`scripts ${ms(scripts.reduce((sum, s) => sum + s.duration, 0))}`];
    if (frame.renderStart) parts.push(`render ${ms(end - frame.renderStart)}`);
    if (frame.styleAndLayoutStart) parts.push(`style+layout ${ms(end - frame.styleAndLayoutStart)}`);
    const blocking = frame.blockingDuration ? ` (blocking ${ms(frame.blockingDuration)})` : "";
    const top = scripts.slice(0, TOP_SCRIPTS).map(describeScript);
    const more = scripts.length > TOP_SCRIPTS ? `; +${scripts.length - TOP_SCRIPTS} more` : "";
    return `long-frame ${ms(frame.duration)}${blocking}: ${parts.join(", ")}${top.length ? `; ${top.join("; ")}${more}` : "; no script"}`;
}

/** Logs a frame's line, up to `MAX_LINES` per `WINDOW_MS`; the frames over that are counted into the next line. */
export function createLongFrameLogger(
    log: (line: string) => void,
    now: () => number = () => performance.now(),
): (frame: LongFrame) => void {
    let windowStart = -Infinity;
    let inWindow = 0;
    let skipped = 0;
    return (frame) => {
        const t = now();
        if (t - windowStart >= WINDOW_MS) {
            windowStart = t;
            inWindow = 0;
        }
        if (inWindow >= MAX_LINES) {
            skipped++;
            return;
        }
        inWindow++;
        const dropped = skipped ? ` (+${skipped} long frames not logged)` : "";
        skipped = 0;
        log(`${PERF_LOG_PREFIX} ${describeLongFrame(frame)}${dropped}`);
    };
}

/** Starts the observer when the runtime has Long Animation Frames; null when it doesn't. */
export function startLongFrameObserver(): (() => void) | null {
    if (typeof PerformanceObserver === "undefined") return null;
    const supported = (PerformanceObserver as unknown as { supportedEntryTypes?: string[] }).supportedEntryTypes;
    if (!supported?.includes("long-animation-frame")) return null;
    const logFrame = createLongFrameLogger((line) => console.warn(line));
    const observer = new PerformanceObserver((list) => {
        for (const entry of list.getEntries()) logFrame(entry as unknown as LongFrame);
    });
    try {
        observer.observe({ type: "long-animation-frame", buffered: true });
    } catch (e) {
        console.warn(`${PERF_LOG_PREFIX} long-animation-frame observer failed to attach:`, e);
        return null;
    }
    return () => observer.disconnect();
}
