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
import { createEffect, createMemo, createSignal, For, type JSX, Match, Show, Switch } from "solid-js";
import { agentColor, AgentsView } from "./tower-agents";
import type { TowerViewModel } from "./tower-model";
import { ProcessName } from "./tower-process-name";
import { SortHeader } from "./tower-sort-header";
import {
    count,
    type CpuMode,
    filterProcesses,
    formatCpu,
    formatMem,
    groupProcesses,
    type OwnerGroup,
    ownerGroups,
    processDetail,
    type ProcessGroup,
    type ProcessGrouping,
    sortGroups,
    sortOwnerGroups,
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
    /** A group of one in the list grouped by app: aligned with the group
     *  names, and counted as 1 (its PID is in the hover details). */
    single?: boolean;
    /** Two levels in: an app's process inside the "Other processes" group. */
    deep?: boolean;
    taskLabel?: string;
    /** Under an agent in the list grouped by agent: the agent's color. */
    ownerColor?: string;
}) {
    return (
        <tr
            style={props.ownerColor ? { "--tower-owner-color": props.ownerColor } : undefined}
            class={clsx(
                "tower-process-row",
                props.nested && "tower-process-row--nested",
                props.deep && "tower-process-row--deep",
                props.ownerColor && "tower-process-row--owned",
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

/** One app in the list grouped by app (or in the "Other processes" group): a
 *  single process as its own row, or a row with the app's count and totals
 *  that opens to its processes. */
function GroupRows(props: {
    model: TowerViewModel;
    group: ProcessGroup;
    cpu: (f: number | undefined) => string;
    /** Inside an owner group: one level in, and opened on its own. */
    inOwner?: boolean;
}) {
    const m = props.model;
    const key = () => `app:${props.inOwner ? "other:" : ""}${props.group.key}`;
    const open = () => m.expanded().has(key());
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
                    nested={props.inOwner}
                    taskLabel={label(props.group.processes[0].task)}
                />
            }
        >
            <tr
                class={clsx("tower-group-row", props.inOwner && "tower-group-row--nested")}
                data-testid={`tower-app-${props.group.key}`}
            >
                <td class="tower-name">
                    <div class="tower-name-line">
                        <IconButton
                            icon={open() ? "chevron-down" : "chevron-right"}
                            label={open() ? "Hide processes" : "Show processes"}
                            density="compact"
                            tooltip={false}
                            aria-expanded={open()}
                            onClick={() => m.toggleExpanded(key())}
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
                {(p) => (
                    <ProcessRow
                        model={m}
                        process={p}
                        cpu={props.cpu}
                        nested
                        deep={props.inOwner}
                        taskLabel={label(p.task)}
                    />
                )}
            </For>
        </Show>
    );
}

/** One owner in the list grouped by agent: a header in the agent's color
 *  with its count and totals, opening to its processes; the "Other
 *  processes" group opens to them grouped by app. While a filter is typed
 *  every group with a match is open, so each match shows under its owner. */
function OwnerRows(props: { model: TowerViewModel; group: OwnerGroup; cpu: (f: number | undefined) => string }) {
    const m = props.model;
    const key = () => `owner:${props.group.key}`;
    const open = () => m.filter().trim() !== "" || m.expanded().has(key());
    const color = () =>
        props.group.kind === "agent"
            ? agentColor(props.group.key, props.group.label, m.snapshot()?.remote ?? false)
            : undefined;
    const processes = createMemo(() =>
        open() && props.group.kind !== "other"
            ? sortProcesses(props.group.processes, m.sort()).slice(0, HOST_ROW_LIMIT)
            : []
    );
    const apps = createMemo(() =>
        open() && props.group.kind === "other"
            ? sortGroups(groupProcesses(props.group.processes), m.sort()).slice(0, HOST_ROW_LIMIT)
            : []
    );
    const appByKey = createMemo(() => new Map(apps().map((g) => [g.key, g])));
    return (
        <>
            <tr
                class={clsx("tower-owner-row", `tower-owner-row--${props.group.kind}`)}
                data-testid={`tower-owner-${props.group.key}`}
                style={color() ? { "--tower-owner-color": color() } : undefined}
            >
                <td class="tower-name">
                    <div class="tower-name-line">
                        <IconButton
                            icon={open() ? "chevron-down" : "chevron-right"}
                            label={
                                open()
                                    ? `Hide ${props.group.label}'s processes`
                                    : `Show ${props.group.label}'s processes`
                            }
                            density="compact"
                            tooltip={false}
                            aria-expanded={open()}
                            onClick={() => m.toggleExpanded(key())}
                        />
                        <span class="tower-label">{props.group.label}</span>
                        <span class="tower-muted">({props.group.processes.length})</span>
                    </div>
                </td>
                <td class="tower-num">{props.cpu(props.group.cpu)}</td>
                <td class="tower-num">{formatMem(props.group.mem)}</td>
                <td class="tower-num">{props.group.processes.length}</td>
            </tr>
            <For each={processes()}>
                {(p) => <ProcessRow model={m} process={p} cpu={props.cpu} nested ownerColor={color()} />}
            </For>
            <For each={apps().map((g) => g.key)}>
                {(appKey) => (
                    <Show when={appByKey().get(appKey)}>
                        {(g) => <GroupRows model={m} group={g()} cpu={props.cpu} inOwner />}
                    </Show>
                )}
            </For>
        </>
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
    // Grouped by app or by owner: rows keyed by the app's name or the owner,
    // so a group stays the same row (and stays open) across refreshes.
    const groups = createMemo(() => sortGroups(groupProcesses(rows()), m.sort()));
    const groupByKey = createMemo(() => new Map(groups().map((g) => [g.key, g])));
    const owners = createMemo(() => sortOwnerGroups(ownerGroups(rows(), m.snapshot()?.tasks ?? []), m.sort()));
    const ownerByKey = createMemo(() => new Map(owners().map((g) => [g.key, g])));
    const grouped = () => m.grouping() !== "none";
    const shown = () => (m.grouping() === "app" ? groups().length : m.grouping() === "none" ? rows().length : 0);
    // The first list grouped by agent opens its two busiest agents; after
    // that, what is open is the user's to choose.
    createEffect(() => {
        if (m.grouping() !== "agent" || m.ownersOpened) return;
        const agents = owners().filter((g) => g.kind === "agent");
        if (!agents.length) return;
        m.ownersOpened = true;
        const busiest = [...agents].sort((a, b) => (b.cpu ?? -1) - (a.cpu ?? -1)).slice(0, 2);
        m.open(busiest.map((g) => `owner:${g.key}`));
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
            <div class="tower-host-bar">
                <FilterInput
                    value={m.filter()}
                    onInput={(q) => m.setFilter(q)}
                    placeholder="Filter by name, PID or task"
                    class="tower-filter"
                    testId="tower-filter"
                />
                <SegmentedControl<ProcessGrouping>
                    density="compact"
                    ariaLabel="Group by"
                    options={[
                        ...(m.hasTasks()
                            ? [
                                  {
                                      value: "agent" as const,
                                      label: "Agent",
                                      title: "Under the agent or terminal that started each",
                                  },
                              ]
                            : []),
                        {
                            value: "app" as const,
                            label: "App",
                            title: "Processes of the same app together, as Task Manager does",
                        },
                        { value: "none" as const, label: "None", title: "One flat list" },
                    ]}
                    value={m.grouping()}
                    onChange={(v) => m.setGrouping(v)}
                />
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
                            label={grouped() ? "Count" : "PID"}
                            numeric
                            title={grouped() ? "Processes in the group, or the PID of one" : undefined}
                        />
                    </tr>
                </thead>
                <tbody>
                    <Switch>
                        <Match when={m.grouping() === "agent"}>
                            <For each={owners().map((g) => g.key)}>
                                {(key) => (
                                    <Show when={ownerByKey().get(key)}>
                                        {(group) => <OwnerRows model={m} group={group()} cpu={props.cpu} />}
                                    </Show>
                                )}
                            </For>
                        </Match>
                        <Match when={m.grouping() === "app"}>
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
                        </Match>
                        <Match when={m.grouping() === "none"}>
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
                        </Match>
                    </Switch>
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
