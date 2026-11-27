// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * How far what a drag moves lags the pointer, as one `[perf] <name>` line per drag, so an installed build answers
 * "did it wait for the page?" without a lab: the pane border (`border-drag`) and a text selection (`select-drag`).
 * A pointer move that no frame has drawn yet starts a clock; the first frame drawn after it stops it (the frame's own
 * work is in it: a frame that waited on the page shows up here as a long lag). Moves that arrive while that frame is
 * still coming share it.
 */

export const PERF_LOG_PREFIX = "[perf]";
/** Fewer pointer moves than this is a click on the border, not a drag: no line. */
export const MIN_MOVES = 5;
/** A border drawn this long after its pointer move, in ms, is counted as late. */
export const LATE_LAG_MS = 50;

interface DragLag {
    moves: number;
    lags: number[];
    from: number | null;
}

/** Calls `cb` once the frame requested now has been drawn. */
function afterNextPaint(cb: () => void): void {
    requestAnimationFrame(() => setTimeout(cb, 0));
}

export interface DragLagTimer {
    /** The first move of a drag: starts timing against the pointer. Nothing when one is on already. */
    begin(): void;
    /** The release: stops timing and logs the line. */
    end(log?: (line: string) => void): void;
}

/** A timer for one kind of drag, logged as `[perf] <name> ...`. One drag of the kind at a time. */
export function createDragLag(name: string): DragLagTimer {
    let drag: DragLag | null = null;
    const onPointerMove = (e: Event): void => {
        if (!drag) return;
        drag.moves++;
        if (drag.from !== null) return;
        const d = drag;
        d.from = e.timeStamp;
        afterNextPaint(() => {
            if (d.from !== null) d.lags.push(performance.now() - d.from);
            d.from = null;
        });
    };
    return {
        begin() {
            if (drag || typeof window === "undefined") return;
            drag = { moves: 0, lags: [], from: null };
            window.addEventListener("pointermove", onPointerMove, { capture: true, passive: true });
        },
        end(log = (line) => console.warn(line)) {
            const d = drag;
            if (!d) return;
            drag = null;
            window.removeEventListener("pointermove", onPointerMove, { capture: true });
            if (d.moves < MIN_MOVES) return;
            const s = summarizeLags(d.moves, d.lags);
            log(
                `${PERF_LOG_PREFIX} ${name} moves=${s.moves} frames-drawn=${s.drawn} lag p50=${s.p50}ms p95=${s.p95}ms worst=${s.worst}ms late(>${LATE_LAG_MS}ms)=${s.late}`
            );
        },
    };
}

const border = createDragLag("border-drag");
/** The first move of a border drag. */
export const beginBorderDrag = (): void => border.begin();
/** The release of a border drag: logs its line. */
export const endBorderDrag = (log?: (line: string) => void): void => border.end(log);

const select = createDragLag("select-drag");
/** The first move of a text selection drag (a field or page text). */
export const beginSelectDrag = (): void => select.begin();
/** The release of a text selection drag: logs its line. */
export const endSelectDrag = (log?: (line: string) => void): void => select.end(log);

export interface LagSummary {
    moves: number;
    drawn: number;
    p50: number;
    p95: number;
    worst: number;
    late: number;
}

/** The lags (ms), as the drag's summary; zeros when there are none. */
export function summarizeLags(moves: number, lags: readonly number[]): LagSummary {
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
