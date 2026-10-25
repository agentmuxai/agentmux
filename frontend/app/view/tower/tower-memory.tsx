// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The Memory view: AgentMux's total, and the four parts it is made of (Core,
// Panes, Agents, Not placed), each opening to what is in it
// (SPEC_TOWER_MEMORY_FIRST_AND_AGENTMUX_BREAKDOWN_2026_10_25.md §4).

import { IconButton, SegmentedControl } from "@/app/element/ui";
import type {
    TowerMemory,
    TowerMemoryGrowth,
    TowerMemoryHistory,
    TowerMemoryPart,
    TowerMemoryPartId,
    TowerMemoryRow,
    TowerMemorySeries,
} from "@/app/store/rpc-api";
import clsx from "clsx";
import { type Accessor, createEffect, createMemo, For, type JSX, Show } from "solid-js";
import { Chart } from "./tower-history";
import { KindIcon } from "./tower-kind-ui";
import type { HistoryPoint, TowerViewModel } from "./tower-model";
import { count, formatClock, formatMem, growthText } from "./tower-util";

/** What each part is called, and what is in it. */
const PARTS: Record<TowerMemoryPartId, { label: string; hint: string }> = {
    core: {
        label: "Core",
        hint: "AgentMux's own programs: the launcher, the server, the browser host and its GPU and service processes",
    },
    panes: {
        label: "Panes",
        hint: "What draws and hosts the panes: the window renderers, and each terminal with what runs in it",
    },
    agents: {
        label: "Agents",
        hint: "Each agent: its CLI, what runs for it (MCP servers, helpers), and what it started",
    },
    not_placed: {
        label: "Not placed",
        hint: "AgentMux started these and no pane claims them",
    },
};

/** A part's share of the whole, as a word: "44%", "<1%". */
export function formatShare(part: number, whole: number): string {
    if (whole <= 0 || part <= 0) return "0%";
    const pct = (part / whole) * 100;
    return pct < 1 ? "<1%" : `${Math.round(pct)}%`;
}

const partKey = (id: TowerMemoryPartId) => `memory:part:${id}`;
const rowKey = (key: string) => `memory:row:${key}`;
/** The ledger's key for a part. */
const partSeriesKey = (id: TowerMemoryPartId) => `part:${id}`;

/** The hour behind the sparklines: each key's series, on one time axis. */
interface Trend {
    byKey: ReadonlyMap<string, TowerMemorySeries>;
    /** The time of the newest point of any series, ms. */
    end: number;
    /** What the ledger keeps: its points times its step, ms. */
    spanMs: number;
}

/** What the rows share: the keys that grow unusually, and the hour. */
interface Shared {
    growth: Accessor<ReadonlyMap<string, TowerMemoryGrowth>>;
    trend: Accessor<Trend | null>;
}

function buildTrend(h: TowerMemoryHistory | null): Trend | null {
    if (!h || h.series.length === 0) return null;
    let end = 0;
    for (const s of h.series) end = Math.max(end, s.start_ms + (s.points.length - 1) * h.step_ms);
    return { byKey: new Map(h.series.map((s) => [s.key, s])), end, spanMs: 360 * h.step_ms };
}

/** A part's growth: its own, else its fastest-growing row's. */
function partGrowth(
    part: TowerMemoryPart,
    growth: ReadonlyMap<string, TowerMemoryGrowth>
): { g: TowerMemoryGrowth; row?: string } | undefined {
    const own = growth.get(partSeriesKey(part.part));
    let best: { g: TowerMemoryGrowth; row?: string } | undefined = own && { g: own };
    for (const r of part.rows) {
        const g = growth.get(r.key);
        if (g && (!best || g.to - g.from > best.g.to - best.g.from)) best = { g, row: r.label };
    }
    return best;
}

/** The last hour of one key as a line, on the shared time axis, scaled
 *  between its own lowest and highest as the rail's sparkline is. */
function Spark(props: { series: TowerMemorySeries | undefined; trend: Trend | null; step: number }) {
    const path = () => {
        const t = props.trend;
        const s = props.series;
        if (!t || !s || s.points.length < 2) return "";
        const lo = Math.min(...s.points);
        const hi = Math.max(...s.points);
        const span = Math.max(hi - lo, hi / 10, 8 * 1024 * 1024);
        const mid = (hi + lo) / 2;
        return s.points
            .map((v, i) => {
                const x = ((s.start_ms + i * props.step - (t.end - t.spanMs)) / t.spanMs) * 60;
                return `${Math.max(0, x).toFixed(1)},${(8 - ((v - mid) / span) * 14).toFixed(1)}`;
            })
            .join(" ");
    };
    return (
        <svg class="tower-spark tower-memory-spark" viewBox="0 0 60 16" preserveAspectRatio="none" aria-hidden="true">
            <polyline points={path()} fill="none" stroke="var(--tower-memory-color, currentColor)" stroke-width="1.5" />
        </svg>
    );
}

/** The amber note on a part or row that grows unusually. */
function Grow(props: { g: TowerMemoryGrowth; row?: string; testid: string }) {
    const text = () => `${props.row ? `${props.row} ` : ""}${growthText(props.g)}`;
    return (
        <span class="tower-memory-grow" data-testid={props.testid} title={text()}>
            {text()}
        </span>
    );
}

/** Open the agent a row stands for in the Agents view. */
function showInAgents(m: TowerViewModel, row: TowerMemoryRow, part: TowerMemoryPartId) {
    if (!row.task) return;
    m.select(part === "panes" ? "terminals" : row.task);
    m.setView("agents");
}

export function MemoryView(props: { model: TowerViewModel }): JSX.Element {
    const m = props.model;
    const memory = () => m.snapshot()?.memory;
    // The busiest part is open the first time, as the Type grouping's are.
    createEffect(() => {
        const v = memory();
        if (!v || m.memoryOpened) return;
        const busiest = [...v.parts].sort((a, b) => b.mem - a.mem)[0];
        if (!busiest || busiest.mem <= 0) return;
        m.memoryOpened = true;
        m.open([partKey(busiest.part)]);
    });
    const shared: Shared = {
        growth: createMemo(() => new Map((memory()?.growth ?? []).map((g) => [g.key, g]))),
        trend: createMemo(() => buildTrend(m.memoryHistory())),
    };
    return (
        <Show when={memory()} fallback={<div class="tower-empty">Measuring…</div>}>
            {(v) => (
                <div class="tower-memory" data-testid="tower-memory">
                    <PartsBar model={m} memory={v()} />
                    <PartsTable model={m} memory={v()} shared={shared} />
                    <MemoryChart model={m} memory={v()} trend={shared.trend()} />
                    <Footnote memory={v()} />
                </div>
            )}
        </Show>
    );
}

/** The line to chart: the one picked while it is still a part or row on
 *  screen and the ledger has its series, else the total. The ledger keeps a
 *  row that ended for an hour, but its line would be drawn ending at "now". */
export function chartKey(picked: string, memory: TowerMemory, trend: Trend | null): string {
    const current =
        picked === "total" ||
        memory.parts.some((p) => partSeriesKey(p.part) === picked || p.rows.some((r) => r.key === picked));
    return current && trend?.byKey.has(picked) ? picked : "total";
}

/** What a ledger key is called. */
export function seriesLabel(key: string, memory: TowerMemory): string {
    if (key === "total") return "AgentMux";
    for (const p of memory.parts) {
        if (partSeriesKey(p.part) === key) return PARTS[p.part].label;
        const row = p.rows.find((r) => r.key === key);
        if (row) return row.label;
    }
    return key;
}

/** The selected line's memory over the last 10 minutes or the hour
 *  (spec section 7.2); AgentMux's total until a part or row is chosen. */
function MemoryChart(props: { model: TowerViewModel; memory: TowerMemory; trend: Trend | null }): JSX.Element {
    const m = props.model;
    const key = () => chartKey(m.memoryChart(), props.memory, props.trend);
    const series = () => props.trend?.byKey.get(key());
    const step = () => m.memoryHistory()?.step_ms ?? 10_000;
    const points = createMemo<HistoryPoint[]>(() => {
        const s = series();
        return s ? s.points.map((v, i) => ({ t: s.start_ms + i * step(), mem: v })) : [];
    });
    const since = () => {
        const h = m.memoryHistory();
        return h && props.trend && props.trend.end - h.since_ms < 59 * 60_000 ? h.since_ms : null;
    };
    return (
        <Show when={props.trend}>
            <div class="tower-memory-chart" data-testid="tower-memory-chart">
                <div class="tower-memory-chart-head">
                    <span class="tower-label" data-testid="tower-memory-chart-label">
                        {seriesLabel(key(), props.memory)}
                    </span>
                    <span class="tower-toolbar-spacer" />
                    <Show when={since()}>{(t) => <span class="tower-muted">since {formatClock(t())}</span>}</Show>
                    <SegmentedControl<"10m" | "1h">
                        density="compact"
                        ariaLabel="Time span of the memory chart"
                        options={[
                            { value: "10m", label: "10 min" },
                            { value: "1h", label: "1 hour" },
                        ]}
                        value={m.memoryWindow()}
                        onChange={m.setMemoryWindow}
                    />
                </div>
                <Chart
                    points={points()}
                    windowMs={m.memoryWindow() === "10m" ? 10 * 60_000 : 60 * 60_000}
                    series={{
                        label: "Memory",
                        value: (p) => p.mem,
                        format: (v) => formatMem(v),
                        floor: 64 * 1024 * 1024,
                        color: "var(--sysinfo-mem-color)",
                    }}
                />
            </div>
        </Show>
    );
}

/** One bar split into the four parts: each a button that opens its part. */
function PartsBar(props: { model: TowerViewModel; memory: TowerMemory }): JSX.Element {
    const describe = (p: TowerMemoryPart) =>
        `${PARTS[p.part].label}: ${formatMem(p.mem)}, ${formatShare(p.mem, props.memory.total)}`;
    return (
        <div class="tower-memory-bar-wrap">
            <div
                class="tower-memory-bar"
                role="group"
                aria-label={`AgentMux's memory by part: ${props.memory.parts.map(describe).join("; ")}`}
            >
                <For each={props.memory.parts}>
                    {(p) => (
                        <Show when={p.mem > 0 && props.memory.total > 0}>
                            <div
                                role="button"
                                tabindex="0"
                                class={clsx("tower-memory-segment", `tower-memory-part--${p.part}`)}
                                style={{ width: `${(p.mem / props.memory.total) * 100}%` }}
                                title={`${describe(p)}. ${PARTS[p.part].hint}`}
                                aria-label={describe(p)}
                                data-testid={`tower-memory-segment-${p.part}`}
                                onClick={() => props.model.open([partKey(p.part)])}
                                onKeyDown={(e) => {
                                    if (e.key === "Enter" || e.key === " ") {
                                        e.preventDefault();
                                        props.model.open([partKey(p.part)]);
                                    }
                                }}
                            />
                        </Show>
                    )}
                </For>
            </div>
            <div class="tower-memory-legend">
                <For each={props.memory.parts}>
                    {(p) => (
                        <span class="tower-memory-key" title={PARTS[p.part].hint}>
                            <span
                                class={clsx("tower-memory-swatch", `tower-memory-part--${p.part}`)}
                                aria-hidden="true"
                            />
                            <span>{PARTS[p.part].label}</span>
                            <span class="tower-num tower-muted">{formatMem(p.mem)}</span>
                        </span>
                    )}
                </For>
            </div>
        </div>
    );
}

function PartsTable(props: { model: TowerViewModel; memory: TowerMemory; shared: Shared }): JSX.Element {
    return (
        <table class="tower-table tower-memory-table" aria-label="Memory by part">
            <thead>
                <tr>
                    <th>Part</th>
                    <th class="tower-num">Memory</th>
                    <th class="tower-num">Share</th>
                    <th class="tower-memory-trend-head">Last hour</th>
                </tr>
            </thead>
            <tbody>
                <For each={props.memory.parts}>
                    {(p) => <PartRows model={props.model} memory={props.memory} part={p} shared={props.shared} />}
                </For>
            </tbody>
        </table>
    );
}

function PartRows(props: {
    model: TowerViewModel;
    memory: TowerMemory;
    part: TowerMemoryPart;
    shared: Shared;
}): JSX.Element {
    const m = props.model;
    const key = () => partKey(props.part.part);
    const grows = () => partGrowth(props.part, props.shared.growth());
    const open = () => m.expanded().has(key());
    const total = () => props.memory.total;
    return (
        <>
            <tr
                class={clsx(
                    "tower-group-row tower-memory-part tower-memory-pick",
                    `tower-memory-part--${props.part.part}`,
                    m.memoryChart() === partSeriesKey(props.part.part) && "tower-memory-picked"
                )}
                data-testid={`tower-memory-part-${props.part.part}`}
                title={PARTS[props.part.part].hint}
                {...pick(m, partSeriesKey(props.part.part))}
            >
                <td class="tower-name">
                    <div class="tower-name-line">
                        <Show
                            when={props.part.rows.length > 0}
                            fallback={<span class="tower-memory-spacer" aria-hidden="true" />}
                        >
                            <IconButton
                                icon={open() ? "chevron-down" : "chevron-right"}
                                label={
                                    open()
                                        ? `Hide what's in ${PARTS[props.part.part].label}`
                                        : `Show what's in ${PARTS[props.part.part].label}`
                                }
                                density="compact"
                                tooltip={false}
                                aria-expanded={open()}
                                onClick={() => m.toggleExpanded(key())}
                            />
                        </Show>
                        <span class="tower-memory-swatch" aria-hidden="true" />
                        <span class="tower-label">{PARTS[props.part.part].label}</span>
                        <span class="tower-muted">({props.part.processes})</span>
                        <Show when={grows()}>
                            {(g) => (
                                <Grow g={g().g} row={g().row} testid={`tower-memory-grow-part-${props.part.part}`} />
                            )}
                        </Show>
                    </div>
                </td>
                <td class="tower-num">{formatMem(props.part.mem)}</td>
                <td class="tower-num tower-muted">{formatShare(props.part.mem, total())}</td>
                <TrendCell shared={props.shared} model={m} seriesKey={partSeriesKey(props.part.part)} />
            </tr>
            <Show when={open()}>
                <Show when={props.part.part === "not_placed" && props.part.rows.length > 0}>
                    <tr class="tower-memory-hint">
                        <td colspan={4} class="tower-muted">
                            {PARTS.not_placed.hint}.
                        </td>
                    </tr>
                </Show>
                <For each={props.part.rows}>
                    {(r) => (
                        <ItemRows
                            model={m}
                            memory={props.memory}
                            part={props.part.part}
                            row={r}
                            shared={props.shared}
                        />
                    )}
                </For>
            </Show>
        </>
    );
}

function ItemRows(props: {
    model: TowerViewModel;
    memory: TowerMemory;
    part: TowerMemoryPartId;
    row: TowerMemoryRow;
    shared: Shared;
}): JSX.Element {
    const m = props.model;
    const key = () => rowKey(props.row.key);
    const open = () => m.expanded().has(key());
    const total = () => props.memory.total;
    const goes = () => !!props.row.task && props.part !== "core";
    return (
        <>
            <tr
                class={clsx(
                    "tower-process-row tower-memory-row tower-memory-pick",
                    props.row.kind && `tower-kind--${props.row.kind}`,
                    m.memoryChart() === props.row.key && "tower-memory-picked"
                )}
                data-testid={`tower-memory-row-${props.row.key}`}
                {...pick(m, props.row.key)}
            >
                <td class="tower-name">
                    <div class="tower-name-line">
                        <Show
                            when={props.row.children.length > 0}
                            fallback={<span class="tower-memory-spacer" aria-hidden="true" />}
                        >
                            <IconButton
                                icon={open() ? "chevron-down" : "chevron-right"}
                                label={open() ? `Hide ${props.row.label}'s parts` : `Show ${props.row.label}'s parts`}
                                density="compact"
                                tooltip={false}
                                aria-expanded={open()}
                                onClick={() => m.toggleExpanded(key())}
                            />
                        </Show>
                        <KindIcon kind={props.row.kind} />
                        <span class="tower-label tower-memory-label">{props.row.label}</span>
                        <Show when={props.row.processes > 1}>
                            <span class="tower-muted">×{props.row.processes}</span>
                        </Show>
                        <Show when={goes()}>
                            <IconButton
                                icon="arrow-up-right-from-square"
                                label={`Show ${props.row.label} in the Agents view`}
                                density="compact"
                                onClick={() => showInAgents(m, props.row, props.part)}
                            />
                        </Show>
                        <Show when={props.shared.growth().get(props.row.key)}>
                            {(g) => <Grow g={g()} testid={`tower-memory-grow-${props.row.key}`} />}
                        </Show>
                    </div>
                </td>
                <td class="tower-num">{formatMem(props.row.mem)}</td>
                <td class="tower-num tower-muted">{formatShare(props.row.mem, total())}</td>
                <TrendCell shared={props.shared} model={m} seriesKey={props.row.key} />
            </tr>
            <Show when={open()}>
                <For each={props.row.children}>
                    {(c) => (
                        <tr class="tower-memory-child" data-testid={`tower-memory-child-${c.key}`}>
                            <td class="tower-name">
                                <div class="tower-name-line">
                                    <span class="tower-label">{c.label}</span>
                                    <Show when={c.processes > 1}>
                                        <span class="tower-muted">×{c.processes}</span>
                                    </Show>
                                </div>
                            </td>
                            <td class="tower-num">{formatMem(c.mem)}</td>
                            <td class="tower-num tower-muted">{formatShare(c.mem, total())}</td>
                            <td class="tower-memory-trend" />
                        </tr>
                    )}
                </For>
            </Show>
        </>
    );
}

/** A line's sparkline cell. */
function TrendCell(props: { shared: Shared; model: TowerViewModel; seriesKey: string }): JSX.Element {
    return (
        <td class="tower-memory-trend">
            <Spark
                series={props.shared.trend()?.byKey.get(props.seriesKey)}
                trend={props.shared.trend()}
                step={props.model.memoryHistory()?.step_ms ?? 10_000}
            />
        </td>
    );
}

/** Choosing a line charts it: a click, or Enter or Space on the row itself. */
function pick(m: TowerViewModel, key: string) {
    return {
        onClick: () => m.setMemoryChart(key),
        tabindex: 0,
        onKeyDown: (e: KeyboardEvent) => {
            if (e.target !== e.currentTarget || (e.key !== "Enter" && e.key !== " ")) return;
            e.preventDefault();
            m.setMemoryChart(key);
        },
    };
}

/** What the number is, and what is left out of it. */
function Footnote(props: { memory: TowerMemory }): JSX.Element {
    return (
        <div class="tower-memory-note tower-muted" data-testid="tower-memory-note">
            <div>
                {count(props.memory.processes, "process")}, measured as {props.memory.metric}: the memory only
                AgentMux's processes use, added up over everything AgentMux started. Shared libraries and shared memory
                aren't counted twice, and video memory isn't counted.
            </div>
            <Show when={props.memory.unreadable > 0}>
                <div>{count(props.memory.unreadable, "process")} couldn't be read and add nothing.</div>
            </Show>
            <Show when={(props.memory.other_instances ?? 0) > 0}>
                <div data-testid="tower-memory-others">
                    Another AgentMux on this computer uses {formatMem(props.memory.other_instances)}, not counted here.
                </div>
            </Show>
        </div>
    );
}
