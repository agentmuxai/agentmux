// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A trace of "show this pane" requests and of the Swarm list rebuilding under
 * the pointer, so the intermittent lost click becomes data
 * (docs/specs/SPEC_PANE_SELECTION_STATE_MACHINE_AND_SWARM_CLICK_2026_10_18.md §6.4).
 *
 * Three things are recorded:
 *  - each reveal: who asked, how long it took, whether it found the block, and
 *    whether another reveal was still running when it started;
 *  - each Swarm rebuild, with the signals that caused it, summed into one line a
 *    minute so a busy fleet does not flood the log;
 *  - each press on an agent row whose click never arrived, and whether the row
 *    that was pressed had left the document by then.
 *
 * It changes no behaviour. The last entries are readable from the console as
 * `__selectTrace()`.
 */

const MAX_ENTRIES = 100;
const SUMMARY_MS = 60_000;
/** A click follows its release at once; this long after, it is not coming. */
export const CLICK_GRACE_MS = 400;

export interface TraceEntry {
    at: number;
    line: string;
}

export interface TraceCounters {
    rebuilds: number;
    rebuildsByCause: Record<string, number>;
    presses: number;
    lostClicks: number;
    lostClicksRowGone: number;
    reveals: number;
    overlappingReveals: number;
}

export interface SelectTraceOptions {
    now?: () => number;
    log?: (line: string) => void;
    setTimer?: (fn: () => void, ms: number) => unknown;
    clearTimer?: (handle: unknown) => void;
}

const short = (id: string | null | undefined): string => (id ? id.slice(0, 8) : "-");

function emptyCounters(): TraceCounters {
    return {
        rebuilds: 0,
        rebuildsByCause: {},
        presses: 0,
        lostClicks: 0,
        lostClicksRowGone: 0,
        reveals: 0,
        overlappingReveals: 0,
    };
}

export function createSelectTrace(opts: SelectTraceOptions = {}) {
    const now = opts.now ?? (() => Date.now());
    const log = opts.log ?? ((line: string) => console.info(line));
    const setTimer = opts.setTimer ?? ((fn, ms) => setTimeout(fn, ms));
    const clearTimer = opts.clearTimer ?? ((h) => clearTimeout(h as ReturnType<typeof setTimeout>));

    const entries: TraceEntry[] = [];
    let counters = emptyCounters();
    let lastSummaryAt = now();
    let revealSeq = 0;
    let revealsInFlight = 0;
    let pendingCauses = new Set<string>();

    let press: { blockId: string; el: { isConnected: boolean }; downAt: number } | null = null;
    let graceTimer: unknown = null;

    const record = (line: string, alsoLog: boolean) => {
        entries.push({ at: now(), line });
        if (entries.length > MAX_ENTRIES) entries.shift();
        if (alsoLog) log(line);
    };

    const flushSummary = () => {
        const c = counters;
        lastSummaryAt = now();
        if (c.rebuilds === 0 && c.presses === 0 && c.reveals === 0) return;
        const causes = Object.entries(c.rebuildsByCause)
            .sort((a, b) => b[1] - a[1])
            .map(([k, v]) => `${k}=${v}`)
            .join(" ");
        record(
            `[swarm] last ${Math.round(SUMMARY_MS / 1000)}s: rebuilds=${c.rebuilds}${causes ? ` (${causes})` : ""} ` +
                `presses=${c.presses} lost=${c.lostClicks} (row gone=${c.lostClicksRowGone}) ` +
                `reveals=${c.reveals} overlapping=${c.overlappingReveals}`,
            true
        );
        counters = emptyCounters();
    };

    const maybeFlush = () => {
        if (now() - lastSummaryAt >= SUMMARY_MS) flushSummary();
    };

    return {
        /** A signal the Swarm list reads has changed; the next rebuild is blamed on it. */
        cause(name: string) {
            pendingCauses.add(name);
        },

        /** The Swarm list was rebuilt. */
        rebuilt() {
            const causes = pendingCauses.size > 0 ? [...pendingCauses] : ["other"];
            pendingCauses = new Set();
            counters.rebuilds++;
            for (const c of causes) counters.rebuildsByCause[c] = (counters.rebuildsByCause[c] ?? 0) + 1;
            maybeFlush();
        },

        /** A reveal started. Returns a function to call with its outcome. */
        revealStarted(source: string, blockId: string): (found: boolean, note?: string) => void {
            const id = ++revealSeq;
            const overlap = revealsInFlight > 0;
            revealsInFlight++;
            counters.reveals++;
            if (overlap) counters.overlappingReveals++;
            const startedAt = now();
            let ended = false;
            return (found: boolean, note?: string) => {
                if (ended) return;
                ended = true;
                revealsInFlight--;
                const ms = now() - startedAt;
                const flags = `${found ? "" : " NOT-FOUND"}${overlap ? " OVERLAP" : ""}${note ? ` ${note}` : ""}`;
                record(`[select] #${id} src=${source} blk=${short(blockId)} +${ms}ms${flags}`, !found || overlap || ms > 700);
                maybeFlush();
            };
        },

        /** The primary button went down on an agent row. */
        pressed(blockId: string, el: { isConnected: boolean }) {
            if (graceTimer != null) clearTimer(graceTimer);
            graceTimer = null;
            press = { blockId, el, downAt: now() };
            counters.presses++;
        },

        /** The button was released (or the press cancelled): a click should follow at once. */
        released() {
            const p = press;
            if (!p) return;
            if (graceTimer != null) clearTimer(graceTimer);
            graceTimer = setTimer(() => {
                graceTimer = null;
                if (press !== p) return;
                press = null;
                const rowGone = !p.el.isConnected;
                counters.lostClicks++;
                if (rowGone) counters.lostClicksRowGone++;
                record(
                    `[swarm] press on blk=${short(p.blockId)} held ${now() - p.downAt - CLICK_GRACE_MS}ms got no click; ` +
                        `row ${rowGone ? "had left the document (rebuilt under the pointer)" : "was still in place"}`,
                    true
                );
            }, CLICK_GRACE_MS);
        },

        /** A click arrived on the row with this block id and it is still in the document:
         *  the press was not lost. A click that landed on the list instead (the pressed
         *  row was replaced) does not count. */
        clicked(blockId: string | null, el: { isConnected: boolean } | null) {
            if (!press || !el?.isConnected || blockId !== press.blockId) return;
            if (graceTimer != null) clearTimer(graceTimer);
            graceTimer = null;
            press = null;
        },

        flushSummary,
        entries: (): readonly TraceEntry[] => entries,
        counters: (): Readonly<TraceCounters> => counters,
    };
}

export const selectTrace = createSelectTrace();

(globalThis as { __selectTrace?: () => string[] }).__selectTrace = () =>
    selectTrace.entries().map((e) => `${new Date(e.at).toISOString()} ${e.line}`);
