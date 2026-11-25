// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { usePaneTabVisibility } from "@/app/block/pane-tab-visibility";
import { useBlockStats, type BlockStats } from "@/app/hook/useBlockStats";
import { t } from "@/app/i18n/i18n";
import { RpcApi } from "@/app/store/rpc-api";
import type {
    BlockStatsConnection,
    BlockStatsDetail,
    BlockStatsFigures,
    BlockStatsRemote,
} from "@/app/store/rpc-api/blockstats";
import { TabRpcClient } from "@/app/store/rpc-util";
import { formatBytes } from "@/util/format-bytes";
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

/** "pwsh, node ×2, git and 3 more (8 processes)". */
export function describeProcesses(figures: BlockStatsFigures): string {
    const shown = figures.names.slice(0, 3).map((n) => (n.count > 1 ? `${n.name} ×${n.count}` : n.name));
    const rest = figures.names.slice(3).reduce((sum, n) => sum + n.count, 0);
    const names = rest > 0 ? t("blockStats.andMore", { names: shown.join(", "), count: rest }) : shown.join(", ");
    return t("blockStats.describe", { names, processes: t("blockStats.processes", { count: figures.processes }) });
}

/** A start time: "13:20" today, "Oct 11, 13:20" before. */
export function formatSince(ms: number, now = new Date()): string {
    const at = new Date(ms);
    const time = at.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
    if (at.toDateString() === now.toDateString()) return time;
    return t("blockStats.dateTime", { date: at.toLocaleDateString([], { month: "short", day: "numeric" }), time });
}

const since = (f: BlockStatsFigures) =>
    f.started_at_ms != null ? t("blockStats.runningSince", { time: formatSince(f.started_at_ms) }) : "";

/**
 * A remote pane's machine: "Connected to build (build.example.net, 203.0.113.5)",
 * "Connected to WSL Ubuntu (198.51.100.5)".
 */
export function describeConnection(c: BlockStatsConnection): string {
    const name = c.kind === "wsl" ? t("blockStats.wsl", { host: c.host ?? c.name.replace(/^wsl:\/\//, "") }) : c.name;
    // The host only when it says more than the name does (an alias's HostName).
    const destHost = c.name.replace(/^.*@/, "").replace(/:\d+$/, "");
    const host = c.kind === "ssh" && c.host && c.host !== destHost && c.host !== c.address ? c.host : undefined;
    const address = c.address !== destHost ? c.address : undefined;
    const where = [host, address].filter(Boolean).join(", ");
    return where ? t("blockStats.connectedWhere", { name, where }) : t("blockStats.connected", { name });
}

/**
 * Whether a pane frame shows the badge: as its pane type's setting says
 * (`settingOn`; `term:showstatsbadge` for a terminal), and never in a
 * drag-preview frame. Every pane renders one off screen, laid out and visible
 * to the visibility hooks, so its badge would be a second one, and a remote
 * pane's would ask the machine a second time on every poll. Nor where the
 * pane draws the readout itself (`drawnByPane`: the capability `statsBadgeIn`).
 */
export function statsBadgeShown(preview: boolean, settingOn: boolean, drawnByPane = false): boolean {
    return !preview && settingOn && !drawnByPane;
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
            return t("blockStats.private.windows");
        case "memory footprint":
            return t("blockStats.private.mac");
        default:
            return t("blockStats.private.generic");
    }
}

/**
 * What a remote host's CPU and memory rows cover: every process, or, where
 * the machine wouldn't measure some (other users' on macOS), only the rest.
 */
export function hostCoverage(h: { processes: number; unmeasured: number }): string {
    if (h.unmeasured === 0) return t("blockStats.coverage.all", { count: h.processes });
    const mine = h.processes - h.unmeasured;
    return t("blockStats.coverage.mine", { count: mine, others: h.unmeasured });
}

/** A note when some of a pane's processes weren't measured: its figures leave them out. */
export function partial(f: BlockStatsFigures): string {
    const n = f.unmeasured ?? 0;
    if (n === 0) return "";
    return ` ${t("blockStats.partial", { count: n })}`;
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
                <Row label={t("blockStats.row.renderer")} value={describeTermRenderer(r()).value} meaning={describeTermRenderer(r()).meaning} />
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
                    fallback={t("blockStats.local.titleCount", { processes: t("blockStats.processes", { count: props.stats.pids }) })}
                >
                    {(d) => t("blockStats.local.title", { described: describeProcesses(d()), since: since(d()) })}
                </Show>
            </div>
            <div class="bst-grid">
                <Row
                    label={t("blockStats.row.cpu")}
                    value={`${props.stats.cpu.toFixed(1)}%`}
                    meaning={t("blockStats.meaning.cpu")}
                />
                <Row
                    label={t("blockStats.row.memory")}
                    value={formatBytes(props.stats.mem)}
                    meaning={windows() ? t("blockStats.meaning.memoryWindows") : t("blockStats.meaning.memory")}
                />
                <Show when={detail()?.mem_private != null}>
                    <Row
                        label={t("blockStats.row.private")}
                        value={formatBytes(detail()!.mem_private!)}
                        meaning={privateMeaning(detail()!.memory_metric)}
                    />
                </Show>
                <Show when={detail()?.mem_commit != null}>
                    <Row
                        label={t("blockStats.row.committed")}
                        value={formatBytes(detail()!.mem_commit!)}
                        meaning={t("blockStats.meaning.committed")}
                    />
                </Show>
                <Row label={t("blockStats.row.peak")} value={formatBytes(props.stats.peak)} meaning={t("blockStats.meaning.peak")} />
                <RendererRow blockId={props.blockId} />
            </div>
            <div class="bst-footer">
                {t("blockStats.local.footer")}
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
                {props.detail.connection ? describeConnection(props.detail.connection) : t("blockStats.remote.title")}
            </div>
            <div class="bst-meaning bst-how">
                <Switch>
                    <Match when={figures() && remote()?.found === "session"}>
                        {t("blockStats.remote.session", { described: describeProcesses(figures()!), since: since(figures()!), partial: partial(figures()!) })}
                    </Match>
                    <Match when={figures()}>
                        {t("blockStats.remote.found", { described: describeProcesses(figures()!), since: since(figures()!), partial: partial(figures()!) })}
                    </Match>
                    <Match when={remote()?.found === "none"}>
                        {t("blockStats.remote.none")}
                    </Match>
                    <Match when={remote()?.found === "unavailable"}>
                        {t("blockStats.remote.unavailable", { reason: remote()?.reason ?? t("blockStats.remote.noAnswer") })}
                    </Match>
                </Switch>
            </div>
            <div class="bst-grid">
                <Show when={figures()}>
                    {(f) => (
                        <>
                            <Row
                                label={t("blockStats.row.cpu")}
                                value={f().cpu == null ? "–" : `${f().cpu!.toFixed(1)}%`}
                                meaning={t("blockStats.meaning.cpu")}
                            />
                            <Show when={f().mem_resident != null}>
                                <Row
                                    label={t("blockStats.row.memory")}
                                    value={formatBytes(f().mem_resident!)}
                                    meaning={t("blockStats.meaning.memory")}
                                />
                            </Show>
                            <Show when={f().mem_private != null}>
                                <Row
                                    label={t("blockStats.row.private")}
                                    value={formatBytes(f().mem_private!)}
                                    meaning={privateMeaning(f().memory_metric)}
                                />
                            </Show>
                            <Show when={remote()?.peak != null}>
                                <Row
                                    label={t("blockStats.row.peak")}
                                    value={formatBytes(remote()!.peak!)}
                                    meaning={t("blockStats.meaning.peak")}
                                />
                            </Show>
                        </>
                    )}
                </Show>
                <Show when={host()}>
                    {(h) => (
                        <>
                            <Row
                                label={t("blockStats.row.hostCpu")}
                                value={h().cpu == null ? "–" : `${h().cpu!.toFixed(1)}%`}
                                meaning={t("blockStats.meaning.hostCpu", { count: h().cpu_count, coverage: hostCoverage(h()) })}
                            />
                            <Row
                                label={t("blockStats.row.hostMemory")}
                                value={formatBytes(h().mem)}
                                meaning={t("blockStats.meaning.hostMemory", { coverage: hostCoverage(h()) })}
                            />
                        </>
                    )}
                </Show>
                <RendererRow blockId={props.blockId} />
            </div>
            <div class="bst-footer">
                {t("blockStats.remote.footer")}
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
                            {shown() ? (shown()!.host ? t("blockStats.hostCpu", { cpu: shown()!.cpu }) : shown()!.cpu) : "–"}
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

/** How the readout is mounted: over the pane's corner (the block frame), or in the flow, as a slot of the agent pane's composer strip. */
export type StatsPlacement = "overlay" | "inline";

/** What an inline badge shows: all of it, or a processor icon and the CPU figure (the full reading is the tooltip). */
export type StatsForm = "full" | "compact";

type LocalBadgeProps = {
    blockId: string;
    noHeader?: boolean;
    placement?: StatsPlacement;
    form?: StatsForm;
    /** A hidden copy that is only measured: no tooltip, not focusable. */
    measuring?: boolean;
};

/** The figures, or in the compact form the icon and the CPU figure. */
function StatsFigures(props: { stats: BlockStats; form: StatsForm }) {
    return (
        <Show
            when={props.form === "compact"}
            fallback={
                <>
                    <span class="stats-cpu">{props.stats.cpu.toFixed(1)}%</span>
                    <span class="stats-mem">{formatMem(props.stats.mem)}</span>
                </>
            }
        >
            <i class="fa-solid fa-microchip stats-icon" aria-hidden="true" />
            <span class="stats-cpu">{props.stats.cpu.toFixed(1)}%</span>
        </Show>
    );
}

/**
 * An inline badge is focusable, and its tooltip stays open while it has focus (the tooltip opens on hover
 * only): Escape closes it. The compact form has no text for a screen reader beyond the figure, so it carries the
 * whole reading as its name.
 */
function InlineChip(props: {
    stats: BlockStats;
    form: StatsForm;
    setPinned: (v: boolean) => void;
    setDismissed: (v: boolean) => void;
}) {
    const reading = () => t("blockStats.reading", { cpu: props.stats.cpu.toFixed(1), mem: formatMem(props.stats.mem) });
    return (
        <span
            class="stats-reading"
            tabindex="0"
            role={props.form === "compact" ? "img" : undefined}
            aria-label={props.form === "compact" ? reading() : undefined}
            onFocus={() => {
                props.setDismissed(false);
                props.setPinned(true);
            }}
            onBlur={() => {
                props.setDismissed(false);
                props.setPinned(false);
            }}
            onMouseLeave={() => props.setDismissed(false)}
            // Escape closes it even while the pointer is still over the badge (hover alone would keep it open):
            // the tooltip stays shut until the pointer leaves or the chip is focused again.
            onKeyDown={(e) => {
                if (e.key !== "Escape") return;
                props.setPinned(false);
                props.setDismissed(true);
            }}
        >
            <StatsFigures stats={props.stats} form={props.form} />
        </span>
    );
}

function LocalBadge(props: LocalBadgeProps) {
    const stats = useBlockStats(props.blockId);
    const [pinned, setPinned] = createSignal(false);
    const [dismissed, setDismissed] = createSignal(false);
    const inline = () => props.placement === "inline";
    const form = () => props.form ?? "full";

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
                    passive
                    forceOpen={inline() && !props.measuring && pinned()}
                    disable={props.measuring || dismissed()}
                    divClassName={`block-stats-badge ${inline() ? `inline ${form()}` : ""} ${cpuClass()} ${props.noHeader ? "no-header" : ""}`}
                    content={<LocalTooltip blockId={props.blockId} stats={s()} />}
                >
                    <Show when={inline() && !props.measuring} fallback={<StatsFigures stats={s()} form={form()} />}>
                        <InlineChip stats={s()} form={form()} setPinned={setPinned} setDismissed={setDismissed} />
                    </Show>
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
 *
 * `placement` is `overlay` (the default: the block frame draws it over the pane's corner) or `inline` (a slot of
 * an agent pane's composer strip, in the flow, with a compact `form` for a tight strip). Inline is for a local pane.
 */
export function BlockStatsBadge(props: {
    blockId: string;
    noHeader?: boolean;
    connection?: string;
    placement?: StatsPlacement;
    form?: StatsForm;
    measuring?: boolean;
}) {
    return (
        <Show
            when={isRemoteConnection(props.connection)}
            fallback={
                <LocalBadge
                    blockId={props.blockId}
                    noHeader={props.noHeader}
                    placement={props.placement}
                    form={props.form}
                    measuring={props.measuring}
                />
            }
        >
            <RemoteBadge blockId={props.blockId} noHeader={props.noHeader} />
        </Show>
    );
}
