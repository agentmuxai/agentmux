// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Tower: what each agent runs and what it costs (the Agents view), and every
// process on the machine (the Processes view)
// (SPEC_TOWER_TASK_MANAGER_PANE_2026_10_08.md,
// SPEC_TOWER_AGENT_CENTRIC_VIEWS_2026_10_08.md). Read-only.

import type { SelectOption } from "@/app/element/ui";
import {
    Button,
    EmptyState,
    FilterInput,
    IconButton,
    SegmentedControl,
    Select,
    tabbedPaneLayout,
    tabPanelId,
    Tabs,
    TextInput,
} from "@/app/element/ui";
import type { TowerPeerInfo, TowerProcess, TowerSnapshot } from "@/app/store/rpc-api";
import type { RemoteRecord } from "@/app/store/rpc-api/remotes";
import clsx from "clsx";
import { createEffect, createMemo, createSignal, For, type JSX, Match, onCleanup, onMount, Show, Switch } from "solid-js";
import { agentColor, AgentsView } from "./tower-agents";
import { KindBreakdown, KindIcon, LoadCells } from "./tower-kind-ui";
import { type KindGroup, kindGroups, kindOf, KINDS, sortKindGroups } from "./tower-kinds";
import { MemoryView, seriesLabel } from "./tower-memory";
import type { TowerViewModel } from "./tower-model";
import { PanesView } from "./tower-panes";
import { ProcessName } from "./tower-process-name";
import { SortHeader } from "./tower-sort-header";
import { towerViewTabs } from "./tower-view-tabs";
import {
    count,
    filterProcesses,
    formatChange,
    formatClock,
    formatCpu,
    formatMem,
    groupProcesses,
    growthText,
    hourChange,
    type LoadLevel,
    loadLevel,
    type OwnerGroup,
    ownerGroups,
    ownerOf,
    processDetail,
    type ProcessGroup,
    type ProcessGrouping,
    sortGroups,
    sortOwnerGroups,
    sortProcesses,
    type TowerView as TowerViewKind,
} from "./tower-util";

import { tip } from "@/app/element/tip";
import "./tower-view.scss";

/** The process list shows this many rows; the filter narrows the rest. */
const HOST_ROW_LIMIT = 400;

/** The picker's choice that opens the pairing form rather than a machine. */
const PAIR_OPTION = "__pair__";

/** The machine picker: this computer, the AgentMux computers it is paired
 *  with, then every SSH host and WSL distribution AgentMux knows (the Remotes
 *  pane's list), and a way to pair another. The helper runs on Linux and
 *  macOS only, so other SSH hosts are listed but can't be picked. */
function machineOptions(current: string, remotes: RemoteRecord[], peers: TowerPeerInfo[] = []): SelectOption[] {
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
    const cpu = (fraction: number | undefined) => formatCpu(fraction, m.snapshot()?.cpu_count ?? 1);
    const [pairing, setPairing] = createSignal(false);
    const isPeer = () => m.connection().startsWith("peer:");
    const tabs = createMemo(() => towerViewTabs(m));
    // The view tabs are icons with tooltips while the pane is too narrow for their labels (as in a TabbedPane).
    let root: HTMLDivElement | undefined;
    const [paneWidth, setPaneWidth] = createSignal(Infinity);
    // From the width and the tab count both: choosing another machine changes the count without a resize.
    const iconTabs = () => tabbedPaneLayout(paneWidth(), tabs().length) === "top-icons";
    onMount(() => {
        if (!root || typeof ResizeObserver === "undefined") return;
        const observer = new ResizeObserver((entries) => {
            setPaneWidth(entries[entries.length - 1].contentRect.width);
        });
        observer.observe(root);
        onCleanup(() => observer.disconnect());
    });

    return (
        <div ref={root} class="tower-view" data-testid="tower-view" style={{ zoom: m.zoom() }}>
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
                    items={tabs()}
                    iconOnly={iconTabs()}
                    value={m.effectiveView()}
                    onChange={(v) => m.setView(v)}
                    idPrefix={idPrefix}
                    ariaLabel="Tower view"
                    density="compact"
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
            <Headline model={m} />
            <div id={tabPanelId(idPrefix)} role="tabpanel" class="tower-body">
                <Show when={m.snapshot()} fallback={<EmptyState loading>Measuring…</EmptyState>}>
                    {(snap) => (
                        <Switch>
                            <Match when={m.effectiveView() === "memory"}>
                                <MemoryView model={m} />
                            </Match>
                            <Match when={m.effectiveView() === "agents"}>
                                <AgentsView model={m} cpu={cpu} />
                            </Match>
                            <Match when={m.effectiveView() === "panes"}>
                                <PanesView model={m} cpu={cpu} />
                            </Match>
                            <Match when={m.effectiveView() === "processes"}>
                                <Show when={snap().host} fallback={<EmptyState loading>Measuring…</EmptyState>}>
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

/** AgentMux's memory, one line under the toolbar in every view and at every
 *  width: everything AgentMux started, added up (its own programs, the
 *  terminals, and each agent with what it runs). Private memory, so a shared
 *  library or a shared page is not counted twice
 *  (SPEC_TOWER_MEMORY_FIRST_AND_AGENTMUX_BREAKDOWN_2026_10_25.md §3.1, §6). */
function Headline(props: { model: TowerViewModel }) {
    const m = props.model;
    const total = createMemo(() => {
        const snap = m.snapshot();
        // This computer's own tasks only: another machine's are a capped list.
        if (!snap || snap.remote || snap.tasks.length === 0) return undefined;
        let mem = 0;
        let processes = 0;
        for (const t of snap.tasks) {
            mem += t.mem;
            processes += t.processes.length;
        }
        return { mem, processes, metric: snap.memory_metric };
    });
    // What the Memory view knows: the hour's change, and what grows unusually
    // (spec section 7.2, 7.3). The other views are not sent it.
    const inMemoryView = () => m.effectiveView() === "memory";
    const change = createMemo(() => (inMemoryView() ? hourChange(m.memoryHistory()) : null));
    const grows = createMemo(() => {
        const memory = inMemoryView() ? m.snapshot()?.memory : undefined;
        return memory ? memory.growth.map((g) => `${seriesLabel(g.key, memory)} ${growthText(g)}`) : [];
    });
    return (
        <Show when={total()}>
            {(t) => {
                const lvl = () => loadLevel("mem", "machine", m.memShare(t().mem));
                return (
                    <div
                        class="tower-headline"
                        data-testid="tower-headline"
                        {...tip(
                            `Memory that only AgentMux's processes use (${t().metric}), added up over everything AgentMux started: its own programs, the terminals, and each agent with its tools. Shared memory is not counted twice.`
                        )}
                    >
                        <span class="tower-headline-label">AgentMux</span>
                        <span class={clsx("tower-headline-mem tower-num", lvl() && `tower-level--${lvl()}`)}>
                            {formatMem(t().mem)}
                        </span>
                        <span class="tower-headline-note tower-muted">{count(t().processes, "process")}</span>
                        <Show when={change()}>
                            {(c) => (
                                <span
                                    class="tower-headline-change tower-num tower-muted"
                                    data-testid="tower-headline-change"
                                    {...tip(
                                        "How AgentMux's memory changed over the last hour (since srv began keeping it, if that was less than an hour ago)"
                                    )}
                                >
                                    {formatChange(c().change)}{" "}
                                    {c().since === null ? "in the last hour" : `since ${formatClock(c().since!)}`}
                                </span>
                            )}
                        </Show>
                        <Show when={grows().length > 0}>
                            <span
                                class="tower-headline-grow"
                                role="img"
                                data-testid="tower-headline-grow"
                                aria-label={`Grows unusually: ${grows().join("; ")}`}
                                {...tip(grows().join("; "))}
                            />
                        </Show>
                    </div>
                );
            }}
        </Show>
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

/** A process of an agent's task carries that agent's colour (its owner bar)
 *  in every list, not only under the agent's header. */
function ownerColorOf(m: TowerViewModel, p: TowerProcess): string | undefined {
    if (!p.task) return undefined;
    const t = m.snapshot()?.tasks.find((x) => x.id === p.task);
    return t?.kind === "agent" ? agentColor(t.id, t.label, m.snapshot()?.remote ?? false) : undefined;
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
    const owner = () => props.ownerColor ?? ownerColorOf(props.model, props.process);
    return (
        <tr
            style={owner() ? { "--tower-owner-color": owner() } : undefined}
            class={clsx(
                "tower-process-row",
                props.nested && "tower-process-row--nested",
                props.deep && "tower-process-row--deep",
                owner() && "tower-process-row--owned",
                props.single && "tower-process-row--single",
                props.process.role && `tower-role--${props.process.role}`,
                props.process.kind && `tower-kind--${props.process.kind}`
            )}
            {...tip(processDetail(props.process, props.model.snapshot()?.memory_metric ?? ""))}
        >
            <td class="tower-name">
                <div class="tower-name-line">
                    <ProcessName process={props.process}>
                        <Show when={props.taskLabel}>
                            {(label) => <span class="tower-badge tower-badge--task">{label()}</span>}
                        </Show>
                    </ProcessName>
                </div>
            </td>
            <LoadCells model={props.model} cpu={props.cpu} cpuValue={props.process.cpu} mem={props.process.mem} />
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
    /** Inside "Other processes": one level in, with its own open state, and
     *  open while a search is typed, so a match isn't hidden in it. */
    inOwner?: boolean;
}) {
    const m = props.model;
    const key = () => `app:${props.inOwner ? "other:" : ""}${props.group.key}`;
    const searching = () => !!props.inOwner && m.filter().trim() !== "";
    const open = () => searching() || m.expanded().has(key());
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
                class={clsx(
                    "tower-group-row",
                    props.inOwner && "tower-group-row--nested",
                    props.group.processes[0]?.kind && `tower-kind--${props.group.processes[0].kind}`
                )}
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
                            disabled={searching()}
                            onClick={() => m.toggleExpanded(key())}
                        />
                        <KindIcon kind={props.group.processes[0]?.kind} />
                        <span class="tower-label">{props.group.name}</span>
                        <span class="tower-muted">({props.group.processes.length})</span>
                        <Show when={label(props.group.task)}>
                            {(l) => <span class="tower-badge tower-badge--task">{l()}</span>}
                        </Show>
                    </div>
                </td>
                <LoadCells model={m} cpu={props.cpu} cpuValue={props.group.cpu} mem={props.group.mem} />
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
    // A search holds every group with a match open; its arrow can't close
    // it then, so it is disabled rather than doing nothing.
    const searching = () => m.filter().trim() !== "";
    const open = () => searching() || m.expanded().has(key());
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
                            disabled={searching()}
                            onClick={() => m.toggleExpanded(key())}
                        />
                        <span class="tower-label">{props.group.label}</span>
                        <span class="tower-muted">({props.group.processes.length})</span>
                    </div>
                </td>
                <LoadCells model={m} cpu={props.cpu} cpuValue={props.group.cpu} mem={props.group.mem} fill={color()} />
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

/** One kind of process: a header in the kind's colour with its count and
 *  totals, opening to its processes grouped by app (the list grouped by
 *  type, and System folded in the list by app), or one by one (System folded
 *  in the flat list). While a filter is typed it is open, so a match isn't
 *  hidden in it. */
function KindRows(props: {
    model: TowerViewModel;
    group: KindGroup;
    cpu: (f: number | undefined) => string;
    flat?: boolean;
}) {
    const m = props.model;
    const key = () => `kind:${props.group.kind}`;
    const searching = () => m.filter().trim() !== "";
    const open = () => searching() || m.expanded().has(key());
    const label = () => KINDS[props.group.kind].label;
    const processes = createMemo(() =>
        open() && props.flat ? sortProcesses(props.group.processes, m.sort()).slice(0, HOST_ROW_LIMIT) : []
    );
    const apps = createMemo(() =>
        open() && !props.flat
            ? sortGroups(groupProcesses(props.group.processes), m.sort()).slice(0, HOST_ROW_LIMIT)
            : []
    );
    const appByKey = createMemo(() => new Map(apps().map((g) => [g.key, g])));
    return (
        <>
            <tr
                class={clsx("tower-kind-row", `tower-kind--${props.group.kind}`)}
                data-testid={`tower-kind-${props.group.kind}`}
            >
                <td class="tower-name">
                    <div class="tower-name-line">
                        <IconButton
                            icon={open() ? "chevron-down" : "chevron-right"}
                            label={open() ? `Hide ${label()}` : `Show ${label()}`}
                            density="compact"
                            tooltip={false}
                            aria-expanded={open()}
                            disabled={searching()}
                            onClick={() => m.toggleExpanded(key())}
                        />
                        <KindIcon kind={props.group.kind} />
                        <span class="tower-label">{label()}</span>
                        <span class="tower-muted">({props.group.processes.length})</span>
                    </div>
                </td>
                <LoadCells model={m} cpu={props.cpu} cpuValue={props.group.cpu} mem={props.group.mem} />
                <td class="tower-num">{props.group.processes.length}</td>
            </tr>
            <For each={processes()}>
                {(p) => (
                    <ProcessRow
                        model={m}
                        process={p}
                        cpu={props.cpu}
                        nested
                        taskLabel={p.task ? m.taskLabel(p.task) : undefined}
                    />
                )}
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

interface HostTableProps {
    model: TowerViewModel;
    cpu: (f: number | undefined) => string;
}

/** What the Processes table works out from the machine's list: the rows after
 *  the filters, the same grouped each way, and the load levels of its totals. */
interface HostTableState {
    props: HostTableProps;
    host: () => TowerSnapshot["host"];
    partial: () => boolean;
    onlyLabel: () => string | undefined;
    listed: () => TowerProcess[];
    systemGroup: () => KindGroup | undefined;
    groups: () => ProcessGroup[];
    kinds: () => KindGroup[];
    owners: () => OwnerGroup[];
    kindByKey: () => Map<KindGroup["kind"], KindGroup>;
    groupByKey: () => Map<string, ProcessGroup>;
    ownerByKey: () => Map<string, OwnerGroup>;
    grouped: () => boolean;
    shown: () => number;
    cpuLevel: () => LoadLevel;
    memLevel: () => LoadLevel;
}

type TasksById = Parameters<typeof ownerOf>[1];

// Carried over from the Agents view: only that owner's processes.
function onlyLabelOf(m: TowerViewModel, host: TowerSnapshot["host"], tasksById: TasksById): string | undefined {
    const only = m.only();
    if (!only) return undefined;
    const p = (host?.processes ?? []).find((x) => ownerOf(x, tasksById)[0] === only);
    return p ? ownerOf(p, tasksById)[2] : (tasksById.get(only)?.label ?? "that owner");
}

function filteredRows(m: TowerViewModel, host: TowerSnapshot["host"], tasksById: TasksById): TowerProcess[] {
    const only = m.only();
    const kind = m.kindFilter();
    const all = (host?.processes ?? []).filter(
        (p) => (!only || ownerOf(p, tasksById)[0] === only) && (!kind || kindOf(p) === kind)
    );
    return sortProcesses(filterProcesses(all, m.filter(), m.taskLabel), m.sort());
}

function createHostTableState(props: HostTableProps): HostTableState {
    const m = props.model;
    const host = () => m.snapshot()?.host;
    const tasksById = createMemo(() => new Map((m.snapshot()?.tasks ?? []).map((t) => [t.id, t])));
    const onlyLabel = createMemo(() => onlyLabelOf(m, host(), tasksById()));
    const rows = createMemo(() => filteredRows(m, host(), tasksById()));
    // The system's processes are half a Windows list: in the list by app and
    // the flat list they start folded into one row (spec §3.3), unless that
    // kind is what was asked for.
    const foldSystem = () =>
        (m.grouping() === "app" || m.grouping() === "none") && !m.kindFilter() && rows().some((p) => p.kind);
    const listed = createMemo(() => (foldSystem() ? rows().filter((p) => p.kind !== "system") : rows()));
    const systemGroup = createMemo(() =>
        foldSystem() ? kindGroups(rows().filter((p) => p.kind === "system"))[0] : undefined
    );
    // Grouped by app, owner or kind: rows keyed by the app's name, the owner
    // or the kind, so a group stays the same row (and stays open) across
    // refreshes.
    const groups = createMemo(() => sortGroups(groupProcesses(listed()), m.sort()));
    const kinds = createMemo(() => sortKindGroups(kindGroups(rows()), m.sort()));
    const kindByKey = createMemo(() => new Map(kinds().map((g) => [g.kind, g])));
    const groupByKey = createMemo(() => new Map(groups().map((g) => [g.key, g])));
    const owners = createMemo(() => sortOwnerGroups(ownerGroups(rows(), m.snapshot()?.tasks ?? []), m.sort()));
    const ownerByKey = createMemo(() => new Map(owners().map((g) => [g.key, g])));
    const s: HostTableState = {
        props,
        host,
        /** Another machine sends only its busiest and largest processes. */
        partial: () => (host()?.processes.length ?? 0) < (host()?.matched ?? 0),
        onlyLabel,
        listed,
        systemGroup,
        groups,
        kinds,
        owners,
        kindByKey,
        groupByKey,
        ownerByKey,
        grouped: () => m.grouping() !== "none",
        shown: () => (m.grouping() === "app" ? groups().length : m.grouping() === "none" ? listed().length : 0),
        cpuLevel: () => loadLevel("cpu", "machine", m.cpuShare(host()?.cpu)),
        memLevel: () => loadLevel("mem", "machine", m.memShare(host()?.mem)),
    };
    createEffect(() => openBusiestOwners(s));
    createEffect(() => openBusiestKinds(s));
    return s;
}

// The first list grouped by agent whose agents have a CPU rate (the very
// first sample has none) opens its two busiest; after that, what is open
// is the user's to choose.
function openBusiestOwners(s: HostTableState): void {
    const m = s.props.model;
    if (m.grouping() !== "agent" || m.ownersOpened) return;
    const agents = s.owners().filter((g) => g.kind === "agent");
    if (!agents.some((g) => g.cpu != null)) return;
    m.ownersOpened = true;
    const busiest = [...agents].sort((a, b) => (b.cpu ?? -1) - (a.cpu ?? -1)).slice(0, 2);
    m.open(busiest.map((g) => `owner:${g.key}`));
}

// The same for the list grouped by type: its two busiest kinds, the
// system aside.
function openBusiestKinds(s: HostTableState): void {
    const m = s.props.model;
    if (m.grouping() !== "type" || m.kindsOpened) return;
    const work = s.kinds().filter((g) => g.kind !== "system");
    if (!work.some((g) => g.cpu != null)) return;
    m.kindsOpened = true;
    const busiest = [...work].sort((a, b) => (b.cpu ?? -1) - (a.cpu ?? -1)).slice(0, 2);
    m.open(busiest.map((g) => `kind:${g.kind}`));
}

function HostTable(props: HostTableProps) {
    const s = createHostTableState(props);
    return (
        <>
            <SummaryLine s={s} />
            <KindBreakdown model={props.model} cpu={props.cpu} />
            <HostBar s={s} />
            <ProcessTable s={s} />
            <Show when={s.shown() > HOST_ROW_LIMIT}>
                <div class="tower-muted tower-more">
                    {s.shown() - HOST_ROW_LIMIT} more not shown: filter to narrow the list.
                </div>
            </Show>
        </>
    );
}

function SummaryLine({ s }: { s: HostTableState }) {
    const host = s.host;
    return (
        <div class="tower-summary">
            {count(host()?.total ?? 0, "process")} · Memory{" "}
            <span class={clsx("tower-total tower-total--mem", s.memLevel() && `tower-level--${s.memLevel()}`)}>
                {formatMem(host()?.mem)}
            </span>{" "}
            · CPU{" "}
            <span class={clsx("tower-total tower-total--cpu", s.cpuLevel() && `tower-level--${s.cpuLevel()}`)}>
                {s.props.cpu(host()?.cpu)}
            </span>
            <Show when={s.partial()}>
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
    );
}

function HostBar({ s }: { s: HostTableState }) {
    const m = s.props.model;
    return (
        <div class="tower-host-bar">
            <Show when={m.kindFilter() || undefined}>
                {(kind) => (
                    <span class="tower-only" data-testid="tower-kind-only">
                        <KindIcon kind={kind()} />
                        Only {KINDS[kind()].label}
                        <Button density="compact" tone="quiet" onClick={() => m.setKindFilter("")}>
                            Show all
                        </Button>
                    </span>
                )}
            </Show>
            <Show when={s.onlyLabel()}>
                {(label) => (
                    <span class="tower-only" data-testid="tower-only">
                        Only {label()}
                        <Button density="compact" tone="quiet" onClick={() => m.showAll()}>
                            Show all
                        </Button>
                    </span>
                )}
            </Show>
            <FilterInput
                value={m.filter()}
                onInput={(q) => m.setFilter(q)}
                placeholder="Filter by name, PID or task"
                class="tower-filter"
                testId="tower-filter"
            />
            <GroupingControl model={m} />
        </div>
    );
}

function GroupingControl(props: { model: TowerViewModel }) {
    const m = props.model;
    return (
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
                {
                    value: "type" as const,
                    label: "Type",
                    title: "By kind of process: agents, builds, browsers, the system…",
                },
                { value: "none" as const, label: "None", title: "One flat list" },
            ]}
            value={m.grouping()}
            onChange={(v) => m.setGrouping(v)}
        />
    );
}

function ProcessTable({ s }: { s: HostTableState }) {
    const m = s.props.model;
    return (
        <table class="tower-table" aria-label="Processes">
            <thead>
                <tr>
                    <SortHeader model={m} key="name" label="Process" />
                    <SortHeader model={m} key="mem" label="Memory" numeric />
                    <SortHeader model={m} key="cpu" label="CPU" numeric />
                    <SortHeader
                        model={m}
                        key="count"
                        label={s.grouped() ? "Count" : "PID"}
                        numeric
                        title={s.grouped() ? "Processes in the group, or the PID of one" : undefined}
                    />
                </tr>
            </thead>
            <tbody>
                <Switch>
                    <Match when={m.grouping() === "agent"}>
                        <OwnerList s={s} />
                    </Match>
                    <Match when={m.grouping() === "app"}>
                        <AppList s={s} />
                    </Match>
                    <Match when={m.grouping() === "type"}>
                        <KindList s={s} />
                    </Match>
                    <Match when={m.grouping() === "none"}>
                        <FlatList s={s} />
                    </Match>
                </Switch>
                <Show when={s.systemGroup()}>
                    {(group) => <KindRows model={m} group={group()} cpu={s.props.cpu} flat={m.grouping() === "none"} />}
                </Show>
            </tbody>
        </table>
    );
}

function OwnerList({ s }: { s: HostTableState }) {
    return (
        <For each={s.owners().map((g) => g.key)}>
            {(key) => (
                <Show when={s.ownerByKey().get(key)}>
                    {(group) => <OwnerRows model={s.props.model} group={group()} cpu={s.props.cpu} />}
                </Show>
            )}
        </For>
    );
}

function AppList({ s }: { s: HostTableState }) {
    return (
        <For
            each={s
                .groups()
                .slice(0, HOST_ROW_LIMIT)
                .map((g) => g.key)}
        >
            {(key) => (
                <Show when={s.groupByKey().get(key)}>
                    {(group) => <GroupRows model={s.props.model} group={group()} cpu={s.props.cpu} />}
                </Show>
            )}
        </For>
    );
}

function KindList({ s }: { s: HostTableState }) {
    return (
        <For each={s.kinds().map((g) => g.kind)}>
            {(kind) => (
                <Show when={s.kindByKey().get(kind)}>
                    {(group) => <KindRows model={s.props.model} group={group()} cpu={s.props.cpu} />}
                </Show>
            )}
        </For>
    );
}

function FlatList({ s }: { s: HostTableState }) {
    const m = s.props.model;
    return (
        <For each={s.listed().slice(0, HOST_ROW_LIMIT)}>
            {(p) => (
                <ProcessRow
                    model={m}
                    process={p}
                    cpu={s.props.cpu}
                    taskLabel={p.task ? m.taskLabel(p.task) : undefined}
                />
            )}
        </For>
    );
}
