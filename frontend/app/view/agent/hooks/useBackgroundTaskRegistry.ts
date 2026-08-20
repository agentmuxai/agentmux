// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useBackgroundTaskRegistry — seeds/refreshes `registryAttachedTaskSince`
 * (a SEPARATE axis from `attachedTask` — see its doc comment in
 * `agent-pane-state/types.ts`) from the durable `db_background_tasks`
 * registry (Phase A/B of
 * docs/specs/SPEC_BACKGROUND_TASK_DASHBOARD_INTELLIGENCE_2026_08_20.md),
 * instead of relying exclusively on this tab's own live transcript replay.
 *
 * Three triggers, all funneling into the same `refresh`:
 * - On mount: a task that survived a session restart under a NEW
 *   controller generation has no transcript history of ever being
 *   launched — the registry is the only place that still knows about it.
 * - On `background-task-updated` (a live, unpersisted invalidation ping —
 *   see `publish_background_task_updated` in websocket.rs): re-query
 *   rather than trust the event's own payload, since it carries no task
 *   data itself.
 * - On WS reconnect (Codex P2, PR #2685): if the mount-time query raced a
 *   disconnected/reconnecting socket and lost, the ONLY other trigger
 *   (`background-task-updated`) is explicitly non-persisted — an
 *   already-running task with no further state transition pending would
 *   never re-signal, leaving this pane blind to it for its entire mounted
 *   lifetime. `addWSReconnectHandler` is a singleton, app-lifetime
 *   registration (see ws.ts — no per-call unregister exists), so this
 *   module registers exactly ONE handler at module scope and fans it out
 *   to every currently-mounted instance's own `refresh` via a small
 *   Set — each hook instance adds/removes itself on mount/cleanup.
 *
 * Dispatches ONLY `RegistryAttachedTaskObserved`/`RegistryAttachedTaskCleared`
 * — never `AttachedTaskObserved`/`AttachedTaskCleared` directly. An
 * earlier version of this hook dispatched into the shared `attachedTask`
 * axis directly; `agent-view.tsx`'s own attached-task effect independently
 * recomputes that axis from the transcript alone and clears it the next
 * time it runs if the transcript disagrees — which, for a survived-restart
 * task with genuinely NO transcript record, is always. That silently
 * undid this hook's whole purpose (Codex P1, PR #2685). Dispatching into
 * `registryAttachedTaskSince` instead — a field only these two commands
 * ever touch — lets that effect read and COMBINE both signals instead of
 * one stomping the other; see its own updated doc comment.
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
import { addWSReconnectHandler } from "@/app/store/ws";
import type { AgentPaneModel } from "@/app/store/agent-pane-model";

export interface UseBackgroundTaskRegistryOptions {
    blockId: string;
    model: AgentPaneModel;
}

/** Pure: the `since` timestamp to observe from a set of registry rows, or
 * `null` if none are currently running. Exported for direct unit coverage
 * without a SolidJS reactive-root harness. */
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
        // Best-effort — the WS-reconnect trigger and the next
        // background-task-updated event both give this another chance;
        // nothing user-visible to report on a single failed query.
        return;
    }
    const since = resolveAttachedTaskObservation(tasks);
    opts.model.dispatchPane(
        since != null ? { type: "RegistryAttachedTaskObserved", at: since } : { type: "RegistryAttachedTaskCleared" },
    );
}

// Singleton reconnect fan-out — see the module doc comment for why this
// can't just be `addWSReconnectHandler(() => refresh(opts))` per instance.
const activeRefreshCallbacks = new Set<() => void>();
addWSReconnectHandler(() => {
    for (const cb of activeRefreshCallbacks) cb();
});

export function useBackgroundTaskRegistry(opts: UseBackgroundTaskRegistryOptions): void {
    const doRefresh = () => { void refresh(opts); };

    onMount(doRefresh);

    activeRefreshCallbacks.add(doRefresh);
    onCleanup(() => { activeRefreshCallbacks.delete(doRefresh); });

    const unsub = waveEventSubscribe({
        eventType: WpsEvent.BackgroundTaskUpdated,
        scope: `block:${opts.blockId}`,
        handler: doRefresh,
    });
    onCleanup(() => { try { unsub(); } catch { /* ignore */ } });
}
