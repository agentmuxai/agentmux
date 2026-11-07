// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Agent-pane perf section for the diag panel — Phase 3 of the
 * virtualization redesign. Polls `agentPerfStore.snapshot()` at 1 Hz
 * and renders per-kind row mount times, estimator-miss rates, recent
 * layout shifts, and (task #40) layout/document store DISPATCH times —
 * the reducer/store cost underneath the DOM that the original probe
 * never covered.
 *
 * No-op in production: snapshot returns empty when probing is
 * disabled, so the section renders nothing.
 *
 * See docs/specs/SPEC_AGENT_PANE_VIRTUALIZATION_REDESIGN.md
 * §"Intelligent perf probing".
 */

import { For, Show, type Accessor, type JSX } from "solid-js";
import {
    ESTIMATOR_MISS_THRESHOLD,
    type AgentPerfSnapshot,
} from "@/app/agent-model/perf-probe";
import { createPerfSnapshot } from "./agent-pane-perf-section-state";

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

/** Per-kind row mount durations, slowest p95 first, with each kind's estimator-miss rate. */
function RowMountTable(props: { snap: Accessor<AgentPerfSnapshot> }): JSX.Element {
    const snap = props.snap;
    return (
            <Show when={snap().rowMountByKind.size > 0}>
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
                            <For each={[...snap().rowMountByKind.entries()].sort(
                                ([, a], [, b]) => (b.p95 ?? 0) - (a.p95 ?? 0),
                            )}>
                                {([kind, q]) => {
                                    const missRate = snap().estimatorMissRateByKind.get(kind) ?? 0;
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
    );
}

/** Store dispatch durations (task #40) — the reducer/store cost underneath the DOM, not the row-mount cost above. "layout" is agent-pane-layout-store.ts (row positions/window), "document" is agent-document-store.ts (the message list). */
function DispatchTable(props: { snap: Accessor<AgentPerfSnapshot> }): JSX.Element {
    const snap = props.snap;
    return (
            <Show when={snap().dispatchByKind.size > 0}>
                <div style={{ "margin-bottom": "6px" }}>
                    <div style={{ color: "#aaa", "font-size": "10px", "margin-bottom": "2px" }}>
                        Store dispatch duration (last 64 per kind)
                    </div>
                    <table style={{ "font-size": "10px", "border-collapse": "collapse", width: "100%" }}>
                        <thead>
                            <tr style={{ color: "#888" }}>
                                <th style={{ "text-align": "left", padding: "1px 4px" }}>kind</th>
                                <th style={{ "text-align": "right", padding: "1px 4px" }}>p50</th>
                                <th style={{ "text-align": "right", padding: "1px 4px" }}>max</th>
                                <th style={{ "text-align": "right", padding: "1px 4px" }}>n</th>
                            </tr>
                        </thead>
                        <tbody>
                            <For each={[...snap().dispatchByKind.entries()].sort(
                                ([, a], [, b]) => (b.max ?? 0) - (a.max ?? 0),
                            )}>
                                {([kind, q]) => (
                                    <tr>
                                        <td style={{ padding: "1px 4px" }}>{kind}</td>
                                        <td style={{ padding: "1px 4px", "text-align": "right" }}>
                                            {formatMs(q.p50)}
                                        </td>
                                        <td style={{ padding: "1px 4px", "text-align": "right" }}>
                                            {formatMs(q.max)}
                                        </td>
                                        <td style={{ padding: "1px 4px", "text-align": "right", color: "#888" }}>
                                            {q.count}
                                        </td>
                                    </tr>
                                )}
                            </For>
                        </tbody>
                    </table>
                </div>
            </Show>
    );
}

/** Transcript cursors (Phase 5a-4, transcript-cursor.ts): how each pane's live records were placed. Anything but delivered/duplicates/echoes is worth a look. */
function CursorList(props: { snap: Accessor<AgentPerfSnapshot> }): JSX.Element {
    const snap = props.snap;
    return (
            <Show when={snap().transcriptCursors.length > 0}>
                <div style={{ "margin-bottom": "6px" }}>
                    <div style={{ color: "#aaa", "font-size": "10px", "margin-bottom": "2px" }}>
                        Transcript cursors
                    </div>
                    <For each={snap().transcriptCursors}>
                        {({ blockId, stats: c }) => (
                            <div style={{ "font-size": "10px", color: "#bbb", "padding-left": "8px" }}>
                                {blockId.slice(0, 8)}: {c.delivered} delivered, {c.duplicates} dup, {c.echoes} echo,
                                gaps {c.gapsFilled} ({c.gapLinesFilled} lines)
                                <span style={{ color: c.linesSkipped + c.unpositioned + c.genChanges > 0 ? "#fa6" : "#6c8" }}>
                                    {" "}· skipped {c.linesSkipped}, unpositioned {c.unpositioned}, gen changes {c.genChanges}, own echoes dropped {c.ownEchoesDropped}
                                </span>
                            </div>
                        )}
                    </For>
                </div>
            </Show>
    );
}

/** The newest layout shifts in the agent pane. */
function LayoutShifts(props: { snap: Accessor<AgentPerfSnapshot> }): JSX.Element {
    const snap = props.snap;
    return (
            <Show when={snap().recentLayoutShifts.length > 0}>
                <div style={{ "margin-bottom": "6px" }}>
                    <div style={{ color: "#aaa", "font-size": "10px" }}>
                        Layout shifts in agent pane (last {snap().recentLayoutShifts.length})
                    </div>
                    <For each={snap().recentLayoutShifts.slice(0, 5)}>
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
    );
}

/** The newest estimator misses. */
function EstimatorMisses(props: { snap: Accessor<AgentPerfSnapshot> }): JSX.Element {
    const snap = props.snap;
    return (
            <Show when={snap().recentEstimatorMisses.length > 0}>
                <div>
                    <div style={{ color: "#aaa", "font-size": "10px" }}>
                        Recent estimator misses (last {snap().recentEstimatorMisses.length})
                    </div>
                    <For each={snap().recentEstimatorMisses.slice(0, 5)}>
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
    );
}

export function AgentPanePerfSection(): JSX.Element {
    const { snapshot, hasData } = createPerfSnapshot();

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

                <RowMountTable snap={snapshot} />
                <DispatchTable snap={snapshot} />
                <CursorList snap={snapshot} />
                <LayoutShifts snap={snapshot} />
                <EstimatorMisses snap={snapshot} />
            </div>
        </Show>
    );
}
