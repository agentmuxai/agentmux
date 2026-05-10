// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Agent-pane perf section for the diag panel — Phase 3 of the
 * virtualization redesign. Polls `agentPerfStore.snapshot()` at 1 Hz
 * and renders per-kind row mount times, estimator-miss rates, and
 * recent layout shifts.
 *
 * No-op in production: snapshot returns empty when probing is
 * disabled, so the section renders nothing.
 *
 * See docs/specs/SPEC_AGENT_PANE_VIRTUALIZATION_REDESIGN.md
 * §"Intelligent perf probing".
 */

import { createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import {
    agentPerfStore,
    ESTIMATOR_MISS_THRESHOLD,
    type AgentPerfSnapshot,
} from "@/view/agent/virtualization/perf-probe";

const POLL_INTERVAL_MS = 1000;

const EMPTY_SNAPSHOT: AgentPerfSnapshot = {
    rowMountByKind: new Map(),
    estimatorMissRateByKind: new Map(),
    recentEstimatorMisses: [],
    recentLayoutShifts: [],
};

function formatMs(n: number | undefined): string {
    if (n == null) return "—";
    if (n < 10) return n.toFixed(1);
    return n.toFixed(0);
}

function formatPct(n: number): string {
    return `${(n * 100).toFixed(0)}%`;
}

function ageMs(now: number, at: number): string {
    const dt = now - at;
    if (dt < 1000) return `${dt.toFixed(0)}ms`;
    if (dt < 60_000) return `${(dt / 1000).toFixed(1)}s`;
    return `${(dt / 60_000).toFixed(1)}min`;
}

export function AgentPanePerfSection(): JSX.Element {
    const [snapshot, setSnapshot] = createSignal<AgentPerfSnapshot>(EMPTY_SNAPSHOT);
    let pollHandle: number | undefined;

    const poll = (): void => {
        setSnapshot(agentPerfStore.snapshot());
    };

    onMount(() => {
        poll();
        pollHandle = window.setInterval(poll, POLL_INTERVAL_MS);
    });
    onCleanup(() => {
        if (pollHandle != null) window.clearInterval(pollHandle);
    });

    const hasData = (): boolean => {
        const s = snapshot();
        return s.rowMountByKind.size > 0
            || s.recentEstimatorMisses.length > 0
            || s.recentLayoutShifts.length > 0;
    };

    return (
        <Show when={hasData()}>
            <div
                style={{
                    "margin-top": "8px",
                    "padding-top": "8px",
                    "border-top": "1px solid #333",
                }}
            >
                <div style={{ "font-weight": "bold", color: "#7af", "margin-bottom": "4px" }}>
                    📊 Agent Pane Perf
                </div>

                {/* Per-kind row mount durations */}
                <Show when={snapshot().rowMountByKind.size > 0}>
                    <div style={{ "margin-bottom": "6px" }}>
                        <div style={{ color: "#aaa", "font-size": "10px", "margin-bottom": "2px" }}>
                            Row mount duration (last 64 per kind)
                        </div>
                        <table style={{ "font-size": "10px", "border-collapse": "collapse", width: "100%" }}>
                            <thead>
                                <tr style={{ color: "#888" }}>
                                    <th style={{ "text-align": "left", padding: "1px 4px" }}>kind</th>
                                    <th style={{ "text-align": "right", padding: "1px 4px" }}>p50</th>
                                    <th style={{ "text-align": "right", padding: "1px 4px" }}>p95</th>
                                    <th style={{ "text-align": "right", padding: "1px 4px" }}>max</th>
                                    <th style={{ "text-align": "right", padding: "1px 4px" }}>n</th>
                                    <th style={{ "text-align": "right", padding: "1px 4px" }}>est-miss</th>
                                </tr>
                            </thead>
                            <tbody>
                                <For each={[...snapshot().rowMountByKind.entries()].sort(
                                    ([, a], [, b]) => (b.p95 ?? 0) - (a.p95 ?? 0),
                                )}>
                                    {([kind, q]) => {
                                        const missRate = snapshot().estimatorMissRateByKind.get(kind) ?? 0;
                                        const flag = missRate > 0.20 ? " ⚠" : "";
                                        return (
                                            <tr>
                                                <td style={{ padding: "1px 4px" }}>{kind}</td>
                                                <td style={{ padding: "1px 4px", "text-align": "right" }}>
                                                    {formatMs(q.p50)}
                                                </td>
                                                <td style={{ padding: "1px 4px", "text-align": "right" }}>
                                                    {formatMs(q.p95)}
                                                </td>
                                                <td style={{ padding: "1px 4px", "text-align": "right" }}>
                                                    {formatMs(q.max)}
                                                </td>
                                                <td style={{ padding: "1px 4px", "text-align": "right", color: "#888" }}>
                                                    {q.count}
                                                </td>
                                                <td style={{
                                                    padding: "1px 4px",
                                                    "text-align": "right",
                                                    color: missRate > 0.20 ? "#fa6" : "#6c8",
                                                }}>
                                                    {formatPct(missRate)}{flag}
                                                </td>
                                            </tr>
                                        );
                                    }}
                                </For>
                            </tbody>
                        </table>
                        <div style={{ color: "#666", "font-size": "9px", "margin-top": "2px" }}>
                            est-miss = pct measured outside ±{formatPct(ESTIMATOR_MISS_THRESHOLD)} of estimate
                        </div>
                    </div>
                </Show>

                {/* Recent layout shifts */}
                <Show when={snapshot().recentLayoutShifts.length > 0}>
                    <div style={{ "margin-bottom": "6px" }}>
                        <div style={{ color: "#aaa", "font-size": "10px" }}>
                            Layout shifts in agent pane (last {snapshot().recentLayoutShifts.length})
                        </div>
                        <For each={snapshot().recentLayoutShifts.slice(0, 5)}>
                            {(s) => {
                                const now = performance.now();
                                return (
                                    <div style={{ "font-size": "10px", color: "#bbb", "padding-left": "8px" }}>
                                        {ageMs(now, s.timestamp)} ago: {s.value.toFixed(4)}
                                    </div>
                                );
                            }}
                        </For>
                    </div>
                </Show>

                {/* Recent estimator misses */}
                <Show when={snapshot().recentEstimatorMisses.length > 0}>
                    <div>
                        <div style={{ color: "#aaa", "font-size": "10px" }}>
                            Recent estimator misses (last {snapshot().recentEstimatorMisses.length})
                        </div>
                        <For each={snapshot().recentEstimatorMisses.slice(0, 5)}>
                            {(s) => {
                                const now = performance.now();
                                return (
                                    <div style={{ "font-size": "10px", color: "#bbb", "padding-left": "8px" }}>
                                        {ageMs(now, s.timestamp)} ago: {s.kind} —
                                        est {s.estimated.toFixed(0)}px, actual {s.actual.toFixed(0)}px
                                        ({formatPct(s.errorPct)})
                                    </div>
                                );
                            }}
                        </For>
                    </div>
                </Show>
            </div>
        </Show>
    );
}
