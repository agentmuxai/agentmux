// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ActivityDock — a strip pinned above the composer listing every long-running
 * activity (shells + subagents spawned by this pane) as a uniform row. Click
 * a row → its live view expands in the dock; click again → collapses.
 *
 * Expansion state is local to the dock (`expandedIds` signal). It is intentionally
 * NOT shared with `documentState.pinnedNodes` — that shared state caused the
 * inline PersistentShellBlock in the conversation to expand simultaneously,
 * showing a duplicate log at whatever scroll position the shell block occupied.
 * Clicking in the dock expands in-dock only; clicking the inline block expands
 * there only. No cross-talk.
 *
 * Spec: docs/specs/SPEC_LONG_RUNNING_SHELL_PINNED_DOCK_2026_06_15.md
 *   (D1 dock vs swarm · D3 ordering · D4 retention · D6 cap/overflow)
 */

import { For, Show, createEffect, createMemo, createSignal, onCleanup, type JSX } from "solid-js";
import { useTick } from "@/app/hook/useTick";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { callBackendService } from "@/app/store/wos";
import { recordTurn } from "@/app/store/token-usage";
import { ActivityRow } from "./ActivityRow";
import { backgroundTaskActivities } from "../activity/background-adapter";
import { shellActivities } from "../activity/shell-adapter";
import { subagentActivities } from "../activity/subagent-adapter";
import { allSubagentsAtom } from "../activity/subagent-source";
import { nextToolPromotionAt, toolActivities } from "../activity/tool-adapter";
import { EXIT_FLASH_MS, RETENTION_MS, type ActivityKind, type ActivityStatus, type PinnedActivity } from "../activity/types";
import type { SignalPair } from "../state";
import type { DocumentNode } from "../types";

const MAX_INLINE = 3;

// D3 — running first, then error, stopped, done.
const STATUS_RANK: Record<ActivityStatus, number> = { running: 0, error: 1, stopped: 2, done: 3 };

const KIND_PLURAL: Record<ActivityKind, string> = { shell: "shell", cron: "cron", subagent: "subagent", tool: "task" };

function overflowSummary(items: PinnedActivity[]): string {
    const counts = new Map<ActivityKind, number>();
    for (const a of items) counts.set(a.kind, (counts.get(a.kind) ?? 0) + 1);
    return [...counts.entries()]
        .map(([k, n]) => `${n} ${KIND_PLURAL[k]}${n === 1 ? "" : "s"}`)
        .join(" · ");
}

interface ActivityDockProps {
    documentAtom: SignalPair<DocumentNode[]>;
    /** This pane's own block id — scopes the subagent adapter to subagents
     *  spawned by THIS agent (D5: the dock is block-scoped), same as shells
     *  are already scoped by living in this pane's own document. */
    blockId: string;
    /** Raw `db_background_tasks` rows for this block, from
     *  `useBackgroundTaskRegistry`'s returned accessor — see
     *  `activity/background-adapter.ts` for why these need reconciling
     *  against the transcript-derived rows below rather than just appended. */
    backgroundTasksAtom: () => BackgroundTaskView[];
}

export const ActivityDock = (props: ActivityDockProps): JSX.Element => {
    const [nodes] = props.documentAtom;
    const tick = useTick(1000);
    const [dismissed, setDismissed] = createSignal<Set<string>>(new Set());
    const [overflowOpen, setOverflowOpen] = createSignal(false);
    // Local expansion state — decoupled from documentState.pinnedNodes so that
    // expanding in the dock does not also expand the inline PersistentShellBlock.
    const [expandedIds, setExpandedIds] = createSignal<Set<string>>(new Set());

    // Tool-call promotion crosses its duration threshold on a plain wall-clock
    // timer, not a document event — nodes() alone won't re-run allActivities
    // at the moment a still-running Bash call turns 30s old. Same one-shot
    // discipline as hasExpiring below: compute the next promotion instant,
    // schedule exactly one setTimeout for it, and bump a nonce allActivities
    // subscribes to so it recomputes right then (and only then), instead of a
    // continuous tick.
    const [toolPromotionNonce, setToolPromotionNonce] = createSignal(0);
    createEffect(() => {
        // Read (subscribe to) the nonce itself so that when the scheduled
        // timer below fires and bumps it, this effect re-runs and schedules
        // the *next*-earliest pending promotion — without this, two running
        // Bash calls at different promotion instants would only ever
        // promote the earlier one (the effect ran once, scheduled once, and
        // nothing re-triggered it once that single timer fired).
        toolPromotionNonce();
        const at = nextToolPromotionAt(nodes(), Date.now());
        if (at == null) return;
        const timer = setTimeout(() => setToolPromotionNonce((n) => n + 1), Math.max(0, at - Date.now()) + 50);
        onCleanup(() => clearTimeout(timer));
    });

    const allActivities = createMemo(() => {
        toolPromotionNonce();
        const transcriptDerived = [
            ...shellActivities(nodes()),
            ...subagentActivities(allSubagentsAtom(), props.blockId),
            ...toolActivities(nodes(), Date.now()),
        ];
        // Registry rows fill the gap left by a session restart (no
        // transcript history exists for a task that survived one) —
        // filtered against ids the transcript already produced so a task
        // still visible in THIS session's transcript isn't rendered twice.
        // See activity/background-adapter.ts's doc comment.
        const knownIds = new Set(transcriptDerived.map((a) => a.id));
        return [...transcriptDerived, ...backgroundTaskActivities(props.backgroundTasksAtom(), knownIds)];
    });

    const activityById = createMemo(() => {
        const m = new Map<string, PinnedActivity>();
        for (const a of allActivities()) m.set(a.id, a);
        return m;
    });

    // Gate: true while at least one terminal row is still within its retention
    // window. Managed by createEffect + setTimeout — the effect subscribes only
    // to allActivities() (no tick dependency), computes the latest expiry, and
    // schedules a timer to flip the gate off. When activities change the effect
    // re-runs, cancels the stale timer via onCleanup, and reschedules.
    //
    // A tick-dependent memo for this gate (the previous `hasCandidates`) cannot
    // work: once any shell exits, exited ShellNodes linger in allActivities()
    // forever (shell-adapter returns every node), so `status !== "running" &&
    // endedAt != null` stays true permanently and the memo never resets —
    // reintroducing the per-second recompute reagent #1428 was added to prevent.
    const [hasExpiring, setHasExpiring] = createSignal(false);
    createEffect(() => {
        const activities = allActivities();
        let maxExpiry = 0;
        for (const a of activities) {
            if (a.status !== "running" && RETENTION_MS[a.status] !== Infinity && a.endedAt != null) {
                // + EXIT_FLASH_MS: the row stays in `visible` a bit past its nominal
                // retention so the departure flash (below) has time to play.
                maxExpiry = Math.max(maxExpiry, a.endedAt + RETENTION_MS[a.status] + EXIT_FLASH_MS);
            }
        }
        const remaining = maxExpiry - Date.now();
        if (remaining <= 0) {
            setHasExpiring(false);
            return;
        }
        setHasExpiring(true);
        const timer = setTimeout(() => setHasExpiring(false), remaining + 100);
        onCleanup(() => clearTimeout(timer));
    });

    // D4 — running always; terminal within its retention window (+ a grace
    // period so the departure flash below can play) — never dismissed.
    const visible = createMemo(() => {
        const dm = dismissed();
        const t = hasExpiring() ? (tick(), Date.now()) : Date.now();
        return allActivities().filter((a) => {
            if (dm.has(a.id)) return false;
            if (a.status === "running") return true;
            const ret = RETENTION_MS[a.status];
            if (ret === Infinity) return true;
            return a.endedAt != null && t - a.endedAt < ret + EXIT_FLASH_MS;
        });
    });

    // True once a row has passed its nominal retention window and is only
    // still mounted for the departure flash — ActivityRow uses this to play
    // the flash and stays out of the way of D4's actual removal logic above.
    const isLeaving = (id: string): boolean => {
        const a = activityById().get(id);
        if (!a || a.status === "running" || a.endedAt == null) return false;
        const ret = RETENTION_MS[a.status];
        if (ret === Infinity) return false;
        return (hasExpiring() ? (tick(), Date.now()) : Date.now()) - a.endedAt >= ret;
    };

    // D3 — running-first, expanded-first, newest-first.
    const ordered = createMemo(() => {
        const expanded = expandedIds();
        return [...visible()].sort((x, y) => {
            const rank = STATUS_RANK[x.status] - STATUS_RANK[y.status];
            if (rank !== 0) return rank;
            const exp = (expanded.has(x.id) ? 0 : 1) - (expanded.has(y.id) ? 0 : 1);
            if (exp !== 0) return exp;
            return y.startedAt - x.startedAt;
        });
    });

    // D6 — up to MAX_INLINE inline; expanded rows past the cap stay visible.
    const inline = createMemo(() => {
        const all = ordered();
        const expanded = expandedIds();
        const head = all.slice(0, MAX_INLINE);
        const keptTail = all.slice(MAX_INLINE).filter((a) => expanded.has(a.id));
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

    const togglePin = (id: string): void => {
        const wasExpanded = expandedIds().has(id);
        setExpandedIds((prev) => {
            const next = new Set(prev);
            if (next.has(id)) next.delete(id);
            else next.add(id);
            return next;
        });
        // First-expand-ever name generation — mirrors SubagentRow.handleToggle
        // (swarm-view.tsx). Without this, a subagent expanded only via the
        // dock (never via the Swarm pane) would permanently show its raw
        // slug/agent_id: nothing else triggers GenerateName (reagent P2, PR
        // #2062).
        if (!wasExpanded) {
            const a = activityById().get(id);
            if (a?.kind === "subagent" && a.subagent && !a.subagent.display_name) {
                void callBackendService("subagent", "GenerateName", [id]).then((result: any) => {
                    if (result?.tokens) recordTurn("ambient:subagent_name", result.tokens);
                });
            }
            // Same, for every unnamed member of a workflow/name group. The
            // dock's roster (ActivityRow.tsx) renders all members flat, with
            // no per-member toggle to hang this off of the way the Swarm
            // pane's per-member SubagentRow.handleToggle does — so the
            // group row's own first expand is the only hook available;
            // fire it for every unnamed member at once instead of one.
            if (a?.kind === "subagent" && a.subagentGroup) {
                for (const member of a.subagentGroup.members) {
                    if (member.display_name) continue;
                    void callBackendService("subagent", "GenerateName", [member.agent_id]).then((result: any) => {
                        if (result?.tokens) recordTurn("ambient:subagent_name", result.tokens);
                    });
                }
            }
        }
    };

    const stop = (id: string): void => {
        const a = activityById().get(id);
        if (a?.kind === "shell" || a?.kind === "cron") {
            RpcApi.ShellStopCommand(TabRpcClient, { shell_id: id }).catch(() => {
                // best-effort; the exit event reconciles status
            });
        }
    };

    const dismiss = (id: string): void => {
        setDismissed((prev) => {
            const next = new Set(prev);
            next.add(id);
            return next;
        });
    };

    return (
        <Show when={ordered().length > 0}>
            <div class="agent-activity-dock">
                <For each={renderedIds()}>
                    {(id) => (
                        <ActivityRow
                            activity={() => activityById().get(id)}
                            expanded={() => expandedIds().has(id)}
                            leaving={() => isLeaving(id)}
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
