// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Tower: CPU and memory per task (each pane and everything it started, plus
// AgentMux itself), and every process on the machine
// (SPEC_TOWER_TASK_MANAGER_PANE_2026_10_08.md). Read-only.

import type { SelectOption } from "@/app/element/ui";
import { Button, FilterInput, IconButton, SegmentedControl, Select, tabPanelId, Tabs } from "@/app/element/ui";
import type { TowerProcess, TowerTask } from "@/app/store/rpc-api";
import type { RemoteRecord } from "@/app/store/rpc-api/remotes";
import { revealBlock } from "@/app/util/reveal-block";
import clsx from "clsx";
import { createMemo, For, type JSX, Match, Show, Switch } from "solid-js";
import type { TowerViewModel } from "./tower-model";
import {
    count,
    type CpuMode,
    filterProcesses,
    formatCpu,
    formatMem,
    nextSort,
    processDetail,
    type SortKey,
    sortProcesses,
    sortTasks,
    type TowerView as TowerViewKind,
    trackingLabel,
} from "./tower-util";

import "./tower-view.scss";

/** The host list shows this many rows; the filter narrows the rest. */
export const HOST_ROW_LIMIT = 400;

const KIND_LABELS: Record<TowerTask["kind"], string> = {
    agent: "agent",
    terminal: "terminal",
    agentmux: "app",
};

/** The machine picker: this computer, then every SSH host and WSL
 *  distribution AgentMux knows (the Remotes pane's list). The helper runs on
 *  Linux and macOS only, so other SSH hosts are listed but can't be picked. */
export function machineOptions(current: string, remotes: RemoteRecord[]): SelectOption[] {
    const options: SelectOption[] = [{ value: "", label: "This computer" }];
    for (const r of remotes) {
        if (r.kind !== "ssh" && r.kind !== "wsl") continue;
        const os = r.platform?.os;
        const supported = r.kind === "wsl" || !os || os === "Linux" || os === "Darwin";
        options.push({
            value: r.name,
            label: supported ? r.name : `${r.name} (${os}: not supported)`,
            disabled: !supported,
        });
    }
    if (current && !options.some((o) => o.value === current)) options.push({ value: current, label: current });
    return options;
}

export function TowerView(props: { model: TowerViewModel }): JSX.Element {
    const m = props.model;
    const idPrefix = `tower-${m.blockId}`;
    const cpu = (fraction: number | undefined) => formatCpu(fraction, m.snapshot()?.cpu_count ?? 1, m.cpuMode());

    return (
        <div class="tower-view" data-testid="tower-view">
            <div class="tower-toolbar">
                <Select
                    density="compact"
                    ariaLabel="Machine"
                    class="tower-machine"
                    value={m.connection()}
                    onChange={(c) => m.setConnection(c)}
                    options={machineOptions(m.connection(), m.remotes())}
                />
                <Tabs<TowerViewKind>
                    items={
                        m.remote()
                            ? [{ id: "host", label: "Host", icon: "server", tooltip: "Every process on that machine" }]
                            : [
                                  {
                                      id: "tasks",
                                      label: "Tasks",
                                      icon: "layer-group",
                                      tooltip: "Each pane and what it started",
                                  },
                                  {
                                      id: "host",
                                      label: "Host",
                                      icon: "server",
                                      tooltip: "Every process on this machine",
                                  },
                              ]
                    }
                    value={m.effectiveView()}
                    onChange={(v) => m.setView(v)}
                    idPrefix={idPrefix}
                    ariaLabel="Tower view"
                    density="compact"
                />
                <div class="tower-toolbar-spacer" />
                <SegmentedControl<CpuMode>
                    density="compact"
                    ariaLabel="Show CPU as"
                    options={[
                        {
                            value: "machine",
                            label: "% of machine",
                            title: "Share of the whole machine, as Windows Task Manager shows it",
                        },
                        {
                            value: "core",
                            label: "% of a core",
                            title: "Share of one core, as top shows it: can pass 100%",
                        },
                    ]}
                    value={m.cpuMode()}
                    onChange={(v) => m.setCpuMode(v)}
                />
            </div>
            <Show when={m.error()}>
                {(err) => (
                    <div class="tower-error" role="alert">
                        Couldn't read processes: {err()}
                    </div>
                )}
            </Show>
            <div id={tabPanelId(idPrefix)} role="tabpanel" class="tower-body">
                <Show when={m.snapshot()} fallback={<div class="tower-empty">Measuring…</div>}>
                    {(snap) => (
                        <Switch>
                            <Match when={m.effectiveView() === "tasks"}>
                                <TasksTable model={m} cpu={cpu} />
                            </Match>
                            <Match when={m.effectiveView() === "host"}>
                                <Show when={snap().host} fallback={<div class="tower-empty">Measuring…</div>}>
                                    <HostTable model={m} cpu={cpu} />
                                </Show>
                            </Match>
                        </Switch>
                    )}
                </Show>
            </div>
        </div>
    );
}

function SortHeader(props: { model: TowerViewModel; key: SortKey; label: string; numeric?: boolean; title?: string }) {
    const active = () => props.model.sort().key === props.key;
    return (
        <th
            class={clsx(props.numeric && "tower-num")}
            aria-sort={active() ? (props.model.sort().desc ? "descending" : "ascending") : "none"}
        >
            <Button
                tone="quiet"
                density="compact"
                class="tower-sort"
                title={props.title}
                icon={active() ? (props.model.sort().desc ? "arrow-down" : "arrow-up") : undefined}
                onClick={() => props.model.setSort(nextSort(props.model.sort(), props.key))}
            >
                {props.label}
            </Button>
        </th>
    );
}

function TasksTable(props: { model: TowerViewModel; cpu: (f: number | undefined) => string }) {
    const m = props.model;
    const tasks = createMemo(() => sortTasks(m.snapshot()?.tasks ?? [], m.sort()));
    const totals = createMemo(() => {
        const ts = m.snapshot()?.tasks ?? [];
        const rates = ts.map((t) => t.cpu).filter((c): c is number => c != null);
        return {
            processes: ts.reduce((n, t) => n + t.processes.length, 0),
            cpu: rates.length ? rates.reduce((a, b) => a + b, 0) : undefined,
            mem: ts.reduce((n, t) => n + t.mem, 0),
        };
    });
    return (
        <>
            <div class="tower-summary">
                {count(tasks().length, "task")} · {count(totals().processes, "process")} · CPU {props.cpu(totals().cpu)}{" "}
                · Memory {formatMem(totals().mem)}
                <span
                    class="tower-muted"
                    title={`Memory is each process's ${m.snapshot()?.memory_metric}: the part only it uses.`}
                >
                    {" "}
                    ({m.snapshot()?.memory_metric})
                </span>
            </div>
            <table class="tower-table" aria-label="Tasks">
                <thead>
                    <tr>
                        <SortHeader model={m} key="name" label="Task" />
                        <SortHeader model={m} key="cpu" label="CPU" numeric />
                        <SortHeader model={m} key="mem" label="Memory" numeric />
                        <SortHeader
                            model={m}
                            key="count"
                            label="Processes"
                            numeric
                            title="Processes, or PID in a task's list"
                        />
                    </tr>
                </thead>
                <tbody>
                    <For
                        each={tasks()}
                        fallback={
                            <tr>
                                <td colSpan={4} class="tower-empty">
                                    No tasks running.
                                </td>
                            </tr>
                        }
                    >
                        {(task) => <TaskRows model={m} task={task} cpu={props.cpu} />}
                    </For>
                </tbody>
            </table>
        </>
    );
}

function TaskRows(props: { model: TowerViewModel; task: TowerTask; cpu: (f: number | undefined) => string }) {
    const m = props.model;
    const open = () => m.expanded().has(props.task.id);
    const processes = createMemo(() => (open() ? sortProcesses(props.task.processes, m.sort()) : []));
    return (
        <>
            <tr class="tower-task-row" data-testid={`tower-task-${props.task.id}`}>
                <td class="tower-name">
                    <div class="tower-name-line">
                        <IconButton
                            icon={open() ? "chevron-down" : "chevron-right"}
                            label={open() ? "Hide processes" : "Show processes"}
                            density="compact"
                            tooltip={false}
                            aria-expanded={open()}
                            onClick={() => m.toggleExpanded(props.task.id)}
                        />
                        <span class="tower-label" title={trackingLabel(props.task.tracking)}>
                            {props.task.label}
                        </span>
                        <span class={clsx("tower-badge", `tower-badge--${props.task.kind}`)}>
                            {KIND_LABELS[props.task.kind]}
                        </span>
                        <Show when={props.task.kind !== "agentmux"}>
                            <IconButton
                                icon="arrow-up-right-from-square"
                                label="Show this pane"
                                density="compact"
                                class="tower-reveal"
                                onClick={() => void revealBlock(props.task.id)}
                            />
                        </Show>
                    </div>
                </td>
                <td
                    class="tower-num"
                    title={props.task.cpu_account ? "Includes processes that exited since the last refresh" : undefined}
                >
                    {props.cpu(props.task.cpu)}
                </td>
                <td class="tower-num">{formatMem(props.task.mem)}</td>
                <td class="tower-num">{props.task.processes.length}</td>
            </tr>
            <For each={processes()}>{(p) => <ProcessRow model={m} process={p} cpu={props.cpu} nested />}</For>
        </>
    );
}

function ProcessRow(props: {
    model: TowerViewModel;
    process: TowerProcess;
    cpu: (f: number | undefined) => string;
    nested?: boolean;
    taskLabel?: string;
}) {
    return (
        <tr
            class={clsx(
                "tower-process-row",
                props.nested && "tower-process-row--nested",
                props.process.role && `tower-role--${props.process.role}`
            )}
            title={processDetail(props.process, props.model.snapshot()?.memory_metric ?? "")}
        >
            <td class="tower-name">
                <div class="tower-name-line">
                    <span class="tower-label">{props.process.name || `PID ${props.process.pid}`}</span>
                    <Show when={props.taskLabel}>
                        {(label) => <span class="tower-badge tower-badge--task">{label()}</span>}
                    </Show>
                </div>
            </td>
            <td class="tower-num">{props.cpu(props.process.cpu)}</td>
            <td class="tower-num">{formatMem(props.process.mem)}</td>
            <td class="tower-num tower-muted">{props.process.pid}</td>
        </tr>
    );
}

function HostTable(props: { model: TowerViewModel; cpu: (f: number | undefined) => string }) {
    const m = props.model;
    const host = () => m.snapshot()?.host;
    /** Another machine sends only its busiest and largest processes. */
    const partial = () => (host()?.processes.length ?? 0) < (host()?.matched ?? 0);
    const rows = createMemo(() => {
        const all = host()?.processes ?? [];
        return sortProcesses(filterProcesses(all, m.filter(), m.taskLabel), m.sort());
    });
    return (
        <>
            <div class="tower-summary">
                {count(host()?.total ?? 0, "process")} · CPU {props.cpu(host()?.cpu)} · Memory {formatMem(host()?.mem)}
                <Show when={partial()}>
                    <span class="tower-muted">
                        {" "}
                        · showing the {host()?.processes.length} busiest and largest
                        {(host()?.matched ?? 0) < (host()?.total ?? 0) ? ` of ${host()?.matched} matching` : ""}
                    </span>
                </Show>
                <Show when={(host()?.unmeasured ?? 0) > 0}>
                    <span class="tower-muted">
                        {" "}
                        · {host()?.unmeasured} owned by other users can't be measured without administrator rights
                    </span>
                </Show>
            </div>
            <FilterInput
                value={m.filter()}
                onInput={(q) => m.setFilter(q)}
                placeholder="Filter by name, PID or task"
                class="tower-filter"
                testId="tower-filter"
            />
            <table class="tower-table" aria-label="Processes">
                <thead>
                    <tr>
                        <SortHeader model={m} key="name" label="Process" />
                        <SortHeader model={m} key="cpu" label="CPU" numeric />
                        <SortHeader model={m} key="mem" label="Memory" numeric />
                        <SortHeader model={m} key="count" label="PID" numeric />
                    </tr>
                </thead>
                <tbody>
                    <For each={rows().slice(0, HOST_ROW_LIMIT)}>
                        {(p) => (
                            <ProcessRow
                                model={m}
                                process={p}
                                cpu={props.cpu}
                                taskLabel={p.task ? m.taskLabel(p.task) : undefined}
                            />
                        )}
                    </For>
                </tbody>
            </table>
            <Show when={rows().length > HOST_ROW_LIMIT}>
                <div class="tower-muted tower-more">
                    {rows().length - HOST_ROW_LIMIT} more not shown: filter to narrow the list.
                </div>
            </Show>
        </>
    );
}
