// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * How far the pane border lags the pointer during a drag, as one `[perf] border-drag` line per drag, so an installed
 * build answers "did the border wait for the panes to repaint?" without a lab. A pointer move that no frame has drawn
 * yet starts a clock; the first frame drawn after it stops it (the frame's own work is in it: a frame that waited on
 * the panes shows up here as a long lag). Moves that arrive while that frame is still coming share it.
 */

const PERF_LOG_PREFIX = "[perf]";
/** Fewer pointer moves than this is a click on the border, not a drag: no line. */
const MIN_MOVES = 5;
/** A border drawn this long after its pointer move, in ms, is counted as late. */
export const LATE_LAG_MS = 50;

interface BorderDrag {
    moves: number;
    lags: number[];
    from: number | null;
}

let drag: BorderDrag | null = null;

/** Calls `cb` once the frame requested now has been drawn. */
function afterNextPaint(cb: () => void): void {
    requestAnimationFrame(() => setTimeout(cb, 0));
}

function onPointerMove(e: Event): void {
    if (!drag) return;
    drag.moves++;
    if (drag.from !== null) return;
    const d = drag;
    d.from = e.timeStamp;
    afterNextPaint(() => {
        if (d.from !== null) d.lags.push(performance.now() - d.from);
        d.from = null;
    });
}

/** The first move of a border drag: starts timing the border against the pointer. */
export function beginBorderDrag(): void {
    if (drag || typeof window === "undefined") return;
    drag = { moves: 0, lags: [], from: null };
    window.addEventListener("pointermove", onPointerMove, { capture: true, passive: true });
}

export interface BorderLagSummary {
    moves: number;
    drawn: number;
    p50: number;
    p95: number;
    worst: number;
    late: number;
}

/** The lags (ms), as the drag's summary; zeros when there are none. */
export function summarizeLags(moves: number, lags: readonly number[]): BorderLagSummary {
    const sorted = [...lags].sort((a, b) => a - b);
    const at = (q: number): number => (sorted.length ? sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * q))] : 0);
    return {
        moves,
        drawn: sorted.length,
        p50: Math.round(at(0.5)),
        p95: Math.round(at(0.95)),
        worst: Math.round(sorted.length ? sorted[sorted.length - 1] : 0),
        late: sorted.filter((l) => l > LATE_LAG_MS).length,
    };
}

/** The release: stops timing and logs the drag's line. Never content, only counts and times. */
export function endBorderDrag(log: (line: string) => void = (line) => console.warn(line)): void {
    const d = drag;
    if (!d) return;
    drag = null;
    window.removeEventListener("pointermove", onPointerMove, { capture: true });
    if (d.moves < MIN_MOVES) return;
    const s = summarizeLags(d.moves, d.lags);
    log(
        `${PERF_LOG_PREFIX} border-drag moves=${s.moves} frames-drawn=${s.drawn} lag p50=${s.p50}ms p95=${s.p95}ms worst=${s.worst}ms late(>${LATE_LAG_MS}ms)=${s.late}`
    );
}
