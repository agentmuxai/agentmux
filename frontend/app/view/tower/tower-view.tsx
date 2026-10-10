// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Tower: what each agent runs and what it costs (the Agents view), and every
// process on the machine (the Processes view)
// (SPEC_TOWER_TASK_MANAGER_PANE_2026_10_08.md,
// SPEC_TOWER_AGENT_CENTRIC_VIEWS_2026_10_08.md). Read-only.

import type { SelectOption } from "@/app/element/ui";
import {
    Button,
    FilterInput,
    IconButton,
    SegmentedControl,
    Select,
    tabPanelId,
    Tabs,
    TextInput,
} from "@/app/element/ui";
import type { TowerPeerInfo, TowerProcess } from "@/app/store/rpc-api";
import type { RemoteRecord } from "@/app/store/rpc-api/remotes";
import clsx from "clsx";
import { createMemo, createSignal, For, type JSX, Match, Show, Switch } from "solid-js";
import { AgentsView } from "./tower-agents";
import { ProcessName } from "./tower-process-name";
import type { TowerViewModel } from "./tower-model";
import { SortHeader } from "./tower-sort-header";
import {
    count,
    type CpuMode,
    filterProcesses,
    formatCpu,
    formatMem,
    groupProcesses,
    processDetail,
    type ProcessGroup,
    sortGroups,
    sortProcesses,
    type TowerView as TowerViewKind,
} from "./tower-util";

import "./tower-view.scss";

/** The process list shows this many rows; the filter narrows the rest. */
export const HOST_ROW_LIMIT = 400;

/** The picker's choice that opens the pairing form rather than a machine. */
export const PAIR_OPTION = "__pair__";

/** The machine picker: this computer, the AgentMux computers it is paired
 *  with, then every SSH host and WSL distribution AgentMux knows (the Remotes
 *  pane's list), and a way to pair another. The helper runs on Linux and
 *  macOS only, so other SSH hosts are listed but can't be picked. */
export function machineOptions(current: string, remotes: RemoteRecord[], peers: TowerPeerInfo[] = []): SelectOption[] {
    const options: SelectOption[] = [{ value: "", label: "This computer" }];
    for (const p of peers) options.push({ value: p.connection, label: `${p.hostname || p.address} (AgentMux)` });
    for (const r of remotes) {
        if (r.kind !== "ssh" && r.kind !== "wsl") continue;
        const os = r.platform?.os;
        // The Remotes list reports `os` lowercased: `linux`, `macos`, else uname -s.
        const supported =
            r.kind === "wsl" || (r.helper?.state !== "unsupported" && (!os || os === "linux" || os === "macos"));
        options.push({
            value: r.name,
            label: supported
                ? r.name
                : `${r.name} (${[os, r.platform?.arch].filter(Boolean).join(" ")}: not supported)`,
            disabled: !supported,
        });
    }
    if (current && !options.some((o) => o.value === current)) options.push({ value: current, label: current });
    options.push({ value: PAIR_OPTION, label: "Pair another AgentMux computer…" });
    return options;
}

export function TowerView(props: { model: TowerViewModel }): JSX.Element {
    const m = props.model;
    const idPrefix = `tower-${m.blockId}`;
    const cpu = (fraction: number | undefined) => formatCpu(fraction, m.snapshot()?.cpu_count ?? 1, m.cpuMode());
    const [pairing, setPairing] = createSignal(false);
    const isPeer = () => m.connection().startsWith("peer:");

    return (
        <div class="tower-view" data-testid="tower-view">
            <div class="tower-toolbar">
                <Select
                    density="compact"
                    ariaLabel="Machine"
                    class="tower-machine"
                    value={pairing() ? PAIR_OPTION : m.connection()}
                    onChange={(c) => {
                        setPairing(c === PAIR_OPTION);
                        if (c !== PAIR_OPTION) m.setConnection(c);
                    }}
                    options={machineOptions(m.connection(), m.remotes(), m.peers())}
                />
                <Show when={isPeer() && !pairing()}>
                    <IconButton
                        icon="link-slash"
                        label="Forget this computer"
                        density="compact"
                        onClick={() => void m.forget(m.connection())}
                    />
                </Show>
                <Tabs<TowerViewKind>
                    items={
                        !m.hasTasks()
                            ? [
                                  {
                                      id: "processes",
                                      label: "Processes",
                                      icon: "server",
                                      tooltip: "Every process on that machine",
                                  },
                              ]
                            : [
                                  {
                                      id: "agents",
                                      label: "Agents",
                                      icon: "robot",
                                      tooltip: "Each agent and what it runs",
                                  },
                                  {
                                      id: "processes",
                                      label: "Processes",
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
            <Show when={pairing()}>
                <PairForm model={m} onDone={() => setPairing(false)} />
            </Show>
            <Show when={m.error()}>
                {(err) => (
                    <div class="tower-error" role="alert">
                        Couldn't read processes: {err()}
                        <Show when={m.stalled()}>
                            {" "}
                            <Button density="compact" onClick={() => m.retry()}>
                                Retry
                            </Button>
                        </Show>
                    </div>
                )}
            </Show>
            <div id={tabPanelId(idPrefix)} role="tabpanel" class="tower-body">
                <Show when={m.snapshot()} fallback={<div class="tower-empty">Measuring…</div>}>
                    {(snap) => (
                        <Switch>
                            <Match when={m.effectiveView() === "agents"}>
                                <AgentsView model={m} cpu={cpu} />
                            </Match>
                            <Match when={m.effectiveView() === "processes"}>
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

/** Pair with another AgentMux computer: paste the link from its "Pair a
 *  device" panel. Its user decides what it shares (Tower's settings there). */
function PairForm(props: { model: TowerViewModel; onDone: () => void }) {
    const [link, setLink] = createSignal("");
    const [busy, setBusy] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);
    const submit = async () => {
        setBusy(true);
        setError(null);
        try {
            await props.model.pair(link());
            props.onDone();
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        } finally {
            setBusy(false);
        }
    };
    return (
        <div class="tower-pair" data-testid="tower-pair">
            <div class="tower-muted">
                On the other computer, open <b>Pair a device</b> in the status bar, choose <b>Copy link</b>, and paste
                it here. It works once, for two minutes. That computer shows its processes only if its user turns on
                sharing in its Tower.
            </div>
            <div class="tower-pair-row">
                <TextInput
                    value={link()}
                    onInput={(e) => setLink(e.currentTarget.value)}
                    placeholder="agentmux://pair?…"
                    aria-label="Pairing link"
                    density="compact"
                    class="tower-pair-input"
                />
                <Button
                    tone="accent"
                    density="compact"
                    busy={busy()}
                    disabled={!link().trim()}
                    onClick={() => void submit()}
                >
                    Pair
                </Button>
                <Button density="compact" onClick={() => props.onDone()}>
                    Cancel
                </Button>
            </div>
            <Show when={error()}>
                <div class="tower-error-inline" role="alert">
                    {error()}
                </div>
            </Show>
        </div>
    );
}

function ProcessRow(props: {
    model: TowerViewModel;
    process: TowerProcess;
    cpu: (f: number | undefined) => string;
    nested?: boolean;
    /** A group of one in the grouped Host view: aligned with the group names,
     *  and counted as 1 (its PID is in the hover details). */
    single?: boolean;
    taskLabel?: string;
}) {
    return (
        <tr
            class={clsx(
                "tower-process-row",
                props.nested && "tower-process-row--nested",
                props.single && "tower-process-row--single",
                props.process.role && `tower-role--${props.process.role}`
            )}
            title={processDetail(props.process, props.model.snapshot()?.memory_metric ?? "")}
        >
            <td class="tower-name">
                <div class="tower-name-line">
                    <ProcessName process={props.process} />
                    <Show when={props.taskLabel}>
                        {(label) => <span class="tower-badge tower-badge--task">{label()}</span>}
                    </Show>
                </div>
            </td>
            <td class="tower-num">{props.cpu(props.process.cpu)}</td>
            <td class="tower-num">{formatMem(props.process.mem)}</td>
            <td class="tower-num tower-muted">{props.single ? 1 : props.process.pid}</td>
        </tr>
    );
}

/** One app in the grouped Host view: a single process as its own row, or a
 *  row with the app's count and totals that opens to its processes. */
function GroupRows(props: { model: TowerViewModel; group: ProcessGroup; cpu: (f: number | undefined) => string }) {
    const m = props.model;
    const open = () => m.expanded().has(`app:${props.group.key}`);
    const processes = createMemo(() => (open() ? sortProcesses(props.group.processes, m.sort()) : []));
    const label = (task: string | undefined) => (task ? m.taskLabel(task) : undefined);
    return (
        <Show
            when={props.group.processes.length > 1}
            fallback={
                <ProcessRow
                    model={m}
                    process={props.group.processes[0]}
                    cpu={props.cpu}
                    single
                    taskLabel={label(props.group.processes[0].task)}
                />
            }
        >
            <tr class="tower-group-row" data-testid={`tower-app-${props.group.key}`}>
                <td class="tower-name">
                    <div class="tower-name-line">
                        <IconButton
                            icon={open() ? "chevron-down" : "chevron-right"}
                            label={open() ? "Hide processes" : "Show processes"}
                            density="compact"
                            tooltip={false}
                            aria-expanded={open()}
                            onClick={() => m.toggleExpanded(`app:${props.group.key}`)}
                        />
                        <span class="tower-label">{props.group.name}</span>
                        <span class="tower-muted">({props.group.processes.length})</span>
                        <Show when={label(props.group.task)}>
                            {(l) => <span class="tower-badge tower-badge--task">{l()}</span>}
                        </Show>
                    </div>
                </td>
                <td class="tower-num">{props.cpu(props.group.cpu)}</td>
                <td class="tower-num">{formatMem(props.group.mem)}</td>
                <td class="tower-num">{props.group.processes.length}</td>
            </tr>
            <For each={processes()}>
                {(p) => <ProcessRow model={m} process={p} cpu={props.cpu} nested taskLabel={label(p.task)} />}
            </For>
        </Show>
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
    // Grouped by app: rows keyed by the app's name, so a group stays the same
    // row (and stays open) across refreshes.
    const groups = createMemo(() => sortGroups(groupProcesses(rows()), m.sort()));
    const groupByKey = createMemo(() => new Map(groups().map((g) => [g.key, g])));
    const shown = () => (m.groupByApp() ? groups().length : rows().length);
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
            <div class="tower-host-bar">
                <FilterInput
                    value={m.filter()}
                    onInput={(q) => m.setFilter(q)}
                    placeholder="Filter by name, PID or task"
                    class="tower-filter"
                    testId="tower-filter"
                />
                <Button
                    density="compact"
                    icon="layer-group"
                    pressed={m.groupByApp()}
                    onClick={() => m.setGroupByApp(!m.groupByApp())}
                    title="Group processes of the same app, as Task Manager does"
                >
                    Group by app
                </Button>
            </div>
            <table class="tower-table" aria-label="Processes">
                <thead>
                    <tr>
                        <SortHeader model={m} key="name" label="Process" />
                        <SortHeader model={m} key="cpu" label="CPU" numeric />
                        <SortHeader model={m} key="mem" label="Memory" numeric />
                        <SortHeader
                            model={m}
                            key="count"
                            label={m.groupByApp() ? "Count" : "PID"}
                            numeric
                            title={m.groupByApp() ? "Processes in the app, or the PID of one" : undefined}
                        />
                    </tr>
                </thead>
                <tbody>
                    <Show
                        when={m.groupByApp()}
                        fallback={
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
                        }
                    >
                        <For
                            each={groups()
                                .slice(0, HOST_ROW_LIMIT)
                                .map((g) => g.key)}
                        >
                            {(key) => (
                                <Show when={groupByKey().get(key)}>
                                    {(group) => <GroupRows model={m} group={group()} cpu={props.cpu} />}
                                </Show>
                            )}
                        </For>
                    </Show>
                </tbody>
            </table>
            <Show when={shown() > HOST_ROW_LIMIT}>
                <div class="tower-muted tower-more">
                    {shown() - HOST_ROW_LIMIT} more not shown: filter to narrow the list.
                </div>
            </Show>
        </>
    );
}
