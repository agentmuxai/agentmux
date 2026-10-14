// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { usePaneTabVisibility } from "@/app/block/pane-tab-visibility";
import { useBlockStats, type BlockStats } from "@/app/hook/useBlockStats";
import { RpcApi } from "@/app/store/rpc-api";
import type {
    BlockStatsConnection,
    BlockStatsDetail,
    BlockStatsFigures,
    BlockStatsRemote,
} from "@/app/store/rpc-api/blockstats";
import { TabRpcClient } from "@/app/store/rpc-util";
import {
    Match,
    Show,
    Switch,
    createEffect,
    createMemo,
    createSignal,
    on,
    onCleanup,
    onMount,
    type Accessor,
} from "solid-js";
import { describeTermRenderer, termRendererOf } from "@/app/store/term-renderer";
import "./blockstats.scss";
import { Tooltip } from "./tooltip";

/**
 * How often an open tooltip asks for a local pane's processes again, and how
 * often a remote pane's badge asks its machine while the pane is visible.
 */
const DETAIL_INTERVAL_MS = 2000;

function formatMem(bytes: number): string {
    if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)}K`;
    if (bytes < 1024 * 1024 * 1024) return `${Math.round(bytes / (1024 * 1024))}M`;
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)}G`;
}

/** The tooltip's spelling of a size: "90 MB". */
export function formatBytes(bytes: number): string {
    if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
    if (bytes < 1024 * 1024 * 1024) return `${Math.round(bytes / (1024 * 1024))} MB`;
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

/** "pwsh, node ×2, git and 3 more (8 processes)". */
export function describeProcesses(figures: BlockStatsFigures): string {
    const shown = figures.names.slice(0, 3).map((n) => (n.count > 1 ? `${n.name} ×${n.count}` : n.name));
    const rest = figures.names.slice(3).reduce((sum, n) => sum + n.count, 0);
    const names = rest > 0 ? `${shown.join(", ")} and ${rest} more` : shown.join(", ");
    const count = figures.processes === 1 ? "1 process" : `${figures.processes} processes`;
    return `${names} (${count})`;
}

/** A start time: "13:20" today, "Oct 11, 13:20" before. */
export function formatSince(ms: number, now = new Date()): string {
    const at = new Date(ms);
    const time = at.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
    if (at.toDateString() === now.toDateString()) return time;
    return `${at.toLocaleDateString([], { month: "short", day: "numeric" })}, ${time}`;
}

const since = (f: BlockStatsFigures) =>
    f.started_at_ms != null ? `, running since ${formatSince(f.started_at_ms)}` : "";

/**
 * A remote pane's machine: "Connected to build (build.example.net, 203.0.113.5)",
 * "Connected to WSL Ubuntu (198.51.100.5)".
 */
export function describeConnection(c: BlockStatsConnection): string {
    const name = c.kind === "wsl" ? `WSL ${c.host ?? c.name.replace(/^wsl:\/\//, "")}` : c.name;
    // The host only when it says more than the name does (an alias's HostName).
    const destHost = c.name.replace(/^.*@/, "").replace(/:\d+$/, "");
    const host = c.kind === "ssh" && c.host && c.host !== destHost && c.host !== c.address ? c.host : undefined;
    const address = c.address !== destHost ? c.address : undefined;
    const where = [host, address].filter(Boolean).join(", ");
    return where ? `Connected to ${name} (${where})` : `Connected to ${name}`;
}

/**
 * Whether a pane frame shows the badge: as its pane type's setting says
 * (`settingOn`; `term:showstatsbadge` for a terminal), and never in a
 * drag-preview frame. Every pane renders one off screen, laid out and visible
 * to the visibility hooks, so its badge would be a second one, and a remote
 * pane's would ask the machine a second time on every poll.
 */
export function statsBadgeShown(preview: boolean, settingOn: boolean): boolean {
    return !preview && settingOn;
}

/** Whether a pane's `connection` is another machine (SSH, WSL). */
export function isRemoteConnection(connection: string | undefined): boolean {
    const c = (connection ?? "").trim();
    return c !== "" && c !== "local";
}

/** What "Private" means on the machine that measured it. */
function privateMeaning(metric: string): string {
    switch (metric) {
        case "private working set":
            return "in RAM and used only by these processes";
        case "memory footprint":
            return "what macOS counts as theirs, compressed and swapped included";
        default:
            return "their own memory, in RAM or swap";
    }
}

/**
 * What a remote host's CPU and memory rows cover: every process, or, where
 * the machine wouldn't measure some (other users' on macOS), only the rest.
 */
export function hostCoverage(h: { processes: number; unmeasured: number }): string {
    if (h.unmeasured === 0) return `all its ${h.processes} processes`;
    const mine = h.processes - h.unmeasured;
    return `your ${mine} processes only; ${h.unmeasured} others' can't be read`;
}

/** A note when some of a pane's processes weren't measured: its figures leave them out. */
export function partial(f: BlockStatsFigures): string {
    const n = f.unmeasured ?? 0;
    if (n === 0) return "";
    return ` ${n === 1 ? "1 of them runs" : `${n} of them run`} as another user and can't be read: the figures below leave ${n === 1 ? "it" : "them"} out.`;
}

/** The badge's two figures for a remote pane, and whether they are the whole host's. */
export function remoteBadge(r: BlockStatsRemote): { cpu: string; mem: string; host: boolean } | null {
    const cpu = (v: number | undefined) => (v == null ? "–%" : `${v.toFixed(1)}%`);
    const mem = (v: number | undefined) => (v == null ? "–" : formatMem(v));
    if (r.figures) return { cpu: cpu(r.figures.cpu), mem: mem(r.figures.mem_resident), host: false };
    if (r.host) return { cpu: cpu(r.host.cpu), mem: formatMem(r.host.mem), host: true };
    return null;
}

function Row(props: { label: string; value: string; meaning?: string }) {
    return (
        <>
            <span class="bst-label">{props.label}</span>
            <span class="bst-value">{props.value}</span>
            <span class="bst-meaning">{props.meaning ?? ""}</span>
        </>
    );
}

/** The terminal's renderer: WebGL, DOM, or WebGL with its context lost (store/term-renderer.ts). */
function RendererRow(props: { blockId: string }) {
    return (
        <Show when={termRendererOf(props.blockId)}>
            {(r) => (
                <Row label="Renderer" value={describeTermRenderer(r()).value} meaning={describeTermRenderer(r()).meaning} />
            )}
        </Show>
    );
}

/**
 * A local pane's tooltip. Mounted only while it is open, so it asks srv for
 * the processes' detail (a process snapshot, tens of milliseconds) only then.
 */
function LocalTooltip(props: { blockId: string; stats: BlockStats }) {
    const [detail, setDetail] = createSignal<BlockStatsFigures | null>(null);
    let timer: ReturnType<typeof setInterval> | undefined;
    let closed = false;
    const load = () =>
        RpcApi.BlockStatsDetailCommand(TabRpcClient, { block_id: props.blockId }).then(
            (d) => !closed && setDetail(d.local ?? null),
            () => !closed && setDetail(null)
        );
    onMount(() => {
        void load();
        timer = setInterval(() => void load(), DETAIL_INTERVAL_MS);
    });
    onCleanup(() => {
        closed = true;
        clearInterval(timer);
    });

    const windows = () => detail()?.memory_metric === "private working set";

    return (
        <div class="block-stats-tooltip">
            <div class="bst-title">
                <Show
                    when={detail()}
                    fallback={`This pane's processes (${props.stats.pids === 1 ? "1 process" : `${props.stats.pids} processes`})`}
                >
                    {(d) => `This pane's processes: ${describeProcesses(d())}${since(d())}`}
                </Show>
            </div>
            <div class="bst-grid">
                <Row
                    label="CPU"
                    value={`${props.stats.cpu.toFixed(1)}%`}
                    meaning="of one core; above 100% is more than one core"
                />
                <Row
                    label="Memory"
                    value={formatBytes(props.stats.mem)}
                    meaning={`in RAM now, including pages shared with other processes${windows() ? "; Windows trims it when the shell is idle" : ""}`}
                />
                <Show when={detail()?.mem_private != null}>
                    <Row
                        label="Private"
                        value={formatBytes(detail()!.mem_private!)}
                        meaning={privateMeaning(detail()!.memory_metric)}
                    />
                </Show>
                <Show when={detail()?.mem_commit != null}>
                    <Row
                        label="Committed"
                        value={formatBytes(detail()!.mem_commit!)}
                        meaning="what they have allocated, in RAM or not; the steadiest measure of growth"
                    />
                </Show>
                <Row label="Peak" value={formatBytes(props.stats.peak)} meaning="the highest Memory has been" />
                <RendererRow blockId={props.blockId} />
            </div>
            <div class="bst-footer">
                The shell and everything started in it. Updated every second; hide with term:showstatsbadge.
            </div>
        </div>
    );
}

/** A remote pane's tooltip: what its machine said on the badge's last ask. */
function RemoteTooltip(props: { blockId: string; detail: BlockStatsDetail }) {
    const remote = () => props.detail.remote;
    const figures = () => remote()?.figures;
    const host = () => remote()?.host;
    return (
        <div class="block-stats-tooltip">
            <div class="bst-title">
                {props.detail.connection ? describeConnection(props.detail.connection) : "A remote pane"}
            </div>
            <div class="bst-meaning bst-how">
                <Switch>
                    <Match when={figures() && remote()?.found === "session"}>
                        Measured on that machine: {describeProcesses(figures()!)}
                        {since(figures()!)}, the pane's durable session.{partial(figures()!)}
                    </Match>
                    <Match when={figures()}>
                        Measured on that machine: {describeProcesses(figures()!)}
                        {since(figures()!)}, found by the pane's id in their environment.{partial(figures()!)}
                    </Match>
                    <Match when={remote()?.found === "none"}>
                        That machine can't tell this pane's processes from the rest: these are its own totals. A durable
                        pane, or a server that accepts AgentMux's variables (AcceptEnv AGENTMUX_*), shows the pane's.
                    </Match>
                    <Match when={remote()?.found === "unavailable"}>
                        Not measured: {remote()?.reason ?? "the machine couldn't be asked"}.
                    </Match>
                </Switch>
            </div>
            <div class="bst-grid">
                <Show when={figures()}>
                    {(f) => (
                        <>
                            <Row
                                label="CPU"
                                value={f().cpu == null ? "–" : `${f().cpu!.toFixed(1)}%`}
                                meaning="of one core; above 100% is more than one core"
                            />
                            <Show when={f().mem_resident != null}>
                                <Row
                                    label="Memory"
                                    value={formatBytes(f().mem_resident!)}
                                    meaning="in RAM now, including pages shared with other processes"
                                />
                            </Show>
                            <Show when={f().mem_private != null}>
                                <Row
                                    label="Private"
                                    value={formatBytes(f().mem_private!)}
                                    meaning={privateMeaning(f().memory_metric)}
                                />
                            </Show>
                            <Show when={remote()?.peak != null}>
                                <Row
                                    label="Peak"
                                    value={formatBytes(remote()!.peak!)}
                                    meaning="the highest Memory has been"
                                />
                            </Show>
                        </>
                    )}
                </Show>
                <Show when={host()}>
                    {(h) => (
                        <>
                            <Row
                                label="Host CPU"
                                value={h().cpu == null ? "–" : `${h().cpu!.toFixed(1)}%`}
                                meaning={`of one core, across ${h().cpu_count} cores; ${hostCoverage(h())}`}
                            />
                            <Row
                                label="Host memory"
                                value={formatBytes(h().mem)}
                                meaning={`private memory of ${hostCoverage(h())}`}
                            />
                        </>
                    )}
                </Show>
                <RendererRow blockId={props.blockId} />
            </div>
            <div class="bst-footer">
                Updated every 2 seconds while the pane is visible; hide with term:showstatsbadge.
            </div>
        </div>
    );
}

/**
 * A remote pane's figures, asked of its machine every 2 seconds while the
 * pane can be seen, the first time included: its window tab is the displayed
 * one, it isn't a dormant kept-alive pane (both hide with `visibility`, so
 * layout can't tell: `usePaneTabVisibility`), and the window isn't hidden.
 */
function useRemoteStats(blockId: string): Accessor<BlockStatsDetail | null> {
    const [detail, setDetail] = createSignal<BlockStatsDetail | null>(null);
    const paneVisibility = usePaneTabVisibility(blockId);
    let closed = false;
    let busy = false;
    const visible = () => paneVisibility() === "active" && document.visibilityState !== "hidden";
    const load = () => {
        if (busy || !visible()) return;
        busy = true;
        RpcApi.BlockStatsDetailCommand(TabRpcClient, { block_id: blockId })
            .then(
                (d) => !closed && setDetail(d),
                () => {}
            )
            .finally(() => (busy = false));
    };
    onMount(() => {
        const timer = setInterval(load, DETAIL_INTERVAL_MS);
        onCleanup(() => clearInterval(timer));
    });
    // Ask at once on mount and whenever the pane comes into view.
    createEffect(on(paneVisibility, (v) => v === "active" && load()));
    onCleanup(() => (closed = true));
    return detail;
}

function RemoteBadge(props: { blockId: string; noHeader?: boolean }) {
    const detail = useRemoteStats(props.blockId);
    const shown = createMemo(() => {
        const r = detail()?.remote;
        return r ? remoteBadge(r) : null;
    });
    const cpuClass = () => {
        const f = detail()?.remote?.figures;
        if (f?.cpu == null) return "";
        return f.cpu > 90 ? "cpu-high" : f.cpu > 50 ? "cpu-medium" : "";
    };
    return (
        <>
            <Show when={detail()}>
                {(d) => (
                    <Tooltip
                        placement="bottom"
                        immediate
                        divClassName={`block-stats-badge remote ${shown()?.host ? "host" : ""} ${cpuClass()} ${props.noHeader ? "no-header" : ""}`}
                        content={<RemoteTooltip blockId={props.blockId} detail={d()} />}
                    >
                        <span class="stats-cpu">
                            {shown() ? (shown()!.host ? `host ${shown()!.cpu}` : shown()!.cpu) : "–"}
                        </span>
                        <Show when={shown()}>
                            <span class="stats-mem">{shown()!.mem}</span>
                        </Show>
                    </Tooltip>
                )}
            </Show>
        </>
    );
}

function LocalBadge(props: { blockId: string; noHeader?: boolean }) {
    const stats = useBlockStats(props.blockId);

    const cpuClass = createMemo(() => {
        const s = stats();
        if (!s) return "";
        if (s.cpu > 90) return "cpu-high";
        if (s.cpu > 50) return "cpu-medium";
        return "";
    });

    return (
        <Show when={stats()}>
            {(s) => (
                <Tooltip
                    placement="bottom"
                    immediate
                    divClassName={`block-stats-badge ${cpuClass()} ${props.noHeader ? "no-header" : ""}`}
                    content={<LocalTooltip blockId={props.blockId} stats={s()} />}
                >
                    <span class="stats-cpu">{s().cpu.toFixed(1)}%</span>
                    <span class="stats-mem">{formatMem(s().mem)}</span>
                </Tooltip>
            )}
        </Show>
    );
}

/**
 * A pane's CPU/memory badge: a terminal's, or an agent's. A local pane's figures
 * come from srv every second while a badge is shown, counted as Tower counts
 * them (srv `sampler::panes`); a remote pane's (SSH, WSL) from its machine,
 * asked while the pane is visible (SPEC_TERMINAL_STATS_BADGE_REMOTE_PANES_2026_10_12.md).
 */
export function BlockStatsBadge(props: { blockId: string; noHeader?: boolean; connection?: string }) {
    return (
        <Show
            when={isRemoteConnection(props.connection)}
            fallback={<LocalBadge blockId={props.blockId} noHeader={props.noHeader} />}
        >
            <RemoteBadge blockId={props.blockId} noHeader={props.noHeader} />
        </Show>
    );
}
