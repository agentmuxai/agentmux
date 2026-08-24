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
 * `block.tsx`, same as `ready()` itself) — resolves immediately for any
 * block whose `viewType()` isn't `"agent"`, since only agent panes ever
 * have subagent history to backfill in the first place.
 *
 * reagentx P1 + codex P1 (PR #2781, round 2): an earlier version defaulted
 * `settled` to `true` (only flipping `false` once a "started" status was
 * actually observed, to avoid gating a fresh agent with nothing to
 * backfill). That was backwards — `blockData()`/`viewModel()` (block.tsx)
 * typically resolve from the local object store well before this hook's
 * async subscribe+RPC round trip completes, so `ready()` would go
 * true→false→true: briefly true (spinner fades, `<Show when={ready()}>`
 * mounts the real content), then false once "started" actually lands. Codex
 * traced the real damage: unmounting disposes `AgentPresentationView`,
 * whose cleanup calls `handleAgentIdChange(blockId, undefined)` and
 * unregisters the reactive agent — remounting on the next `ready()===true`
 * re-registers it, which re-triggers ANOTHER backfill scan, which can cycle
 * indefinitely. `ready()` must never go true before this hook has an actual
 * answer. Fixed by defaulting to NOT settled for agent blocks specifically
 * (monotonic: starts false, becomes true at most once per mount, never
 * flips back) and resolving non-agent blocks to `true` explicitly and
 * immediately, rather than via the same default.
 *
 * Mirrors `useResumeRetryStream.ts`'s live-subscribe + mount-time
 * `EventReadHistoryCommand` read + discard-on-live-race shape.
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

/**
 * codex P2 (PR #2781, round 2): the backend publishes "done" the instant
 * its own file-scan returns, but `dispatch-source.ts`/`subagent-source.ts`
 * only SCHEDULE their own trailing-edge-debounced `ListActive`/
 * `ListDispatches` refresh in reaction to the same burst of events
 * (`debounced-refresh.ts`, 100ms window) — so revealing the pane the
 * instant "done" lands can still race ahead of the Activity Dock's own
 * data actually catching up. Not a guarantee (a slow network response
 * could still exceed this), but a pragmatic buffer comfortably covering
 * the common case: 100ms debounce window + a healthy margin for the
 * dock's own RPC round trip to resolve.
 */
const DOCK_SETTLE_BUFFER_MS = 250;

export function useSubagentBackfillGate(blockId: string, viewType: Accessor<string | undefined>): Accessor<boolean> {
    const [settled, setSettled] = createSignal(false);
    let wired = false;
    let settleTimer: ReturnType<typeof setTimeout> | undefined;

    createEffect(() => {
        const vt = viewType();
        if (vt === undefined) return; // not yet resolved — stay gated, nothing to decide yet
        if (vt !== "agent") {
            // Only agent panes ever get a backfill_status event at all —
            // resolve immediately rather than via the same async path, so
            // every non-agent block's `ready()` is completely unaffected
            // by this hook (matches its pre-this-PR behavior exactly).
            setSettled(true);
            return;
        }
        if (wired) return;
        wired = true;

        let receivedLiveEvent = false;
        const scheduleSettle = () => {
            clearTimeout(settleTimer);
            settleTimer = setTimeout(() => setSettled(true), DOCK_SETTLE_BUFFER_MS);
        };
        const applyStatus = (data: unknown) => {
            const status = resolveBackfillStatus(data);
            if (status === "started") {
                clearTimeout(settleTimer);
                setSettled(false);
            } else if (status === "done") {
                scheduleSettle();
            }
        };

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
        // triggered for this block (e.g. a fresh agent with no persisted
        // session id) — resolve settled immediately rather than waiting on
        // an event that will never arrive.
        void RpcApi.EventReadHistoryCommand(TabRpcClient, {
            event: WpsEvent.SubagentBackfillStatus,
            scope: `block:${blockId}`,
            maxitems: 2,
        })
            .then((history) => {
                if (receivedLiveEvent) return;
                const latest = history?.[history.length - 1];
                if (latest) {
                    applyStatus(latest.data);
                } else {
                    setSettled(true);
                }
            })
            .catch((e) => {
                // Fail open, not stuck: an RPC error here must never gate
                // `ready()` forever.
                console.log("[useSubagentBackfillGate] failed to load initial backfill status", e);
                setSettled(true);
            });

        onCleanup(() => {
            clearTimeout(settleTimer);
            try { unsub(); } catch { /* ignore */ }
        });
    });

    return settled;
}
