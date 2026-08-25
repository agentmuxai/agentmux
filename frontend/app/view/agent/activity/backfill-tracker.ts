// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tracks which blocks currently have a subagent/dispatch backfill in
 * progress (`subagent:backfill_status`, `subagent_watcher/scan.rs`),
 * globally across every open pane — shared by `dispatch-source.ts` and
 * `subagent-source.ts` so both app-wide singletons can suppress their own
 * per-event refresh WHILE a backfill is running and fire exactly one
 * settle-refresh once it's genuinely done, instead of relying purely on
 * `debounced-refresh.ts`'s timing window to approximate settlement.
 *
 * Root cause this closes: per
 * docs/retro/retro-activity-dock-flicker-survives-debounce-fix-2026-08-24.md
 * §4, "the debounce coalesces request *volume*, not visual *settlement*" —
 * even the 2-3 refreshes that survive the debounce during a backfill burst
 * are each a genuinely different, real, but still-converging snapshot, so
 * Activity Dock rows still visibly appear-then-vanish as each one lands.
 * Suppressing refreshes entirely until the backend itself reports the
 * backfill done, then firing exactly one, removes those intermediate
 * partial-snapshot renders rather than just reducing their count.
 *
 * Deliberately much simpler than `useSubagentBackfillGate.ts`'s per-block
 * gate (8 review rounds of edge cases closed there, per that file's own
 * history): this is a best-effort refresh-suppression optimization, not a
 * correctness-critical gate blocking pane reveal. If a block's "done" event
 * is somehow missed entirely (e.g. a dropped WS connection, or the owning
 * pane closes mid-backfill with no further activity), `PANE_CLOSED_OR_LOST_MS`
 * below fails that one block open again on a timer — unlike the spinner
 * gate there's no user-visible "stuck forever" failure mode to guard
 * against (worst case without this fallback: dock refreshes stay suppressed
 * app-wide, which `debounced-refresh.ts`'s own `maxWaitMs` ceiling would
 * still eventually override for any consumer that kept receiving events —
 * but a block that stops producing events entirely, e.g. because its pane
 * closed, would never trigger even that ceiling, so the fallback still
 * matters).
 */

import { waveEventSubscribe } from "@/app/store/wps";

const backfillingBlocks = new Map<string, ReturnType<typeof setTimeout>>();
const settleListeners = new Set<() => void>();

/** See this module's doc comment — bounds how long a single block can keep
 *  every dock refresh app-wide suppressed if its own "done" event is lost. */
const PANE_CLOSED_OR_LOST_MS = 20_000;

function fireSettleListeners(): void {
    if (settleListeners.size === 0) return;
    const listeners = Array.from(settleListeners);
    settleListeners.clear();
    for (const listener of listeners) listener();
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

/** True while any open pane's subagent/dispatch history is still
 *  backfilling — `dispatch-source.ts`/`subagent-source.ts` consult this to
 *  decide whether to suppress an event-triggered refresh. */
export function isAnyBlockBackfilling(): boolean {
    return backfillingBlocks.size > 0;
}

/** Register a one-shot callback fired the next time every currently-tracked
 *  backfill finishes (`backfillingBlocks` transitions to empty). A caller
 *  with nothing in flight right now (`isAnyBlockBackfilling()` already
 *  false) has nothing to wait for and should not register. */
export function onNextBackfillSettle(listener: () => void): () => void {
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
            setTimeout(() => {
                backfillingBlocks.delete(blockId);
                if (backfillingBlocks.size === 0) fireSettleListeners();
            }, PANE_CLOSED_OR_LOST_MS)
        );
        return;
    }

    // status === "done"
    clearTimeout(backfillingBlocks.get(blockId));
    backfillingBlocks.delete(blockId);
    if (backfillingBlocks.size === 0) fireSettleListeners();
}

/**
 * Wrap an event-triggered refresh trigger so that, while any block is
 * backfilling, no refresh fires at all for this event — it's silently
 * absorbed — and exactly one refresh fires once every tracked backfill
 * settles. When nothing is backfilling, delegates straight through to
 * `scheduleDebouncedRefresh` (today's existing debounce behavior, unchanged,
 * for live responsiveness once backfill isn't a factor).
 *
 * Shared by `dispatch-source.ts` and `subagent-source.ts` — each still owns
 * its own `createDebouncedRefresh` instance (passed in as
 * `scheduleDebouncedRefresh`) and its own `refresh()` (passed in as
 * `refreshNow`); this only decides WHICH of the two to call per event.
 */
export function createBackfillAwareTrigger(scheduleDebouncedRefresh: () => void, refreshNow: () => void): () => void {
    let pendingSettleUnsub: (() => void) | null = null;
    return function trigger(): void {
        if (isAnyBlockBackfilling()) {
            if (!pendingSettleUnsub) {
                pendingSettleUnsub = onNextBackfillSettle(() => {
                    pendingSettleUnsub = null;
                    refreshNow();
                });
            }
            return;
        }
        scheduleDebouncedRefresh();
    };
}

// Started once at module load (ES modules are singletons — every importer
// shares this one subscription and the one `backfillingBlocks` map behind
// it), mirroring `dispatch-source.ts`/`subagent-source.ts`'s own lifecycle.
// No `scope` — this needs every open pane's block, not just one, unlike
// `useSubagentBackfillGate.ts`'s per-block-scoped subscription (that hook
// and this tracker both listen to the same backend event independently and
// safely: `wps.ts`'s `dispatchToSubjects` fans one incoming message out to
// every registered listener, filtering by each listener's own `scope`).
waveEventSubscribe({
    eventType: "subagent:backfill_status",
    handler: (event: { scopes?: string[]; data?: unknown }) => handleBackfillStatusEvent(event?.scopes, event?.data),
});
