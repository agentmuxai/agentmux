// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Tower's Agents view: a rail of the agents running here, by color, with what
// each costs, and the selected one's processes as a tree
// (SPEC_TOWER_AGENT_CENTRIC_VIEWS_2026_10_08.md §3.1).

import { blockRoleColor, isLightThemeActive } from "@/app/block/pane-identity";
import { Button, IconButton, Select } from "@/app/element/ui";
import * as MOS from "@/app/store/mos";
import type { TowerProcess, TowerTask } from "@/app/store/rpc-api";
import { revealBlock } from "@/app/util/reveal-block";
import { pickAgentColor } from "@/app/view/agent/agent-color";
import clsx from "clsx";
import { createMemo, createSignal, For, type JSX, Show } from "solid-js";
import { HISTORY_POINTS, type TowerViewModel } from "./tower-model";
import { ProcessName } from "./tower-process-name";
import { SortHeader } from "./tower-sort-header";
import {
    buildProcessTree,
    count,
    formatMem,
    orderRail,
    processDetail,
    type ProcessNode,
    railEntries,
    type RailEntry,
    type RailSort,
    type TreeLine,
    treeLines,
} from "./tower-util";

type Cpu = (fraction: number | undefined) => string;

const FIXED_ICONS: Record<Exclude<RailEntry["kind"], "agent">, string> = {
    terminals: "terminal",
    agentmux: "window-maximize",
    other: "ellipsis",
};

/** An agent's color: its pane's (the user's pick, else the agent's color),
 *  as the pane's own tab shows it; another computer's agents, whose panes
 *  aren't here, get the color their name picks. */
export function agentColor(taskId: string, label: string, remote: boolean): string {
    if (!remote) {
        const block = MOS.getObjectValue<Block>(MOS.makeORef("block", taskId));
        const color = blockRoleColor(block?.meta, isLightThemeActive(), "pill", { widget: false });
        if (color) return color;
    }
    return pickAgentColor(label);
}

/** A rail entry's color: its agent's; the fixed entries have none. */
export function entryColor(entry: RailEntry, remote: boolean): string | undefined {
    return entry.kind === "agent" ? agentColor(entry.id, entry.label, remote) : undefined;
}

export function AgentsView(props: { model: TowerViewModel; cpu: Cpu }): JSX.Element {
    const m = props.model;
    const entries = createMemo(() => {
        const snap = m.snapshot();
        return snap ? railEntries(snap) : [];
    });
    // While the pointer is over the rail its order holds still, so a row
    // never moves out from under a click (Task Manager's freeze, without a
    // hidden key).
    const [frozen, setFrozen] = createSignal<string[] | null>(null);
    const ordered = createMemo(() => {
        const live = orderRail(entries(), m.railSort(), m.smoothedCpu);
        const hold = frozen();
        if (!hold) return live;
        const byId = new Map(live.map((e) => [e.id, e]));
        const kept = hold.map((id) => byId.get(id)).filter((e): e is RailEntry => e != null);
        return [...kept, ...live.filter((e) => !hold.includes(e.id))];
    });
    const orderIds = createMemo(() => ordered().map((e) => e.id));
    const byId = createMemo(() => new Map(entries().map((e) => [e.id, e])));
    const selected = createMemo(() => {
        const ids = orderIds();
        return ids.includes(m.selected()) ? m.selected() : (ids[0] ?? "");
    });
    const move = (step: number) => {
        const ids = orderIds();
        const i = ids.indexOf(selected());
        const next = ids[Math.min(ids.length - 1, Math.max(0, i + step))];
        if (next) m.select(next);
    };
    return (
        <div class="tower-agents" data-testid="tower-agents">
            <div class="tower-rail">
                <div class="tower-rail-bar">
                    <Select
                        density="compact"
                        ariaLabel="Order agents by"
                        value={m.railSort()}
                        onChange={(v) => m.setRailSort(v as RailSort)}
                        options={[
                            { value: "pane", label: "Pane order" },
                            { value: "cpu", label: "CPU" },
                            { value: "mem", label: "Memory" },
                            { value: "name", label: "Name" },
                        ]}
                    />
                </div>
                <div
                    class="tower-rail-list"
                    role="listbox"
                    aria-label="Agents"
                    tabIndex={0}
                    aria-activedescendant={selected() ? `tower-rail-${m.blockId}-${selected()}` : undefined}
                    onPointerEnter={() => setFrozen(orderIds())}
                    onPointerLeave={() => setFrozen(null)}
                    onKeyDown={(e) => {
                        if (e.key === "ArrowDown" || e.key === "ArrowUp") {
                            e.preventDefault();
                            move(e.key === "ArrowDown" ? 1 : -1);
                        }
                    }}
                >
                    <For each={orderIds()}>
                        {(id) => (
                            <Show when={byId().get(id)}>
                                {(entry) => (
                                    <RailRow model={m} entry={entry()} cpu={props.cpu} selected={selected() === id} />
                                )}
                            </Show>
                        )}
                    </For>
                </div>
            </div>
            <div class="tower-detail">
                <Show when={byId().get(selected())} fallback={<div class="tower-empty">No agents running.</div>}>
                    {(entry) => <Detail model={m} entry={entry()} cpu={props.cpu} />}
                </Show>
            </div>
        </div>
    );
}

function Swatch(props: { entry: RailEntry; remote: boolean }) {
    return (
        <Show
            when={props.entry.kind === "agent"}
            fallback={
                <span class="tower-swatch tower-swatch--fixed" aria-hidden="true">
                    <i
                        class={`fa fa-solid fa-${FIXED_ICONS[props.entry.kind as Exclude<RailEntry["kind"], "agent">]}`}
                    />
                </span>
            }
        >
            <span
                class="tower-swatch"
                aria-hidden="true"
                style={{ "background-color": entryColor(props.entry, props.remote) }}
            />
        </Show>
    );
}

function RailRow(props: { model: TowerViewModel; entry: RailEntry; cpu: Cpu; selected: boolean }) {
    const m = props.model;
    const remote = () => m.snapshot()?.remote ?? false;
    return (
        <div
            id={`tower-rail-${m.blockId}-${props.entry.id}`}
            role="option"
            aria-selected={props.selected}
            class={clsx(
                "tower-rail-row",
                props.selected && "tower-rail-row--selected",
                `tower-rail-row--${props.entry.kind}`
            )}
            data-testid={`tower-rail-${props.entry.id}`}
            title={`${props.entry.label}: ${count(props.entry.processes, "process")}`}
            onClick={() => m.select(props.entry.id)}
        >
            <Swatch entry={props.entry} remote={remote()} />
            <span class="tower-rail-name">{props.entry.label}</span>
            <span class="tower-rail-cpu tower-num">{props.cpu(props.entry.cpu)}</span>
            <span class="tower-rail-mem tower-num tower-muted">{formatMem(props.entry.mem)}</span>
            <Sparkline points={m.history().get(props.entry.id) ?? []} color={entryColor(props.entry, remote())} />
        </div>
    );
}

/** The last minute of CPU, scaled to the entry's own peak (at least half a
 *  core, so an idle agent stays flat instead of magnifying noise). */
export function Sparkline(props: { points: readonly number[]; color?: string }) {
    const path = () => {
        const pts = props.points;
        if (pts.length < 2) return "";
        const peak = Math.max(0.5, ...pts);
        const step = 60 / (HISTORY_POINTS - 1);
        const x0 = 60 - step * (pts.length - 1);
        return pts.map((v, i) => `${(x0 + i * step).toFixed(1)},${(15 - (v / peak) * 14).toFixed(1)}`).join(" ");
    };
    return (
        <svg class="tower-spark" viewBox="0 0 60 16" preserveAspectRatio="none" aria-hidden="true">
            <polyline points={path()} fill="none" stroke={props.color ?? "currentColor"} stroke-width="1.5" />
        </svg>
    );
}

function Detail(props: { model: TowerViewModel; entry: RailEntry; cpu: Cpu }) {
    const m = props.model;
    const remote = () => m.snapshot()?.remote ?? false;
    // Every tree node with children, for Collapse all.
    const parents = createMemo(() => {
        const keys: string[] = [];
        const walk = (n: ProcessNode) => {
            if (n.children.length) keys.push(foldKey(n.process));
            n.children.forEach(walk);
        };
        for (const t of props.entry.tasks) buildProcessTree(t.processes).forEach(walk);
        return keys;
    });
    return (
        <>
            <div class="tower-detail-head">
                <Swatch entry={props.entry} remote={remote()} />
                <span class="tower-detail-name">{props.entry.label}</span>
                <Show when={props.entry.kind === "agent" && !remote()}>
                    <IconButton
                        icon="arrow-up-right-from-square"
                        label="Show this pane"
                        density="compact"
                        onClick={() => void revealBlock(props.entry.id)}
                    />
                </Show>
                <span class="tower-toolbar-spacer" />
                <span class="tower-detail-totals">
                    CPU {props.cpu(props.entry.cpu)} · Memory {formatMem(props.entry.mem)} ·{" "}
                    {count(props.entry.processes, "process")}
                </span>
            </div>
            <Show
                when={props.entry.kind !== "other"}
                fallback={
                    <div class="tower-empty">
                        <p>
                            {count(props.entry.processes, "process")} AgentMux didn't start: other apps, services and
                            the system.
                        </p>
                        <Button density="compact" onClick={() => m.setView("processes", props.entry.id)}>
                            Show in Processes
                        </Button>
                    </div>
                }
            >
                <div class="tower-detail-bar">
                    <Button density="compact" tone="quiet" icon="angles-down" onClick={() => m.unfold(parents())}>
                        Expand all
                    </Button>
                    <Button density="compact" tone="quiet" icon="angles-up" onClick={() => m.fold(parents())}>
                        Collapse all
                    </Button>
                </div>
                <table class="tower-table" aria-label={`${props.entry.label}'s processes`}>
                    <thead>
                        <tr>
                            <SortHeader model={m} key="name" label="Process" />
                            <SortHeader model={m} key="cpu" label="CPU" numeric />
                            <SortHeader model={m} key="mem" label="Memory" numeric />
                            <SortHeader model={m} key="count" label="PID" numeric />
                        </tr>
                    </thead>
                    <tbody>
                        <For each={props.entry.tasks}>
                            {(task) => (
                                <TaskTree
                                    model={m}
                                    task={task}
                                    cpu={props.cpu}
                                    heading={props.entry.kind === "terminals"}
                                />
                            )}
                        </For>
                    </tbody>
                </table>
            </Show>
        </>
    );
}

function foldKey(p: TowerProcess): string {
    return `fold:${p.id}`;
}

/** One task's processes as a tree; under Terminals, headed by the terminal. */
function TaskTree(props: { model: TowerViewModel; task: TowerTask; cpu: Cpu; heading: boolean }) {
    const roots = createMemo(() => buildProcessTree(props.task.processes));
    return (
        <>
            <Show when={props.heading}>
                <tr class="tower-task-row" data-testid={`tower-task-${props.task.id}`}>
                    <td class="tower-name">
                        <div class="tower-name-line">
                            <span class="tower-label">{props.task.label}</span>
                        </div>
                    </td>
                    <td class="tower-num">{props.cpu(props.task.cpu)}</td>
                    <td class="tower-num">{formatMem(props.task.mem)}</td>
                    <td class="tower-num tower-muted">{props.task.processes.length}</td>
                </tr>
            </Show>
            <TreeRows
                model={props.model}
                nodes={roots()}
                depth={props.heading ? 1 : 0}
                cpu={props.cpu}
                scope={props.task.id}
            />
        </>
    );
}

function TreeRows(props: {
    model: TowerViewModel;
    nodes: ProcessNode[];
    depth: number;
    cpu: Cpu;
    scope: string;
    /** The processes an opened "×n" line stands for: listed one by one. */
    unfolded?: boolean;
}) {
    const m = props.model;
    const lines = createMemo(() => treeLines(props.nodes, m.sort(), { fold: !props.unfolded }));
    const byKey = createMemo(() => new Map(lines().map((l) => [l.key, l])));
    // Keyed by the line's key, so a line keeps its row (and its open state)
    // across refreshes; a key is always the same kind of line.
    return (
        <For each={lines().map((l) => l.key)}>
            {(key) => (
                <Show when={byKey().get(key)}>
                    {(line) =>
                        line().kind === "many" ? (
                            <ManyRows
                                model={m}
                                line={line() as ManyLine}
                                depth={props.depth}
                                cpu={props.cpu}
                                scope={props.scope}
                            />
                        ) : (
                            <ProcessRows
                                model={m}
                                node={(line() as ProcessLine).node}
                                depth={props.depth}
                                cpu={props.cpu}
                                scope={props.scope}
                            />
                        )
                    }
                </Show>
            )}
        </For>
    );
}

type ManyLine = Extract<TreeLine, { kind: "many" }>;
type ProcessLine = Extract<TreeLine, { kind: "process" }>;

const indentStyle = (depth: number) => ({ "padding-left": `${8 + depth * 16}px` });

/** Same-named siblings with nothing under them: one line, opening to each. */
function ManyRows(props: { model: TowerViewModel; line: ManyLine; depth: number; cpu: Cpu; scope: string }) {
    const m = props.model;
    const openKey = () => `many:${props.scope}:${props.line.key}`;
    const open = () => m.expanded().has(openKey());
    return (
        <>
            <tr class="tower-process-row tower-many-row" data-testid={`tower-many-${props.line.name}`}>
                <td class="tower-name">
                    <div class="tower-name-line" style={indentStyle(props.depth)}>
                        <IconButton
                            icon={open() ? "chevron-down" : "chevron-right"}
                            label={
                                open()
                                    ? `Hide the ${props.line.name} processes`
                                    : `Show the ${props.line.name} processes`
                            }
                            density="compact"
                            tooltip={false}
                            aria-expanded={open()}
                            onClick={() => m.toggleExpanded(openKey())}
                        />
                        <span class="tower-label">{props.line.name}</span>
                        <span class="tower-muted">×{props.line.nodes.length}</span>
                    </div>
                </td>
                <td class="tower-num">{props.cpu(props.line.cpu)}</td>
                <td class="tower-num">{formatMem(props.line.mem)}</td>
                <td class="tower-num tower-muted">{props.line.nodes.length}</td>
            </tr>
            <Show when={open()}>
                <TreeRows
                    model={m}
                    nodes={props.line.nodes}
                    depth={props.depth + 1}
                    cpu={props.cpu}
                    scope={`${props.scope}:${props.line.key}`}
                    unfolded
                />
            </Show>
        </>
    );
}

/** A process, and what it started below it unless folded. Its CPU and
 *  memory include everything under it. */
function ProcessRows(props: { model: TowerViewModel; node: ProcessNode; depth: number; cpu: Cpu; scope: string }) {
    const m = props.model;
    const hasChildren = () => props.node.children.length > 0;
    const folded = () => m.expanded().has(foldKey(props.node.process));
    const subtree = () => (hasChildren() ? "With everything it started" : undefined);
    return (
        <>
            <tr
                class={clsx("tower-process-row", props.node.process.role && `tower-role--${props.node.process.role}`)}
                title={processDetail(props.node.process, m.snapshot()?.memory_metric ?? "")}
            >
                <td class="tower-name">
                    <div class="tower-name-line" style={indentStyle(props.depth)}>
                        <Show when={hasChildren()} fallback={<span class="tower-chevron-space" />}>
                            <IconButton
                                icon={folded() ? "chevron-right" : "chevron-down"}
                                label={folded() ? "Show what it started" : "Hide what it started"}
                                density="compact"
                                tooltip={false}
                                aria-expanded={!folded()}
                                onClick={() => m.toggleExpanded(foldKey(props.node.process))}
                            />
                        </Show>
                        <ProcessName process={props.node.process} />
                    </div>
                </td>
                <td class="tower-num" title={subtree()}>
                    {props.cpu(props.node.cpu)}
                </td>
                <td class="tower-num" title={subtree()}>
                    {formatMem(props.node.mem)}
                </td>
                <td class="tower-num tower-muted">{props.node.process.pid}</td>
            </tr>
            <Show when={hasChildren() && !folded()}>
                {/* The parent joins the scope: an "×n" line under one parent
                    opens on its own, not with a same-named one elsewhere. */}
                <TreeRows
                    model={m}
                    nodes={props.node.children}
                    depth={props.depth + 1}
                    cpu={props.cpu}
                    scope={`${props.scope}>${props.node.process.id}`}
                />
            </Show>
        </>
    );
}
