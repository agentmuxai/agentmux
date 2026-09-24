// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tracks which blocks currently have a subagent/dispatch backfill in
 * progress (`subagent:backfill_status`, `subagent_watcher/scan.rs`), shared by
 * `dispatch-source.ts` and `subagent-source.ts`. Both fetch ONE app-wide list
 * per refresh; while a pane backfills, `holdBackfillingRows` keeps THAT pane's
 * rows at their pre-backfill values and applies everyone else's, and the
 * trigger fires one refresh when the pane settles.
 *
 * Root cause this closes: per
 * docs/retro/retro-activity-dock-flicker-survives-debounce-fix-2026-08-24.md
 * §4, "the debounce coalesces request *volume*, not visual *settlement*" —
 * even the 2-3 refreshes that survive the debounce during a backfill burst
 * are each a genuinely different, real, but still-converging snapshot, so
 * Activity Dock rows still visibly appear-then-vanish as each one lands.
 *
 * Per pane, not app-wide (REPORT_AGENT_PANE_SIDE_BY_SIDE_SCROLL_AND_FOCUS_QUIRKS_2026_09_23.md
 * §6): this used to suppress EVERY refresh while ANY pane backfilled, so
 * opening one agent pane froze the Activity Dock of every other pane for up
 * to `PANE_CLOSED_OR_LOST_MS`.
 *
 * Deliberately much simpler than `useSubagentBackfillGate.ts`'s per-block
 * gate: this is a best-effort flicker optimization, not a gate blocking pane
 * reveal. If a block's "done" event is somehow missed entirely (a dropped WS
 * connection, or the owning pane closing mid-backfill), `PANE_CLOSED_OR_LOST_MS`
 * below settles that one block on a timer, releasing its held rows.
 */

import { muxEventSubscribe } from "@/app/store/mps";

const backfillingBlocks = new Map<string, ReturnType<typeof setTimeout>>();
const settleListeners = new Set<(blockId: string) => void>();

/** See this module's doc comment — bounds how long a single block's rows can
 *  stay held if its own "done" event is lost. */
const PANE_CLOSED_OR_LOST_MS = 20_000;

function settle(blockId: string): void {
    if (!backfillingBlocks.has(blockId)) return;
    clearTimeout(backfillingBlocks.get(blockId));
    backfillingBlocks.delete(blockId);
    for (const listener of Array.from(settleListeners)) listener(blockId);
}

/** Pure — exported for direct unit coverage, same rationale as
 *  `resolveBackfillStatus` in `useSubagentBackfillGate.ts` (deliberately not
 *  imported from there: that module owns the per-block spinner gate, this
 *  one owns an unrelated refresh-suppression optimization; sharing the
 *  parse function alone isn't worth a cross-concern dependency). */
export function parseBackfillStatusEvent(scopes: string[] | undefined, data: unknown): { blockId: string; status: "started" | "done" } | null {
    const scope = scopes?.find((s) => s.startsWith("block:"));
    if (!scope) return null;
    const blockId = scope.slice("block:".length);
    if (!blockId) return null;
    const status = (data as Record<string, unknown> | null | undefined)?.status;
    if (status !== "started" && status !== "done") return null;
    return { blockId, status };
}

/** True while `blockId`'s subagent/dispatch history is still backfilling. */
export function isBlockBackfilling(blockId: string): boolean {
    return backfillingBlocks.has(blockId);
}

/** Register a listener fired with the block id each time ONE block's backfill
 *  finishes — its "done" event, or the `PANE_CLOSED_OR_LOST_MS` fallback. */
export function onBackfillSettle(listener: (blockId: string) => void): () => void {
    settleListeners.add(listener);
    return () => settleListeners.delete(listener);
}

export function handleBackfillStatusEvent(scopes: string[] | undefined, data: unknown): void {
    const parsed = parseBackfillStatusEvent(scopes, data);
    if (!parsed) return;
    const { blockId, status } = parsed;

    if (status === "started") {
        clearTimeout(backfillingBlocks.get(blockId));
        backfillingBlocks.set(
            blockId,
            setTimeout(() => settle(blockId), PANE_CLOSED_OR_LOST_MS)
        );
        return;
    }
    settle(blockId);
}

/**
 * Apply a freshly fetched app-wide list, except for panes still backfilling:
 * their rows stay exactly as they were in `prev` until they settle. Every
 * other pane's rows come from `next`. This is what keeps one pane's history
 * replay from both (a) flashing its own converging snapshots and (b) freezing
 * every OTHER pane's Activity Dock, which the old app-wide suppression did for
 * up to `PANE_CLOSED_OR_LOST_MS`.
 */
export function holdBackfillingRows<T extends { parent_block_id: string }>(prev: readonly T[], next: T[]): T[] {
    if (backfillingBlocks.size === 0) return next;
    const fresh = next.filter((r) => !backfillingBlocks.has(r.parent_block_id));
    const held = prev.filter((r) => backfillingBlocks.has(r.parent_block_id));
    return [...fresh, ...held];
}

/**
 * Wrap an event-triggered refresh: always the debounced refresh (its result
 * goes through `holdBackfillingRows`, so a backfilling pane's rows don't move),
 * plus exactly ONE immediate refresh when a pane that saw events during its
 * backfill settles — so its rows land the moment it's done, without waiting
 * for any other pane's backfill.
 *
 * Shared by `dispatch-source.ts` and `subagent-source.ts` — each still owns
 * its own `createDebouncedRefresh` instance (passed in as
 * `scheduleDebouncedRefresh`) and its own `refresh()` (passed in as
 * `refreshNow`).
 */
export function createBackfillAwareTrigger(scheduleDebouncedRefresh: () => void, refreshNow: () => void): () => void {
    const sawEventsDuringBackfill = new Set<string>();
    onBackfillSettle((blockId) => {
        if (sawEventsDuringBackfill.delete(blockId)) refreshNow();
    });
    return function trigger(): void {
        for (const blockId of backfillingBlocks.keys()) sawEventsDuringBackfill.add(blockId);
        scheduleDebouncedRefresh();
    };
}

// Started once at module load (ES modules are singletons — every importer
// shares this one subscription and the one `backfillingBlocks` map behind
// it), mirroring `dispatch-source.ts`/`subagent-source.ts`'s own lifecycle.
// No `scope` — this needs every open pane's block, not just one, unlike
// `useSubagentBackfillGate.ts`'s per-block-scoped subscription (that hook
// and this tracker both listen to the same backend event independently and
// safely: `mps.ts`'s `dispatchToSubjects` fans one incoming message out to
// every registered listener, filtering by each listener's own `scope`).
muxEventSubscribe({
    eventType: "subagent:backfill_status",
    handler: (event: { scopes?: string[]; data?: unknown }) => handleBackfillStatusEvent(event?.scopes, event?.data),
});
