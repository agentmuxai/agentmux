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
 * actually observed). That was backwards — `blockData()`/`viewModel()`
 * (block.tsx) typically resolve before this hook's async subscribe+RPC
 * round trip completes, so `ready()` would go true→false→true, and because
 * content is wrapped in `<Show when={ready()}>`, that unmounted/remounted
 * the real pane content instead of merely covering it with the spinner.
 * Fixed (that round) by defaulting to NOT settled for agent blocks and
 * resolving non-agent blocks to `true` immediately.
 *
 * reagentx P0 (PR #2781, round 4): that fix was still unsound for the
 * COMMON case — an agent block WITH a persisted session id (i.e. every
 * ordinary reopen of a previously-used agent). On a genuine first mount,
 * `scan_session_subagents` (agentmux-srv/src/server/reactive.rs) hasn't
 * necessarily started broadcasting yet by the time this hook's
 * mount-time history read resolves, so the read comes back EMPTY — the
 * previous version treated "empty history" as "nothing will ever happen
 * here" and resolved `settled=true` immediately, exactly as unsound as the
 * round-2 bug it was meant to fix. The REAL "started" then arrived moments
 * later once the backend's registration-triggered scan actually ran,
 * flipping `settled` back to `false` — unmounting the just-mounted pane,
 * which (per round 2's own finding) re-registers and re-triggers ANOTHER
 * backfill scan, repeating indefinitely on every reopen of any agent with
 * a persisted session.
 *
 * Fixed by never inferring "will a backfill happen at all" from the
 * (inherently racy) history read. The caller now passes
 * `hasPersistedSession` — read synchronously from this block's own
 * `blockData().meta["agent:sessionid"]`, the EXACT same condition
 * `server/reactive.rs` gates `scan_session_subagents` on — so this hook
 * knows definitively, with no async round trip involved, whether a
 * backfill is coming. Only a block with NO persisted session (a fresh
 * agent — `scan_session_subagents` is never even called for it) resolves
 * immediately; every persisted-session agent block waits for a REAL
 * "done" (live or historical), never short-circuiting on an empty read.
 * A generous safety-net timeout still guards against getting stuck forever
 * if "done" never arrives for some unrelated reason (e.g. a dropped WS
 * connection) — failing open, not stuck, same posture as every other
 * safety net in this hook.
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
 * the common case.
 */
const DOCK_SETTLE_BUFFER_MS = 250;

/**
 * Safety net (PR #2781 round 4): fail open rather than get stuck forever
 * if "done" never arrives for a block that genuinely has a backfill
 * pending — comfortably above the ~14s worst-case total settle time
 * measured for a very heavy real backfill in
 * docs/reports/REPORT_AGENT_PANE_REOPEN_SUBAGENT_STORM_2026_08_23.md §1.
 */
const SETTLE_SAFETY_TIMEOUT_MS = 20_000;

export function useSubagentBackfillGate(
    blockId: string,
    viewType: Accessor<string | undefined>,
    hasPersistedSession: Accessor<boolean>,
): Accessor<boolean> {
    const [settled, setSettled] = createSignal(false);
    let wired = false;
    let settleTimer: ReturnType<typeof setTimeout> | undefined;
    let safetyTimer: ReturnType<typeof setTimeout> | undefined;

    createEffect(() => {
        const vt = viewType();
        if (vt === undefined) return; // not yet resolved — stay gated, nothing to decide yet
        if (vt !== "agent" || !hasPersistedSession()) {
            // Only an agent block WITH a persisted session id ever gets a
            // backfill_status event at all (server/reactive.rs gates
            // scan_session_subagents on the exact same condition) —
            // resolve immediately for everything else, rather than via the
            // same async path. See this module's own doc comment (round 4)
            // for why this must be known synchronously, not inferred from
            // an empty history read.
            setSettled(true);
            return;
        }
        if (wired) return;
        wired = true;

        safetyTimer = setTimeout(() => {
            console.log(
                `[useSubagentBackfillGate] block ${blockId}: no backfill "done" after ${SETTLE_SAFETY_TIMEOUT_MS}ms, revealing anyway`
            );
            setSettled(true);
        }, SETTLE_SAFETY_TIMEOUT_MS);

        let receivedLiveEvent = false;
        const scheduleSettle = () => {
            clearTimeout(settleTimer);
            settleTimer = setTimeout(() => {
                clearTimeout(safetyTimer);
                setSettled(true);
            }, DOCK_SETTLE_BUFFER_MS);
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
        // current truth. An EMPTY result here does NOT mean "resolve
        // settled" (see this module's own doc comment, round 4) — it just
        // means the backend's scan hasn't started broadcasting yet; the
        // live subscribe above (already registered) will catch the real
        // pair once it does, and the safety-net timer covers the case
        // where it somehow never does.
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
                console.log("[useSubagentBackfillGate] failed to load initial backfill status", e);
            });

        onCleanup(() => {
            clearTimeout(settleTimer);
            clearTimeout(safetyTimer);
            try { unsub(); } catch { /* ignore */ }
        });
    });

    return settled;
}
