// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The last closes, for the next "closing a pane took a while". A ring of 50, read from the console
 * as `__closeTrace()`: each close's phases with the time since the click. A close slower than
 * 500 ms from click to pane gone is also one console line naming its slowest phase, so a slow close
 * says whether it was the probes, the prompt, the relayout or srv.
 * SPEC_PANE_CLOSING_INDICATOR_AND_FASTER_CLOSE_2026_10_28.md §3.4.
 */

export const CLOSE_TRACE_MAX = 50;
export const SLOW_CLOSE_MS = 500;

/** `probes`: the busy-agent and leave-site asks. `prompt`: a person's answer. `reducer`: the pane leaves
 *  the layout. `gone`: the next frame, the pane's element out of the DOM. `closePane`: srv's answer. */
export type ClosePhase = "probes" | "prompt" | "reducer" | "gone" | "closePane" | "kept" | "failed";

export interface CloseTrace {
    /** The phase that just ended. */
    mark(phase: ClosePhase): void;
    /** The close is over without the pane having left (a throw, an early return, a no): records `kept`
     *  or `failed` unless the close got as far as removing it. Safe to call on every path. */
    end(phase: "kept" | "failed"): void;
}

export interface CloseTraceRing {
    start(label: string): CloseTrace;
    lines(): string[];
}

export function createCloseTraceRing(
    now: () => number = () => performance.now(),
    log: (line: string) => void = console.warn
): CloseTraceRing {
    const ring: string[] = [];
    const push = (line: string) => {
        ring.push(line);
        if (ring.length > CLOSE_TRACE_MAX) ring.shift();
    };
    return {
        start(label) {
            const t0 = now();
            const marks: { phase: ClosePhase; at: number }[] = [];
            let slowReported = false;
            let finished = false;
            // The phase that took longest, up to and including `gone`: each phase is the time since the one before.
            const slowest = (): { phase: ClosePhase; ms: number } | null => {
                let prev = 0;
                let worst: { phase: ClosePhase; ms: number } | null = null;
                for (const m of marks) {
                    const ms = m.at - prev;
                    prev = m.at;
                    if (!worst || ms > worst.ms) worst = { phase: m.phase, ms };
                    if (m.phase === "gone") break;
                }
                return worst;
            };
            const has = (p: ClosePhase) => marks.some((m) => m.phase === p);
            return {
                end(phase) {
                    if (!finished && !has("reducer")) this.mark(phase);
                },
                mark(phase) {
                    if (finished) return;
                    marks.push({ phase, at: Math.round(now() - t0) });
                    const gone = marks.find((m) => m.phase === "gone");
                    if (gone && gone.at > SLOW_CLOSE_MS && !slowReported) {
                        slowReported = true;
                        const worst = slowest();
                        log(`[close] ${label} took ${gone.at}ms to leave; slowest phase: ${worst?.phase} (${worst?.ms}ms)`);
                    }
                    // Done once srv answered and the pane left, in either order, or the close was kept or failed.
                    if (has("kept") || has("failed") || (has("closePane") && has("gone"))) {
                        finished = true;
                        push(`[close] ${label} ${marks.map((m) => `${m.phase}+${m.at}ms`).join(" ")}`);
                    }
                },
            };
        },
        lines: () => [...ring],
    };
}

export const closeTraceRing = createCloseTraceRing();

(globalThis as { __closeTrace?: () => string[] }).__closeTrace = () => closeTraceRing.lines();

/** Runs `fn` when the frame that follows has been prepared (the pane's element is out of the DOM by then);
 *  a hidden window has no frames, so it runs soon after instead. */
export function afterFrame(fn: () => void): void {
    if (typeof requestAnimationFrame === "function" && !(typeof document !== "undefined" && document.hidden)) {
        requestAnimationFrame(() => fn());
    } else {
        setTimeout(fn, 0);
    }
}
