// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * One structured log line per agent open — SPEC_AGENT_OPEN_LATENCY_2026_09_27.md
 * §4.6 (F6: the logs couldn't say how long an open took, or which part of it
 * was the system).
 *
 * A trace starts at the My Agents click (`source=my-agents`) or, for a pane
 * that mounts without one (restore at startup, a tear-off), at the agent
 * view's mount (`source=mount`). Each step marks its time since that start,
 * once. The clock stops when the pane is revealed and the page has gone
 * quiet — no frame gap over {@link QUIET_GAP_MS} for {@link QUIET_HOLD_MS} —
 * so a login or account prompt the pane shows afterwards is never counted as
 * open latency; the auth phase the pane landed in is recorded instead
 * (`auth=first-login` = the open ended at a login prompt).
 *
 * The line (host log, `[fe] ` prefix added by the log pipe), all times in ms
 * since the start:
 *
 *   [agent-open] agent="Lazo" block=1a2b3c4d source=my-agents outcome=quiet
 *     total=1341 cli=180 cli_source=local_install config=402 committed=470
 *     history_start=520 history_read=610 history_lines=5000 parsed=627
 *     painted=812 revealed=815 quiet=1341 auth=authenticated
 *
 * `muxlog opens` reads these (docs/MUXLOG.md).
 */

/** A frame gap longer than this ends "quiet". */
export const QUIET_GAP_MS = 50;
/** How long frames must stay regular to call the page quiet. */
export const QUIET_HOLD_MS = 1500;
/** After the reveal, give up waiting for quiet (busy page, hidden window). */
export const QUIET_CAP_MS = 15_000;
/** A trace that never reaches its reveal is logged and dropped after this. */
export const OPEN_TIMEOUT_MS = 120_000;

/** Steps, in the order they normally happen — also the line's key order. */
export const OPEN_PHASES = [
    "cli",
    "config",
    "committed",
    "history_start",
    "history_read",
    "parsed",
    "painted",
    "revealed",
    "quiet",
] as const;
export type OpenPhase = (typeof OPEN_PHASES)[number];

export type OpenOutcome =
    /** Revealed, then quiet. */
    | "quiet"
    /** Revealed, but never quiet within {@link QUIET_CAP_MS}. */
    | "unsettled"
    /** The launch itself failed. */
    | "failed"
    /** Never revealed within {@link OPEN_TIMEOUT_MS}. */
    | "timeout"
    /** The pane closed first. */
    | "closed"
    /** Another open of the same block replaced this one. */
    | "superseded";

export interface OpenTrace {
    blockId: string;
    agent: string;
    source: "my-agents" | "mount";
    start: number;
    marks: Partial<Record<OpenPhase, number>>;
    notes: Record<string, string>;
}

/** Clock and frame scheduling, swappable in tests. */
export const openTraceEnv = {
    now: (): number => performance.now(),
    log: (line: string): void => console.info(line),
    requestFrame: (cb: (t: number) => void): number => requestAnimationFrame(cb),
    cancelFrame: (id: number): void => cancelAnimationFrame(id),
};

interface Live {
    trace: OpenTrace;
    timeout: ReturnType<typeof setTimeout>;
    quietCap?: ReturnType<typeof setTimeout>;
    raf?: number;
}

const live = new Map<string, Live>();

/** Start a trace for `blockId`; an unfinished one is logged as superseded. */
export function beginAgentOpen(blockId: string, agent: string, source: OpenTrace["source"]): void {
    if (!blockId) return;
    if (live.has(blockId)) finishAgentOpen(blockId, "superseded");
    const trace: OpenTrace = { blockId, agent, source, start: openTraceEnv.now(), marks: {}, notes: {} };
    live.set(blockId, {
        trace,
        timeout: setTimeout(() => finishAgentOpen(blockId, "timeout"), OPEN_TIMEOUT_MS),
    });
}

/** Start a `mount` trace unless a click already started one for this block. */
export function beginAgentOpenOnMount(blockId: string, agent: string): void {
    if (!live.has(blockId)) beginAgentOpen(blockId, agent, "mount");
}

/**
 * Record `phase` and its notes, the first time only: ResolveCli runs on the
 * launch and again on mount, and the first one is the one that installed.
 * No-op without a trace.
 */
export function markAgentOpen(blockId: string, phase: OpenPhase, notes?: Record<string, string | number>): void {
    const l = live.get(blockId);
    if (!l || l.trace.marks[phase] !== undefined) return;
    l.trace.marks[phase] = openTraceEnv.now() - l.trace.start;
    noteAgentOpen(blockId, notes);
}

/** Attach notes (`key=value`) to the trace without marking a phase. */
export function noteAgentOpen(blockId: string, notes?: Record<string, string | number>): void {
    const l = live.get(blockId);
    if (!l || !notes) return;
    for (const [k, v] of Object.entries(notes)) l.trace.notes[k] = String(v);
}

/** The pane is revealed: mark it, then finish once the page is quiet. */
export function agentOpenRevealed(blockId: string): void {
    const l = live.get(blockId);
    if (!l || l.trace.marks.revealed !== undefined) return;
    markAgentOpen(blockId, "revealed");
    const detector = new QuietDetector();
    const onFrame = (t: number) => {
        if (live.get(blockId) !== l) return;
        const quietAt = detector.frame(t);
        if (quietAt !== null) {
            l.trace.marks.quiet = quietAt - l.trace.start;
            finishAgentOpen(blockId, "quiet");
            return;
        }
        l.raf = openTraceEnv.requestFrame(onFrame);
    };
    l.raf = openTraceEnv.requestFrame(onFrame);
    // Frames stop entirely in a hidden/occluded window, so the cap is a timer.
    l.quietCap = setTimeout(() => {
        if (typeof document !== "undefined" && document.visibilityState === "hidden") {
            noteAgentOpen(blockId, { hidden: 1 });
        }
        finishAgentOpen(blockId, "unsettled");
    }, QUIET_CAP_MS);
}

/** Log the trace's line and drop it. No-op without a trace. */
export function finishAgentOpen(blockId: string, outcome: OpenOutcome): void {
    const l = live.get(blockId);
    if (!l) return;
    live.delete(blockId);
    clearTimeout(l.timeout);
    clearTimeout(l.quietCap);
    if (l.raf !== undefined) openTraceEnv.cancelFrame(l.raf);
    const total = openTraceEnv.now() - l.trace.start;
    openTraceEnv.log(formatAgentOpenLine(l.trace, outcome, total));
}

/** Pure: the `[agent-open]` line for `trace`. */
export function formatAgentOpenLine(trace: OpenTrace, outcome: OpenOutcome, totalMs: number): string {
    const parts = [
        "[agent-open]",
        `agent=${JSON.stringify(trace.agent)}`,
        `block=${trace.blockId.slice(0, 8)}`,
        `source=${trace.source}`,
        `outcome=${outcome}`,
        `total=${Math.round(totalMs)}`,
    ];
    for (const phase of OPEN_PHASES) {
        const at = trace.marks[phase];
        if (at !== undefined) parts.push(`${phase}=${Math.round(at)}`);
        if (phase === "cli" && trace.notes.cli_source) parts.push(`cli_source=${trace.notes.cli_source}`);
        if (phase === "history_read" && trace.notes.history_lines) {
            parts.push(`history_lines=${trace.notes.history_lines}`);
        }
    }
    for (const [k, v] of Object.entries(trace.notes)) {
        if (k === "cli_source" || k === "history_lines") continue;
        parts.push(`${k}=${/\s/.test(v) ? JSON.stringify(v) : v}`);
    }
    return parts.join(" ");
}

/**
 * Pure: feed it frame timestamps; returns the time quiet began once frames
 * have come at most {@link QUIET_GAP_MS} apart for {@link QUIET_HOLD_MS}.
 */
export class QuietDetector {
    private last: number | null = null;
    private quietSince: number | null = null;

    frame(t: number): number | null {
        if (this.last === null || t - this.last > QUIET_GAP_MS) this.quietSince = t;
        this.last = t;
        return this.quietSince !== null && t - this.quietSince >= QUIET_HOLD_MS ? this.quietSince : null;
    }
}

/** Tests only: drop every live trace without logging. */
export function resetAgentOpenTracesForTests(): void {
    for (const l of live.values()) {
        clearTimeout(l.timeout);
        clearTimeout(l.quietCap);
    }
    live.clear();
}
