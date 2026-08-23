// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Shared, app-lifetime source of every tracked dispatch (Solo Agent/Task
 * calls and Workflow runs), for the main transcript's dispatch-card
 * rendering. One module-level singleton instead of a per-pane fetch+
 * subscribe — mirrors `subagent-source.ts`'s `allSubagentsAtom` exactly,
 * just backed by `ListDispatches` instead of `ListActive`.
 *
 * Unlike `subagent-source.ts`, this also subscribes to `dispatch:updated`
 * (Workflow-kind `member_count`/`status`/`dispatch_name` changes) and, as of
 * reagent/codex's PR #2676 review, `subagent:abandoned` — without the
 * latter, a solo dispatch whose subagent reconciles from active to
 * abandoned (parent turn ended, see `reconcile_stale_subagents`) never
 * refreshes here, leaving its card/pill stuck on "running" until an
 * unrelated dispatch happens to fire a refresh.
 */

import { callBackendService } from "@/app/store/wos";
import { waveEventSubscribe } from "@/app/store/wps";
import { createSignal, type Accessor } from "solid-js";
import type { AgentDispatch } from "../../swarm/swarm-model";
import { createDebouncedRefresh } from "./debounced-refresh";

/** The backend's own quiet window (`refresh_dispatch_status`,
 *  `subagent_watcher/jsonl.rs`) — how long a dispatch must go without a new
 *  event before a counts-complete Running dispatch lazily flips to
 *  Completed. Kept in sync with that constant by name, not by import (no
 *  shared frontend/backend constants module exists for this). */
const DISPATCH_QUIET_WINDOW_MS = 60_000;

/**
 * Milliseconds until the earliest pending quiet-window transition among
 * `dispatches`, or `null` if none are waiting on one. The backend only
 * flips a counts-complete dispatch from `"running"` to `"completed"`
 * lazily — on the NEXT event or `ListDispatches` read, once quiet for
 * `DISPATCH_QUIET_WINDOW_MS`. No WS event fires when that window elapses on
 * its own, so a Workflow (or solo) dispatch with no further activity after
 * its last member finishes would read "running" indefinitely without a
 * scheduled follow-up poll (reagent/codex P1 on PR #2676). Exported as a
 * pure function for direct unit-testability.
 */
export function msUntilNextQuietWindowRefresh(dispatches: readonly AgentDispatch[], now: number): number | null {
    const pending = dispatches.filter(
        (d) => d.status === "running" && d.member_count > 0 && d.members_done >= d.member_count
    );
    if (pending.length === 0) return null;
    const earliestDeadline = Math.min(...pending.map((d) => d.last_event_at + DISPATCH_QUIET_WINDOW_MS));
    return Math.max(0, earliestDeadline - now);
}

const [allDispatches, setAllDispatches] = createSignal<AgentDispatch[]>([]);

let pendingQuietRefresh: ReturnType<typeof setTimeout> | undefined;

/** Schedule exactly one follow-up refresh at the earliest pending quiet-
 *  window deadline (replacing any previously-scheduled one) — never more
 *  than one in flight, since every refresh (including this one) re-derives
 *  the next deadline from the latest data. A small buffer past the exact
 *  deadline avoids racing the backend's own boundary check. */
function scheduleQuietWindowRefresh(list: readonly AgentDispatch[]): void {
    if (pendingQuietRefresh !== undefined) {
        clearTimeout(pendingQuietRefresh);
        pendingQuietRefresh = undefined;
    }
    const delay = msUntilNextQuietWindowRefresh(list, Date.now());
    if (delay === null) return;
    pendingQuietRefresh = setTimeout(() => {
        pendingQuietRefresh = undefined;
        void refresh();
    }, delay + 250);
}

async function refresh(): Promise<void> {
    try {
        const result = await callBackendService("subagent", "ListDispatches", []);
        const list = (result as AgentDispatch[]) ?? [];
        setAllDispatches(list);
        scheduleQuietWindowRefresh(list);
    } catch {
        // silently ignore — panes just keep rendering CompactResult fallback this refresh
    }
}

// Coalesces a burst of event-triggered refreshes into a single call — see
// docs/specs/SPEC_ACTIVITY_DOCK_REFRESH_COALESCING_2026_08_23.md. Without
// this, the up-to-200-event subagent backfill replay on pane reopen fired
// one uncoalesced ListDispatches call per event
// (docs/reports/REPORT_AGENT_PANE_REOPEN_SUBAGENT_STORM_2026_08_23.md).
const scheduleRefresh = createDebouncedRefresh(() => void refresh(), 100, 1000);

// Started once at module load (ES modules are singletons — every importer
// shares this one subscription set), never torn down — mirrors
// `subagent-source.ts`'s own lifecycle rationale. The module-load-time
// refresh itself stays immediate (undebounced) — see that module's
// identical comment for why.
void refresh();
waveEventSubscribe({ eventType: "subagent:spawned", handler: () => scheduleRefresh() });
waveEventSubscribe({ eventType: "subagent:completed", handler: () => scheduleRefresh() });
waveEventSubscribe({ eventType: "subagent:named", handler: () => scheduleRefresh() });
waveEventSubscribe({ eventType: "subagent:abandoned", handler: () => scheduleRefresh() });
waveEventSubscribe({ eventType: "dispatch:updated", handler: () => scheduleRefresh() });

/** Every tracked dispatch (Solo or Workflow) currently known, across the
 *  whole app. Callers filter by `parent_block_id` for their own pane. */
export const allDispatchesAtom: Accessor<AgentDispatch[]> = allDispatches;
