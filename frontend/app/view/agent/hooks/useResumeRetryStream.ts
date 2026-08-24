// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useResumeRetryStream — the single per-block WPS subscription for
 * `agent-resume-retry`, published by the persistent controller's
 * stale-`--resume` recovery path (`retry_after_resume_failure` /
 * `publish_resume_retry_status` in `agentmux-srv`) so the pane can show a
 * "Reconnecting…" readout instead of going silent for the
 * seconds-to-tens-of-seconds it can take to detect a stale registry
 * `session_id` and respawn against a recovered one. See
 * docs/status/STATUS_STALE_RESUME_LIVE_REPRO_AND_FIX_PLAN_2026_08_23.md §6.2.
 *
 * Simpler than `useCompactionStream.ts`'s sibling hook in one respect: both
 * the "retrying" and "resolved" ends of this signal travel over this SAME
 * WPS channel (backend publishes with `persist: 2`, keeping the latest
 * pair), so there's no cross-channel staleness race to guard against — a
 * fresh subscribe from a genuinely new WebSocket connection always replays
 * the correct current pair.
 *
 * reagentx P1 (PR #2776, round 2): a same-connection pane unmount+remount
 * (switching tabs/panes away and back) does NOT get a fresh replay —
 * `Broker::replay_to_route` (`agentmux-srv/src/backend/wps.rs`) dedupes
 * replay per `(route_id, event, scope)` and is only cleared on a true
 * route reconnect (`unsubscribe_all`), while `registerPane` resets
 * `AgentPaneState.reconnecting` to `null` on every mount regardless. Relying
 * on `waveEventSubscribe`'s replay alone would silently reproduce this PR's
 * own "did it crash?" gap for exactly the pane-hidden-during-a-retry case.
 * Fixed by explicitly reading current history via `EventReadHistoryCommand`
 * on mount (same pattern `sysinfo-model.ts`'s `loadInitialData` already
 * uses) instead of depending solely on subscribe-time replay — this reads
 * the SAME persisted history replay would have delivered, just via a path
 * that isn't deduped per-route.
 */

import { onCleanup } from "solid-js";
import { waveEventSubscribe } from "@/app/store/wps";
import { WpsEvent } from "@/app/store/wps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { AgentPaneModel } from "@/app/store/agent-pane-model";

export interface UseResumeRetryStreamOptions {
    blockId: string;
    model: AgentPaneModel;
}

/**
 * Resolve a raw `agent-resume-retry` WPS payload into a dispatchable pane
 * command, or `null` to ignore it (malformed shape). Pure and exported for
 * direct unit coverage, same rationale as `resolveCompactionStart`.
 */
export function resolveResumeRetryEvent(
    data: unknown,
    now: number,
): { type: "ResumeRetryStarted"; at: number } | { type: "ResumeRetryResolved" } | null {
    if (!data || typeof data !== "object") return null;
    const d = data as Record<string, unknown>;
    if (d.status === "resolved") return { type: "ResumeRetryResolved" };
    if (d.status !== "retrying") return null;
    const at = typeof d.startedAt === "string" ? Date.parse(d.startedAt) : NaN;
    return { type: "ResumeRetryStarted", at: Number.isNaN(at) ? now : at };
}

export function useResumeRetryStream(opts: UseResumeRetryStreamOptions): void {
    const applyEvent = (data: unknown) => {
        const command = resolveResumeRetryEvent(data, Date.now());
        if (!command) return;
        // model.dispatchPane is the disposal-safe wrapper — required here,
        // not the raw (throwing) dispatch: this can run after the pane's
        // slot unregisters (a live event racing the unmount, or the
        // initial-history fetch below resolving after an already-fast
        // unmount), mirroring useDockClearStream's identical reasoning.
        opts.model.dispatchPane(command, "system");
    };

    const unsub = waveEventSubscribe({
        eventType: WpsEvent.AgentResumeRetry,
        scope: `block:${opts.blockId}`,
        handler: (event: any) => applyEvent(event?.data),
    });

    // Explicit current-history read on mount — see this module's doc
    // comment (reagentx P1, round 2) for why subscribe-time replay alone
    // isn't enough on a same-connection pane remount. `maxitems: 1` only
    // needs the single most recent status; `read_event_history` returns
    // oldest-first, so the last (only) element is current truth.
    void RpcApi.EventReadHistoryCommand(TabRpcClient, {
        event: WpsEvent.AgentResumeRetry,
        scope: `block:${opts.blockId}`,
        maxitems: 1,
    })
        .then((history) => {
            const latest = history?.[history.length - 1];
            if (latest) applyEvent(latest.data);
        })
        .catch((e) => {
            console.log("[useResumeRetryStream] failed to load initial reconnect status", e);
        });

    // Own the subscription at body scope so it is torn down even if the
    // caller's onMount early-returns (e.g. enabled:false).
    onCleanup(() => { try { unsub(); } catch { /* ignore */ } });
}
