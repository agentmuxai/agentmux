// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The rest of an agent pane's restore window, loaded after its first screen
 * (SPEC_AGENT_OPEN_FIRST_SCREEN_THEN_REST_2026_10_19.md §3.2).
 *
 * The pane opens on ~64 KB of the transcript (open-tail.ts). The earlier part
 * of what it used to read in one go (up to 10,000 lines and the live feed's
 * 1 MB) loads here, a page at a time, on idle, and prepends above what is
 * showing: the same older-page path a scroll-up uses, so the rows on screen
 * don't move. It never holds the user up: it waits while the user is active or
 * the pane is hidden, and ends when the pane closes.
 *
 * Pure of Solid and RPC: `useHistoryPagination` supplies the page read.
 */

/**
 * The UTF-8 length of `text` in bytes, the unit of `tail_bytes` and the feed's restore
 * budget. `text.length` counts UTF-16 code units, which is a third of the bytes of CJK
 * text and a half of an emoji's. A lone surrogate counts as 3, as `TextEncoder` writes
 * its replacement character.
 */
export function utf8Bytes(text: string): number {
    let bytes = 0;
    for (let i = 0; i < text.length; i++) {
        const c = text.charCodeAt(i);
        if (c < 0x80) bytes += 1;
        else if (c < 0x800) bytes += 2;
        else if (c >= 0xd800 && c <= 0xdbff && i + 1 < text.length && (text.charCodeAt(i + 1) & 0xfc00) === 0xdc00) {
            bytes += 4;
            i++;
        } else bytes += 3;
    }
    return bytes;
}

/** The bytes of `lines` as stored: each line and its newline. */
export function linesBytes(lines: readonly string[]): number {
    let bytes = 0;
    for (const line of lines) bytes += utf8Bytes(line) + 1;
    return bytes;
}

/** How long to wait before looking again while the user is active or the pane hidden. */
export const FILL_HOLD_RECHECK_MS = 250;

export interface FillPlan {
    /** The lowest line the fill reaches: the restore window's start. */
    floorLine: number;
    /** Transcript bytes the fill may still load. */
    budgetBytes: number;
}

/**
 * What to fill below a first screen that starts at `startLine`, or `null` when
 * nothing is left: the slice already reaches the window's start or the feed's
 * byte budget. `windowLines`: the restore window's size in lines.
 * `restoreBytes`: what the live feed keeps (absent: no byte cap, the line
 * window alone bounds it).
 */
export function planFill(args: {
    startLine: number;
    endLine: number;
    sliceBytes: number;
    windowLines: number;
    restoreBytes?: number;
}): FillPlan | null {
    const floorLine = Math.max(0, args.endLine - args.windowLines);
    if (args.startLine <= floorLine) return null;
    const budgetBytes = args.restoreBytes === undefined ? Number.POSITIVE_INFINITY : args.restoreBytes - args.sliceBytes;
    if (budgetBytes <= 0) return null;
    return { floorLine, budgetBytes };
}

export interface FillPageResult {
    lines: number;
    bytes: number;
    /** The page was cut by the byte budget, so nothing older is wanted. */
    last?: boolean;
}

export interface FillDeps {
    plan: FillPlan;
    pageLines: number;
    /** Where the document's earliest loaded line sits now. */
    offset(): number;
    /**
     * Load the page of up to `maxLines` lines just below `offset()` and prepend
     * it. `null`: nothing loaded (the read failed, or another reader holds
     * the pages); it ends the fill unless `stopped()` says it should wait.
     */
    page(maxLines: number): Promise<FillPageResult | null>;
    /** The pane closed: end now. */
    stopped(): boolean;
    /** The user is active or the pane is hidden: wait, then look again. */
    holding(): boolean;
    /** Resolve at the next moment the main thread is idle. */
    idle(): Promise<void>;
    /** Resolve after `ms`. */
    wait(ms: number): Promise<void>;
}

export interface FillOutcome {
    outcome: "done" | "stopped" | "failed";
    pages: number;
    lines: number;
    bytes: number;
}

/**
 * Run the fill: pages until the window's floor or the byte budget is reached.
 * A page that is over budget still comes in whole, so the budget is a bound to
 * the next page, not an exact cut.
 */
export async function runHistoryFill(deps: FillDeps): Promise<FillOutcome> {
    const out: FillOutcome = { outcome: "done", pages: 0, lines: 0, bytes: 0 };
    let budget = deps.plan.budgetBytes;
    for (;;) {
        if (deps.stopped()) return { ...out, outcome: "stopped" };
        const room = deps.offset() - deps.plan.floorLine;
        if (room <= 0 || budget <= 0) return out;
        if (deps.holding()) {
            await deps.wait(FILL_HOLD_RECHECK_MS);
            continue;
        }
        await deps.idle();
        if (deps.stopped()) return { ...out, outcome: "stopped" };
        // The user may have started something while waiting for idle.
        if (deps.holding()) continue;
        const res = await deps.page(Math.min(deps.pageLines, room));
        if (res === null) return { ...out, outcome: deps.stopped() ? "stopped" : "failed" };
        // A page that moved nothing would repeat forever.
        if (deps.offset() >= room + deps.plan.floorLine) return { ...out, outcome: "failed" };
        out.pages++;
        out.lines += res.lines;
        out.bytes += res.bytes;
        budget -= res.bytes;
        if (res.last) return out;
    }
}
