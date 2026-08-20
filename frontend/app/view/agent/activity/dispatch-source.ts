// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Shared, app-lifetime source of every tracked dispatch (Solo Agent/Task
 * calls and Workflow runs), for the main transcript's dispatch-card
 * rendering. One module-level singleton instead of a per-pane fetch+
 * subscribe — mirrors `subagent-source.ts`'s `allSubagentsAtom` exactly,
 * just backed by `ListDispatches` instead of `ListActive`.
 *
 * Unlike `subagent-source.ts`, this also subscribes to `dispatch:updated` —
 * the event that carries a Workflow-kind dispatch's `member_count`/`status`/
 * `dispatch_name` changes (see `swarm-model.ts`'s own listener for the same
 * event). Without it, a Workflow's card would never advance past its
 * initial member count.
 */

import { callBackendService } from "@/app/store/wos";
import { waveEventSubscribe } from "@/app/store/wps";
import { createSignal, type Accessor } from "solid-js";
import type { AgentDispatch } from "../../swarm/swarm-model";

const [allDispatches, setAllDispatches] = createSignal<AgentDispatch[]>([]);

async function refresh(): Promise<void> {
    try {
        const result = await callBackendService("subagent", "ListDispatches", []);
        setAllDispatches((result as AgentDispatch[]) ?? []);
    } catch {
        // silently ignore — panes just keep rendering CompactResult fallback this refresh
    }
}

// Started once at module load (ES modules are singletons — every importer
// shares this one subscription set), never torn down — mirrors
// `subagent-source.ts`'s own lifecycle rationale.
void refresh();
waveEventSubscribe({ eventType: "subagent:spawned", handler: () => void refresh() });
waveEventSubscribe({ eventType: "subagent:completed", handler: () => void refresh() });
waveEventSubscribe({ eventType: "subagent:named", handler: () => void refresh() });
waveEventSubscribe({ eventType: "dispatch:updated", handler: () => void refresh() });

/** Every tracked dispatch (Solo or Workflow) currently known, across the
 *  whole app. Callers filter by `parent_block_id` for their own pane. */
export const allDispatchesAtom: Accessor<AgentDispatch[]> = allDispatches;
