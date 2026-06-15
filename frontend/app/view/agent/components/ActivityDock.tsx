// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ActivityDock — a strip pinned to the top of the agent pane listing every
 * long-running activity (Phase 1: shells) as a uniform row. Click a row → its
 * live view expands; click again → collapses. The conversation scrolls under it.
 *
 * Pure derived view of the agent-document store — no new state. Expand state is
 * the shared `documentState.pinnedNodes`, so the dock and the inline
 * PersistentShellBlock stay in sync.
 *
 * Spec: docs/specs/SPEC_LONG_RUNNING_SHELL_PINNED_DOCK_2026_06_15.md
 *   (D1 dock vs swarm · D3 ordering · D4 retention · D6 cap/overflow)
 */

import { For, Show, createEffect, createMemo, createSignal, onCleanup, type JSX } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { ActivityRow } from "./ActivityRow";
import { shellActivities } from "../activity/shell-adapter";
import { RETENTION_MS, type ActivityKind, type ActivityStatus, type PinnedActivity } from "../activity/types";
import type { SignalPair } from "../state";
import type { DocumentNode, DocumentState } from "../types";

const MAX_INLINE = 3;

// D3 — running first, then error, stopped, done.
const STATUS_RANK: Record<ActivityStatus, number> = { running: 0, error: 1, stopped: 2, done: 3 };

const KIND_PLURAL: Record<ActivityKind, string> = { shell: "shell", cron: "cron", subagent: "subagent" };

function overflowSummary(items: PinnedActivity[]): string {
    const counts = new Map<ActivityKind, number>();
    for (const a of items) counts.set(a.kind, (counts.get(a.kind) ?? 0) + 1);
    return [...counts.entries()]
        .map(([k, n]) => `${n} ${KIND_PLURAL[k]}${n === 1 ? "" : "s"}`)
        .join(" · ");
}

interface ActivityDockProps {
    documentAtom: SignalPair<DocumentNode[]>;
    documentStateAtom: SignalPair<DocumentState>;
}

export const ActivityDock = (props: ActivityDockProps): JSX.Element => {
    const [nodes] = props.documentAtom;
    const [docState, setDocState] = props.documentStateAtom;
    const [now, setNow] = createSignal(Date.now());
    const [dismissed, setDismissed] = createSignal<Set<string>>(new Set());
    const [overflowOpen, setOverflowOpen] = createSignal(false);

    const allActivities = createMemo(() => shellActivities(nodes()));

    const activityById = createMemo(() => {
        const m = new Map<string, PinnedActivity>();
        for (const a of allActivities()) m.set(a.id, a);
        return m;
    });

    // Tick once a second only while a terminal row is STILL within its retention
    // window. The `now() - endedAt < retention` check is what lets this go false:
    // ShellNodes linger in the document forever, so without it the guard stayed
    // true permanently and the 1Hz setInterval never cleared — a perpetual
    // re-render loop per pane after the first shell exited (reagent #1428 P2).
    // Depending on now() means each tick re-evaluates; once every lingering row
    // has aged past its retention the effect re-runs and clears the interval.
    const hasExpiring = createMemo(() => {
        const t = now();
        return allActivities().some(
            (a) =>
                a.status !== "running" &&
                RETENTION_MS[a.status] !== Infinity &&
                a.endedAt != null &&
                t - a.endedAt < RETENTION_MS[a.status],
        );
    });
    createEffect(() => {
        if (!hasExpiring()) return;
        const id = setInterval(() => setNow(Date.now()), 1000);
        onCleanup(() => clearInterval(id));
    });

    // D4 — running always; terminal within its retention window; never dismissed.
    const visible = createMemo(() => {
        const dm = dismissed();
        const t = now();
        return allActivities().filter((a) => {
            if (dm.has(a.id)) return false;
            if (a.status === "running") return true;
            const ret = RETENTION_MS[a.status];
            if (ret === Infinity) return true; // error → until acknowledged
            return a.endedAt != null && t - a.endedAt < ret;
        });
    });

    // D3 — running-first, expanded-first, newest-first.
    const ordered = createMemo(() => {
        const pinned = docState().pinnedNodes;
        return [...visible()].sort((x, y) => {
            const rank = STATUS_RANK[x.status] - STATUS_RANK[y.status];
            if (rank !== 0) return rank;
            const exp = (pinned.has(x.id) ? 0 : 1) - (pinned.has(y.id) ? 0 : 1);
            if (exp !== 0) return exp;
            return y.startedAt - x.startedAt;
        });
    });

    // D6 — up to MAX_INLINE inline; expanded rows past the cap stay visible.
    const inline = createMemo(() => {
        const all = ordered();
        const pinned = docState().pinnedNodes;
        const head = all.slice(0, MAX_INLINE);
        const keptTail = all.slice(MAX_INLINE).filter((a) => pinned.has(a.id));
        return [...head, ...keptTail];
    });
    const overflow = createMemo(() => {
        const shown = new Set(inline().map((a) => a.id));
        return ordered().filter((a) => !shown.has(a.id));
    });

    // Rows currently rendered (inline + expanded overflow). Keyed by id string so
    // rows persist across chunk updates (the For sees equal strings).
    const renderedIds = createMemo(() => {
        const list = overflowOpen() ? [...inline(), ...overflow()] : inline();
        return list.map((a) => a.id);
    });

    const togglePin = (id: string): void =>
        setDocState((prev) => {
            const pinned = new Set(prev.pinnedNodes);
            if (pinned.has(id)) pinned.delete(id);
            else pinned.add(id);
            return { ...prev, pinnedNodes: pinned };
        });

    const stop = (id: string): void => {
        const a = activityById().get(id);
        if (a?.kind === "shell" || a?.kind === "cron") {
            RpcApi.ShellStopCommand(TabRpcClient, { shell_id: id }).catch(() => {
                // best-effort; the exit event reconciles status
            });
        }
    };

    const dismiss = (id: string): void =>
        setDismissed((prev) => {
            const next = new Set(prev);
            next.add(id);
            return next;
        });

    return (
        <Show when={ordered().length > 0}>
            <div class="agent-activity-dock">
                <For each={renderedIds()}>
                    {(id) => (
                        <ActivityRow
                            activity={() => activityById().get(id)}
                            expanded={() => docState().pinnedNodes.has(id)}
                            onToggle={() => togglePin(id)}
                            onStop={() => stop(id)}
                            onDismiss={() => dismiss(id)}
                        />
                    )}
                </For>
                <Show when={overflow().length > 0}>
                    <button
                        class="agent-activity-overflow"
                        onClick={() => setOverflowOpen((v) => !v)}
                    >
                        {overflowOpen()
                            ? "▾ fewer"
                            : `▸ ${overflow().length} more (${overflowSummary(overflow())})`}
                    </button>
                </Show>
            </div>
        </Show>
    );
};

ActivityDock.displayName = "ActivityDock";
