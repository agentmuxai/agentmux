// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * One-way-flow recorder — Phase 0 of
 * docs/specs/SPEC_AGENT_PANE_ONE_WAY_FLOW_2026_10_07.md.
 *
 * While a pane follows the conversation, text must only ever move up (V1) and
 * no blank space may be shown and then taken back (V2). This module samples
 * every painted frame of the transcript scroller and reports each frame that
 * breaks either rule, with the most likely cause. It is the acceptance test
 * for every later phase, and the baseline for the current code.
 *
 * Opt-in, in every build (like `shrink-trace.ts`, not DEV-gated: packaged
 * local builds are where the live data comes from). Off, it costs one boolean
 * check per noted event. On, it reads the rects of the mounted rows once per
 * frame. Enable from devtools or CDP with `__agentmuxOneWay.enable()`, which
 * persists in localStorage until `disable()`.
 *
 * Two layers, like the follow reducer the spec builds on:
 * - `checkFrame` is pure: two consecutive samples in, violations out. Table
 *   tested.
 * - `OneWayRecorder` binds it to one scroller: samples after paint, tracks
 *   user input (which exempts a frame), keeps counters and a ring of recent
 *   violations, and logs `[one-way]` lines, rate-limited.
 */

/** One mounted row, in unzoomed CSS px relative to the scroller viewport's top edge. */
export interface RowBox {
    id: string;
    type: string;
    top: number;
    bottom: number;
}

export interface FrameSample {
    t: number;
    /** The pane is following (data-follow-state="following"). */
    following: boolean;
    /** The user touched the pane recently: this frame is exempt (spec §1). */
    userInput: boolean;
    scrollTop: number;
    scrollHeight: number;
    clientHeight: number;
    clientWidth: number;
    zoom: number;
    /** The scroller's padding-bottom: room below the last row that is not a gap. */
    bottomPad: number;
    rows: RowBox[];
}

export type ViolationKind = "down" | "overshoot";

export interface Violation {
    kind: ViolationKind;
    t: number;
    /** The visible row that moved down the most. */
    rowId: string;
    rowType: string;
    /** How far it moved down, px. */
    dy: number;
    /** How many visible rows moved down. */
    rows: number;
    /** Blank space below the last row in the previous frame, px (overshoot only). */
    gapBefore: number;
    causes: string[];
}

export interface FrameResult {
    violation: Violation | null;
    /** Blank space below the last row in `cur`, px; 0 when none. */
    gap: number;
}

/** Below this, a move is sub-pixel settling, not a visible jump. */
export const MOVE_TOLERANCE_PX = 0.5;
/** Below this, space under the last row is rounding, not a gap. */
export const GAP_TOLERANCE_PX = 1;

const visible = (r: RowBox, clientHeight: number): boolean => r.bottom > 0 && r.top < clientHeight;

/** Blank space between the last row's bottom and the viewport's bottom, while overflowing. */
export function blankGap(s: FrameSample): number {
    if (s.rows.length === 0 || s.scrollHeight <= s.clientHeight + GAP_TOLERANCE_PX) return 0;
    let last = Number.NEGATIVE_INFINITY;
    for (const r of s.rows) if (r.bottom > last) last = r.bottom;
    const gap = s.clientHeight - s.bottomPad - last;
    return gap > GAP_TOLERANCE_PX ? gap : 0;
}

/** Why the frame changed, from the two samples alone (no knowledge of the code). */
export function inferCauses(prev: FrameSample, cur: FrameSample): string[] {
    const causes: string[] = [];
    if (cur.clientHeight > prev.clientHeight + MOVE_TOLERANCE_PX) {
        causes.push(`viewport-grow:${Math.round(cur.clientHeight - prev.clientHeight)}`);
    }
    if (cur.scrollHeight < prev.scrollHeight - MOVE_TOLERANCE_PX) {
        causes.push(`content-shrink:${Math.round(prev.scrollHeight - cur.scrollHeight)}`);
    }
    if (cur.scrollTop < prev.scrollTop - MOVE_TOLERANCE_PX) {
        causes.push(`scroll-up:${Math.round(prev.scrollTop - cur.scrollTop)}`);
    }
    const before = new Map(prev.rows.map((r) => [r.id, r]));
    const shrinks: { type: string; px: number }[] = [];
    for (const r of cur.rows) {
        const p = before.get(r.id);
        if (!p) continue;
        const d = p.bottom - p.top - (r.bottom - r.top);
        if (d > MOVE_TOLERANCE_PX) shrinks.push({ type: r.type, px: d });
    }
    shrinks.sort((a, b) => b.px - a.px);
    for (const s of shrinks.slice(0, 2)) causes.push(`row-shrink:${s.type}:${Math.round(s.px)}`);
    const now = new Set(cur.rows.map((r) => r.id));
    const removed = prev.rows.filter((r) => !now.has(r.id) && visible(r, prev.clientHeight)).length;
    if (removed > 0) causes.push(`row-removed:${removed}`);
    // Rows moved with no change in scroll position, viewport or heights:
    // something drew them somewhere else (a transform or animation).
    if (causes.length === 0 && Math.abs(cur.scrollTop - prev.scrollTop) <= MOVE_TOLERANCE_PX) causes.push("transform");
    return causes;
}

/**
 * Compare two consecutive painted frames. A frame is checked only when both
 * samples follow, neither is inside a user-input window, and the pane's width
 * and zoom are unchanged (user-initiated changes are exempt, spec §1).
 * `events` are the code's own notes since the previous frame (pin paths,
 * hold, glide), appended to the inferred causes.
 */
export function checkFrame(prev: FrameSample, cur: FrameSample, events: readonly string[] = []): FrameResult {
    const gap = blankGap(cur);
    const exempt =
        !prev.following ||
        !cur.following ||
        prev.userInput ||
        cur.userInput ||
        cur.clientHeight <= 0 ||
        prev.clientHeight <= 0 ||
        Math.abs(cur.clientWidth - prev.clientWidth) > MOVE_TOLERANCE_PX ||
        Math.abs(cur.zoom - prev.zoom) > 0.001;
    if (exempt) return { violation: null, gap };
    const before = new Map(prev.rows.map((r) => [r.id, r]));
    let worst: { row: RowBox; dy: number } | null = null;
    let moved = 0;
    for (const r of cur.rows) {
        const p = before.get(r.id);
        if (!p || !visible(p, prev.clientHeight) || !visible(r, cur.clientHeight)) continue;
        const dy = r.top - p.top;
        if (dy <= MOVE_TOLERANCE_PX) continue;
        moved++;
        if (!worst || dy > worst.dy) worst = { row: r, dy };
    }
    if (!worst) return { violation: null, gap };
    const gapBefore = blankGap(prev);
    return {
        gap,
        violation: {
            kind: gapBefore > 0 ? "overshoot" : "down",
            t: cur.t,
            rowId: worst.row.id,
            rowType: worst.row.type,
            dy: worst.dy,
            rows: moved,
            gapBefore,
            causes: [...inferCauses(prev, cur), ...events],
        },
    };
}

// ── DOM binding ─────────────────────────────────────────────────────────────

/** How long after the user's last input a frame is still theirs. */
export const USER_INPUT_EXEMPT_MS = 300;
const STORAGE_KEY = "agentmux:one-way-recorder";
const RING_SIZE = 50;
const LOG_BURST = 20;
const LOG_WINDOW_MS = 10_000;

export interface RecorderStats {
    pane: string;
    framesChecked: number;
    framesFollowing: number;
    down: number;
    overshoot: number;
    maxDy: number;
    /** Frames that showed blank space below the last row while following. */
    blankFrames: number;
    maxGap: number;
    /** Violations per inferred cause (first cause of each). */
    byCause: Record<string, number>;
    recent: Violation[];
}

const emptyStats = (pane: string): RecorderStats => ({
    pane,
    framesChecked: 0,
    framesFollowing: 0,
    down: 0,
    overshoot: 0,
    maxDy: 0,
    blankFrames: 0,
    maxGap: 0,
    byCause: {},
    recent: [],
});

/** Run `fn` right after the current frame is painted (approximately). */
function afterPaint(fn: () => void): void {
    const sched = (globalThis as { scheduler?: { postTask?: (f: () => void, o: object) => unknown } }).scheduler;
    if (sched?.postTask) {
        void sched.postTask(fn, { priority: "user-blocking" });
        return;
    }
    const ch = new MessageChannel();
    ch.port1.onmessage = () => fn();
    ch.port2.postMessage(0);
}

class OneWayRecorder {
    private prev: FrameSample | null = null;
    private events: string[] = [];
    private lastInputAt = Number.NEGATIVE_INFINITY;
    private raf: number | null = null;
    private running = false;
    private logTimes: number[] = [];
    private bottomPad = 0;
    stats: RecorderStats;
    private readonly detachInput: () => void;

    constructor(
        readonly pane: string,
        private readonly el: HTMLElement,
    ) {
        this.stats = emptyStats(pane);
        const mark = (): void => {
            this.lastInputAt = performance.now();
        };
        const opts: AddEventListenerOptions = { passive: true, capture: true };
        const onScroller = ["wheel", "touchstart", "touchmove", "pointerdown"] as const;
        for (const t of onScroller) el.addEventListener(t, mark, opts);
        // Any key: typing grows the composer (a user-initiated viewport change)
        // and jumps to the bottom; scroll keys move the transcript.
        document.addEventListener("keydown", mark, opts);
        window.addEventListener("pointerup", mark, opts);
        this.detachInput = () => {
            for (const t of onScroller) el.removeEventListener(t, mark, opts);
            document.removeEventListener("keydown", mark, opts);
            window.removeEventListener("pointerup", mark, opts);
        };
    }

    note(label: string): void {
        if (this.running && this.events.length < 16) this.events.push(label);
    }

    start(): void {
        if (this.running) return;
        this.running = true;
        this.prev = null;
        this.bottomPad = parseFloat(getComputedStyle(this.el).paddingBottom) || 0;
        const loop = (): void => {
            if (!this.running) return;
            afterPaint(() => this.sample());
            this.raf = requestAnimationFrame(loop);
        };
        this.raf = requestAnimationFrame(loop);
    }

    stop(): void {
        this.running = false;
        if (this.raf != null) cancelAnimationFrame(this.raf);
        this.raf = null;
        this.prev = null;
    }

    dispose(): void {
        this.stop();
        this.detachInput();
    }

    reset(): void {
        this.stats = emptyStats(this.pane);
        this.prev = null;
    }

    private read(): FrameSample | null {
        const el = this.el;
        if (!el.isConnected) return null;
        const box = el.getBoundingClientRect();
        const clientHeight = el.clientHeight;
        if (clientHeight <= 0 || box.height <= 0) return null;
        // getBoundingClientRect is scaled by an ancestor CSS `zoom`; clientHeight
        // and scrollTop are not. Their ratio is the zoom, so every row is
        // converted to the same unzoomed px as scrollTop.
        const zoom = box.height / el.offsetHeight || 1;
        const top = box.top + el.clientTop * zoom;
        const rows: RowBox[] = [];
        for (const row of el.querySelectorAll<HTMLElement>(
            ":scope > .agent-document-virtualizer > [data-node-id], :scope > .agent-document-streaming-buffer > [data-node-id]",
        )) {
            const r = row.getBoundingClientRect();
            if (r.height === 0 && r.width === 0) continue;
            rows.push({
                id: row.dataset.nodeId!,
                type: row.dataset.nodeType ?? "?",
                top: (r.top - top) / zoom,
                bottom: (r.bottom - top) / zoom,
            });
        }
        return {
            t: performance.now(),
            following: el.dataset.followState === "following",
            userInput: performance.now() - this.lastInputAt <= USER_INPUT_EXEMPT_MS,
            scrollTop: el.scrollTop,
            scrollHeight: el.scrollHeight,
            clientHeight,
            clientWidth: el.clientWidth,
            zoom,
            bottomPad: this.bottomPad,
            rows,
        };
    }

    private sample(): void {
        if (!this.running) return;
        const cur = this.read();
        const events = this.events;
        this.events = [];
        if (!cur) {
            this.prev = null;
            return;
        }
        const prev = this.prev;
        this.prev = cur;
        if (!prev) return;
        const s = this.stats;
        s.framesChecked++;
        const { violation, gap } = checkFrame(prev, cur, events);
        if (cur.following && !cur.userInput) {
            s.framesFollowing++;
            if (gap > 0) {
                s.blankFrames++;
                s.maxGap = Math.max(s.maxGap, gap);
            }
        }
        if (!violation) return;
        if (violation.kind === "down") s.down++;
        else s.overshoot++;
        s.maxDy = Math.max(s.maxDy, violation.dy);
        const key = violation.causes[0] ?? "unknown";
        s.byCause[key] = (s.byCause[key] ?? 0) + 1;
        s.recent.push(violation);
        if (s.recent.length > RING_SIZE) s.recent.shift();
        this.log(violation);
    }

    private log(v: Violation): void {
        const now = performance.now();
        this.logTimes = this.logTimes.filter((t) => now - t < LOG_WINDOW_MS);
        if (this.logTimes.length >= LOG_BURST) return;
        this.logTimes.push(now);
        console.info(
            "[one-way]",
            `pane=${this.pane}`,
            `kind=${v.kind}`,
            `dy=${v.dy.toFixed(1)}px`,
            `rows=${v.rows}`,
            `row=${v.rowType}:${v.rowId.slice(0, 8)}`,
            v.gapBefore > 0 ? `gapBefore=${v.gapBefore.toFixed(1)}px` : "",
            `causes=${v.causes.join(",") || "unknown"}`,
        );
    }
}

// ── Registry and the devtools / CDP handle ───────────────────────────────────

const recorders = new Map<string, OneWayRecorder>();
let enabled = readEnabled();

function readEnabled(): boolean {
    try {
        return globalThis.localStorage?.getItem(STORAGE_KEY) === "1";
    } catch {
        return false;
    }
}

function writeEnabled(on: boolean): void {
    try {
        if (on) globalThis.localStorage?.setItem(STORAGE_KEY, "1");
        else globalThis.localStorage?.removeItem(STORAGE_KEY);
    } catch {
        // storage unavailable: the setting just doesn't persist
    }
}

/**
 * Bind a recorder to a transcript scroller. Returns its cleanup. The recorder
 * samples only while the registry is enabled.
 */
export function attachOneWayRecorder(pane: string, el: HTMLElement): () => void {
    const rec = new OneWayRecorder(pane, el);
    recorders.get(pane)?.dispose();
    recorders.set(pane, rec);
    if (enabled) rec.start();
    return () => {
        rec.dispose();
        if (recorders.get(pane) === rec) recorders.delete(pane);
    };
}

/** Record that the code itself did something that can move the content (a pin, a hold, a glide). */
export function noteOneWay(pane: string, label: string): void {
    if (!enabled) return;
    recorders.get(pane)?.note(label);
}

export const oneWayRecorder = {
    enable(): void {
        enabled = true;
        writeEnabled(true);
        for (const r of recorders.values()) r.start();
    },
    disable(): void {
        enabled = false;
        writeEnabled(false);
        for (const r of recorders.values()) r.stop();
    },
    isEnabled: (): boolean => enabled,
    reset(): void {
        for (const r of recorders.values()) r.reset();
    },
    snapshot(): RecorderStats[] {
        return [...recorders.values()].map((r) => ({ ...r.stats, recent: [...r.stats.recent] }));
    },
};

if (typeof window !== "undefined") {
    (window as unknown as { __agentmuxOneWay?: typeof oneWayRecorder }).__agentmuxOneWay = oneWayRecorder;
}
