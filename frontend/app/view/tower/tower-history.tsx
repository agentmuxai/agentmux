// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The Agents view's history: the selected entry's CPU and memory over the last
// 10 minutes, as two small charts (two measures, two scales: never one chart
// with two y-axes), each a line in the agent's color over a faint area, with a
// crosshair and tooltip under the pointer
// (SPEC_TOWER_AGENT_CENTRIC_VIEWS_2026_10_08.md §3.1, §5.5). Kept in the pane
// only; reopening starts empty.

import { createMemo, createSignal, For, type JSX, Show } from "solid-js";
import type { HistoryPoint } from "./tower-model";
import { formatAgo, formatMem } from "./tower-util";

/** The window the charts show, ending at the latest sample. */
export const WINDOW_MS = 10 * 60 * 1000;
const W = 600;
const H = 48;

interface Series {
    label: string;
    /** The value plotted, in the chart's own unit; absent where not known. */
    value: (p: HistoryPoint) => number | undefined;
    format: (v: number | undefined) => string;
    /** The top of the scale: at least this, so a quiet entry stays flat
     *  rather than magnifying noise. */
    floor: number;
}

export function HistoryCharts(props: {
    points: readonly HistoryPoint[];
    color?: string;
    /** CPU as the pane shows it (share of the machine or of a core). */
    cpuPercent: (fraction: number) => number;
    formatCpu: (fraction: number | undefined) => string;
}): JSX.Element {
    const series = (): Series[] => [
        {
            label: "CPU",
            value: (p) => (p.cpu == null ? undefined : props.cpuPercent(p.cpu)),
            // Back from percent to the pane's own CPU format.
            format: (v) => (v == null ? "—" : props.formatCpu(v / props.cpuPercent(1))),
            floor: 5,
        },
        { label: "Memory", value: (p) => p.mem, format: (v) => formatMem(v), floor: 64 * 1024 * 1024 },
    ];
    return (
        <Show when={props.points.length >= 2}>
            <div class="tower-history" data-testid="tower-history">
                <For each={series()}>{(s) => <Chart points={props.points} series={s} color={props.color} />}</For>
            </div>
        </Show>
    );
}

function Chart(props: { points: readonly HistoryPoint[]; series: Series; color?: string }) {
    const end = () => props.points[props.points.length - 1]?.t ?? 0;
    const known = createMemo(() =>
        props.points
            .map((p) => ({ t: p.t, v: props.series.value(p) }))
            .filter((p): p is { t: number; v: number } => p.v != null && p.t >= end() - WINDOW_MS)
    );
    const top = createMemo(() => Math.max(props.series.floor, ...known().map((p) => p.v)) * 1.1);
    const x = (t: number) => ((t - (end() - WINDOW_MS)) / WINDOW_MS) * W;
    const y = (v: number) => H - (v / top()) * (H - 2);
    const line = () =>
        known()
            .map((p, i) => `${i ? "L" : "M"}${x(p.t).toFixed(1)},${y(p.v).toFixed(1)}`)
            .join("");
    const area = () => {
        const pts = known();
        return pts.length
            ? `${line()}L${x(pts[pts.length - 1].t).toFixed(1)},${H}L${x(pts[0].t).toFixed(1)},${H}Z`
            : "";
    };
    const latest = () => known()[known().length - 1]?.v;
    const peak = () => (known().length ? Math.max(...known().map((p) => p.v)) : undefined);
    const [hover, setHover] = createSignal<{ t: number; v: number } | null>(null);
    const onMove = (e: PointerEvent) => {
        const box = (e.currentTarget as HTMLElement).getBoundingClientRect();
        const t = end() - WINDOW_MS + ((e.clientX - box.left) / Math.max(1, box.width)) * WINDOW_MS;
        let best: { t: number; v: number } | null = null;
        for (const p of known()) if (!best || Math.abs(p.t - t) < Math.abs(best.t - t)) best = p;
        setHover(best);
    };
    return (
        <div class="tower-chart">
            <div class="tower-chart-head">
                <span class="tower-chart-label">{props.series.label}</span>
                <span class="tower-num">{props.series.format(latest())}</span>
                <span class="tower-muted">peak {props.series.format(peak())}</span>
            </div>
            <div
                class="tower-chart-plot"
                role="img"
                aria-label={`${props.series.label} over the last 10 minutes: now ${props.series.format(latest())}, peak ${props.series.format(peak())}`}
                onPointerMove={onMove}
                onPointerLeave={() => setHover(null)}
            >
                <svg viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" aria-hidden="true">
                    <path class="tower-chart-area" d={area()} style={{ fill: props.color ?? "currentColor" }} />
                    <path
                        class="tower-chart-line"
                        d={line()}
                        style={{ stroke: props.color ?? "currentColor" }}
                        vector-effect="non-scaling-stroke"
                    />
                    <Show when={hover()}>
                        {(h) => (
                            <line
                                class="tower-chart-crosshair"
                                x1={x(h().t)}
                                x2={x(h().t)}
                                y1={0}
                                y2={H}
                                vector-effect="non-scaling-stroke"
                            />
                        )}
                    </Show>
                </svg>
                <Show when={hover()}>
                    {(h) => (
                        <div
                            class="tower-chart-tip"
                            data-testid="tower-chart-tip"
                            style={{ left: `${(x(h().t) / W) * 100}%` }}
                        >
                            {formatAgo(h().t, end())} · {props.series.format(h().v)}
                        </div>
                    )}
                </Show>
            </div>
            <div class="tower-chart-axis tower-muted">
                <span>10 min ago</span>
                <span>now</span>
            </div>
        </div>
    );
}
