// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Button } from "@/app/element/ui/Button";
import { For, type JSX } from "solid-js";
import { ActivityRow } from "./ActivityRow";
import type { ActivityDockState } from "./activity-dock-state";
import type { ActivityKind, PinnedActivity } from "../../../agent-model/activity-types";

const KIND_PLURAL: Record<ActivityKind, string> = { shell: "shell", cron: "cron", subagent: "subagent", tool: "task" };

function overflowSummary(items: PinnedActivity[]): string {
    const counts = new Map<ActivityKind, number>();
    for (const a of items) counts.set(a.kind, (counts.get(a.kind) ?? 0) + 1);
    return [...counts.entries()]
        .map(([k, n]) => `${n} ${KIND_PLURAL[k]}${n === 1 ? "" : "s"}`)
        .join(" · ");
}

/** The rows of the dock. */
export const DockRows = ({ s }: { s: ActivityDockState }): JSX.Element => {
    return (
        <For each={s.renderedIds()}>
            {(id) => (
                <ActivityRow
                    activity={() => s.activityById().get(id)}
                    expanded={() => s.expandedIds().has(id)}
                    leaving={() => s.isLeaving(id)}
                    onToggle={() => s.togglePin(id)}
                    onStop={() => s.stop(id)}
                    onDismiss={() => s.dismiss(id)}
                />
            )}
        </For>
    );
};

/** The button that opens or closes the rows past the cap. */
export const OverflowButton = ({ s }: { s: ActivityDockState }): JSX.Element => {
    return (
        <Button variant="row" density="compact" class="agent-activity-overflow" onClick={() => s.setOverflowOpen((v) => !v)}>
            {s.overflowOpen()
                ? "▾ fewer"
                : `▸ ${s.overflow().length} more (${overflowSummary(s.overflow())})`}
        </Button>
    );
};
