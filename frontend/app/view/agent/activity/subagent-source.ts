// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Shared, app-lifetime source of every active/recent subagent, for the
 * activity dock's subagent adapter. One module-level singleton instead of a
 * per-`ActivityDock`-instance fetch+subscribe: every agent pane's dock reads
 * the same signal, so opening N panes costs one `ListActive` poll cycle, not
 * N. Mirrors `SwarmViewModel`'s own `loadSubagents` + event wiring
 * (`swarm-model.ts`), just as a bare singleton rather than a per-pane
 * ViewModel — the dock only needs the list, not Swarm's tree/expand-state
 * machinery.
 *
 * Spec: docs/specs/SPEC_LONG_RUNNING_SHELL_PINNED_DOCK_2026_06_15.md §3, §7
 * (subagent adapter).
 */

import { callBackendService } from "@/app/store/wos";
import { waveEventSubscribe } from "@/app/store/wps";
import { createSignal, type Accessor } from "solid-js";
import { mergeSubagentsPreservingIdentity, type ActiveSubagent } from "../../swarm/swarm-model";

const [allSubagents, setAllSubagents] = createSignal<ActiveSubagent[]>([]);

async function refresh(): Promise<void> {
    try {
        const result = await callBackendService("subagent", "ListActive", []);
        const list = (result as ActiveSubagent[]) ?? [];
        setAllSubagents((prev) => mergeSubagentsPreservingIdentity(prev, list));
    } catch {
        // silently ignore — dock just shows no subagent rows this refresh
    }
}

// Started once at module load (ES modules are singletons — every importer
// shares this one subscription set), never torn down: the dock's subagent
// rows should reflect swarm-wide activity for the lifetime of the app, same
// as `providers/index.ts`'s `modelOverlay` singleton.
void refresh();
waveEventSubscribe({ eventType: "subagent:spawned", handler: () => void refresh() });
waveEventSubscribe({ eventType: "subagent:completed", handler: () => void refresh() });
// Without this, a subagent the backend reconciles from active to abandoned
// (parent turn already ended — see `reconcile_stale_subagents`, which runs
// on every pane reopen with a persisted session id, i.e. exactly the app-
// restart case) never refreshes here: the dock keeps showing the stale
// pre-restart snapshot ("running", with a frozen timestamp/event count)
// until some UNRELATED subagent happens to spawn/complete and trigger a
// refresh — which, for an otherwise-idle pane, may never happen. Mirrors
// `dispatch-source.ts`'s identical fix (reagent/codex, PR #2676) for the
// sibling dispatch-card singleton, which this module predates.
waveEventSubscribe({ eventType: "subagent:abandoned", handler: () => void refresh() });
waveEventSubscribe({
    eventType: "subagent:named",
    handler: (event: WaveEvent) => {
        const data = event?.data as { agentId?: string; displayName?: string } | undefined;
        if (!data?.agentId || !data.displayName) return;
        setAllSubagents((prev) =>
            prev.map((s) => (s.agent_id === data.agentId ? { ...s, display_name: data.displayName! } : s))
        );
    },
});

/** Every subagent currently known (active or recently completed), across the
 *  whole app. Callers filter by `parent_block_id` for their own pane. */
export const allSubagentsAtom: Accessor<ActiveSubagent[]> = allSubagents;
