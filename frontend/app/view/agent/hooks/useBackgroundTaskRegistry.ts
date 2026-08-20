// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useBackgroundTaskRegistry — seeds/refreshes the `attachedTask` axis from
 * the durable `db_background_tasks` registry (Phase A/B of
 * docs/specs/SPEC_BACKGROUND_TASK_DASHBOARD_INTELLIGENCE_2026_08_20.md),
 * instead of relying exclusively on this tab's own live transcript replay
 * (`earliestLiveAttachedStartMs`, dispatched from `agent-view.tsx`).
 *
 * Two triggers, both funneling into the same `refresh`:
 * - On mount: a task that survived a session restart under a NEW
 *   controller generation has no transcript history of ever being
 *   launched — the registry is the only place that still knows about it.
 * - On `background-task-updated` (a live, unpersisted invalidation ping —
 *   see `publish_background_task_updated` in websocket.rs): re-query
 *   rather than trust the event's own payload, since it carries no task
 *   data itself.
 *
 * Deliberately ONE-DIRECTIONAL: only ever dispatches `AttachedTaskObserved`
 * (an ADDITIVE floor — "at least this much is known to be running"),
 * never `AttachedTaskCleared`. The registry's snapshot can be a moment
 * stale relative to a real, still-in-flight local signal (e.g. the
 * transcript-derived path just observed a task this exact RPC response —
 * sampled a beat earlier — doesn't reflect yet); clearing from here could
 * incorrectly stomp that live state. Completion is a more specific signal
 * than "not currently Running in a possibly-stale snapshot" and stays the
 * transcript-derived path's job alone (its own `<task-notification>`
 * parse) — see spec §3.1's explicit framing of the registry as a floor,
 * not an override.
 *
 * Installed at BODY scope by the caller (mirrors `useToolChunkStream`/
 * `useCompactionStream`'s own convention) so the subscription tears down
 * even if the caller's own `onMount` early-returns.
 */

import { onCleanup, onMount } from "solid-js";
import { waveEventSubscribe } from "@/app/store/wps";
import { WpsEvent } from "@/app/store/wps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { AgentPaneModel } from "@/app/store/agent-pane-model";

export interface UseBackgroundTaskRegistryOptions {
    blockId: string;
    model: AgentPaneModel;
}

/** Pure: the `since` timestamp to observe from a set of registry rows, or
 * `null` if none are currently running (in which case the caller should
 * dispatch nothing — see the module doc comment for why). Exported for
 * direct unit coverage without a SolidJS reactive-root harness. */
export function resolveAttachedTaskObservation(tasks: BackgroundTaskView[]): number | null {
    const running = tasks.filter((t) => t.status === "running");
    if (running.length === 0) return null;
    return Math.min(...running.map((t) => t.started_at_ms));
}

async function refresh(opts: UseBackgroundTaskRegistryOptions): Promise<void> {
    let tasks: BackgroundTaskView[];
    try {
        tasks = await RpcApi.ListBackgroundTasksCommand(TabRpcClient, { blockid: opts.blockId });
    } catch {
        // Best-effort: the transcript-derived signal remains the primary
        // source and this hook gets another chance on the next
        // background-task-updated event — nothing user-visible to report.
        return;
    }
    const since = resolveAttachedTaskObservation(tasks);
    if (since != null) {
        opts.model.dispatchPane({ type: "AttachedTaskObserved", at: since });
    }
}

export function useBackgroundTaskRegistry(opts: UseBackgroundTaskRegistryOptions): void {
    onMount(() => {
        void refresh(opts);
    });

    const unsub = waveEventSubscribe({
        eventType: WpsEvent.BackgroundTaskUpdated,
        scope: `block:${opts.blockId}`,
        handler: () => {
            void refresh(opts);
        },
    });

    onCleanup(() => { try { unsub(); } catch { /* ignore */ } });
}
