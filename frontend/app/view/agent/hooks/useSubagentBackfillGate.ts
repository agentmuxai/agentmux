// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useSubagentBackfillGate — tracks whether THIS block's own subagent/
 * dispatch history is still cold-backfilling on pane (re)open, so
 * `block.tsx`'s `ready()` gate (the BrainSpinner overlay) can stay up until
 * it's genuinely done, instead of fading as soon as this block's own
 * `blockData`/`viewModel` resolve — which happens well before the Activity
 * Dock's data (a separate, app-wide singleton source) has settled. See
 * docs/retro/retro-activity-dock-flicker-survives-debounce-fix-2026-08-24.md
 * §5, options 1-2.
 *
 * Generic over every block type by design (called unconditionally from
 * `block.tsx`, same as `ready()` itself) — internally a no-op for any block
 * whose `viewType()` isn't `"agent"`, since only agent panes ever have
 * subagent history to backfill in the first place.
 *
 * Mirrors `useResumeRetryStream.ts`'s exact shape and the same reagentx
 * round-2 fixes that hook went through: a live `waveEventSubscribe` PLUS an
 * explicit mount-time `EventReadHistoryCommand` read (since
 * `agentmux-srv/src/backend/subagent_watcher/scan.rs`'s backfill can
 * complete before this hook's subscription is even registered — the
 * backend's own scan runs synchronously inside the same reactive-
 * registration RPC handler that triggers this block's mount in the first
 * place), with the same "discard a stale history read that loses a race to
 * a live event" guard.
 *
 * Defaults to `settled` (not gated) rather than pessimistically gating on
 * every agent block: a block with no persisted session id never has
 * `scan_session_subagents` called for it at all (see
 * `agentmux-srv/src/server/reactive.rs`), so it would never publish either
 * status and a pessimistic default would leave `ready()` stuck forever —
 * the exact "stuck BrainSpinner" failure class
 * `docs/retro/retro-block-ready-gate-spinner-stuck-visible-race-2026-08-23.md`
 * already fixed once. Only flips to "not settled" once a "started" status
 * is actually observed (live or historical).
 */

import { createEffect, createSignal, onCleanup, type Accessor } from "solid-js";
import { waveEventSubscribe } from "@/app/store/wps";
import { WpsEvent } from "@/app/store/wps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

/**
 * Resolve a raw `subagent:backfill_status` WPS payload into `"started"` /
 * `"done"`, or `null` to ignore it (malformed shape). Pure and exported for
 * direct unit coverage, same rationale as `resolveResumeRetryEvent`.
 */
export function resolveBackfillStatus(data: unknown): "started" | "done" | null {
    if (!data || typeof data !== "object") return null;
    const status = (data as Record<string, unknown>).status;
    return status === "started" || status === "done" ? status : null;
}

export function useSubagentBackfillGate(blockId: string, viewType: Accessor<string | undefined>): Accessor<boolean> {
    const [settled, setSettled] = createSignal(true);
    let wired = false;

    createEffect(() => {
        if (wired || viewType() !== "agent") return;
        wired = true;

        const applyStatus = (data: unknown) => {
            const status = resolveBackfillStatus(data);
            if (status === "started") setSettled(false);
            else if (status === "done") setSettled(true);
        };

        let receivedLiveEvent = false;
        const unsub = waveEventSubscribe({
            eventType: WpsEvent.SubagentBackfillStatus,
            scope: `block:${blockId}`,
            handler: (event: any) => {
                receivedLiveEvent = true;
                applyStatus(event?.data);
            },
        });

        // Explicit current-history read on mount — same rationale as
        // useResumeRetryStream.ts's identical read: subscribe-time replay
        // alone doesn't cover a same-connection remount, and here the
        // backend's scan can even outrace a genuinely first-ever mount's
        // subscription. `maxitems: 2` reads the latest started/done pair
        // (backend publishes with `persist: 2`); the last element is
        // current truth. An empty result means no backfill was ever
        // triggered for this block — leave the default (`settled`) as-is.
        void RpcApi.EventReadHistoryCommand(TabRpcClient, {
            event: WpsEvent.SubagentBackfillStatus,
            scope: `block:${blockId}`,
            maxitems: 2,
        })
            .then((history) => {
                if (receivedLiveEvent) return;
                const latest = history?.[history.length - 1];
                if (latest) applyStatus(latest.data);
            })
            .catch((e) => {
                // Fail open, not stuck: an RPC error here must never gate
                // `ready()` forever.
                console.log("[useSubagentBackfillGate] failed to load initial backfill status", e);
            });

        onCleanup(() => { try { unsub(); } catch { /* ignore */ } });
    });

    return settled;
}
