// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Show, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { useBlockStats, type BlockStats } from "@/app/hook/useBlockStats";
import { RpcApi } from "@/app/store/rpc-api";
import type { BlockStatsDetail } from "@/app/store/rpc-api/blockstats";
import { TabRpcClient } from "@/app/store/rpc-util";
import { Tooltip } from "./tooltip";
import "./blockstats.scss";

/** How often an open tooltip asks for its processes again. */
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
export function describeProcesses(detail: BlockStatsDetail): string {
    const shown = detail.names.slice(0, 3).map((n) => (n.count > 1 ? `${n.name} ×${n.count}` : n.name));
    const rest = detail.names.slice(3).reduce((sum, n) => sum + n.count, 0);
    const names = rest > 0 ? `${shown.join(", ")} and ${rest} more` : shown.join(", ");
    const count = detail.processes === 1 ? "1 process" : `${detail.processes} processes`;
    return `${names} (${count})`;
}

/** A start time: "13:20" today, "Oct 11, 13:20" before. */
export function formatSince(ms: number, now = new Date()): string {
    const at = new Date(ms);
    const time = at.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
    if (at.toDateString() === now.toDateString()) return time;
    return `${at.toLocaleDateString([], { month: "short", day: "numeric" })}, ${time}`;
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

function Row(props: { label: string; value: string; meaning?: string }) {
    return (
        <>
            <span class="bst-label">{props.label}</span>
            <span class="bst-value">{props.value}</span>
            <span class="bst-meaning">{props.meaning ?? ""}</span>
        </>
    );
}

/**
 * The badge's tooltip. Mounted only while it is open, so it asks srv for the
 * processes' detail (a process snapshot, tens of milliseconds) only then.
 */
function BlockStatsTooltip(props: { blockId: string; stats: BlockStats }) {
    const [detail, setDetail] = createSignal<BlockStatsDetail | null>(null);
    let timer: ReturnType<typeof setInterval> | undefined;
    let closed = false;
    const load = () =>
        RpcApi.BlockStatsDetailCommand(TabRpcClient, { block_id: props.blockId }).then(
            (d) => !closed && setDetail(d),
            () => !closed && setDetail(null),
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
                    {(d) => (
                        <>
                            This pane's processes: {describeProcesses(d())}
                            <Show when={d().started_at_ms}>{(at) => <>, running since {formatSince(at())}</>}</Show>
                        </>
                    )}
                </Show>
            </div>
            <div class="bst-grid">
                <Row label="CPU" value={`${props.stats.cpu.toFixed(1)}%`} meaning="of one core; above 100% is more than one core" />
                <Row
                    label="Memory"
                    value={formatBytes(props.stats.mem)}
                    meaning={`in RAM now, including pages shared with other processes${windows() ? "; Windows trims it when the shell is idle" : ""}`}
                />
                <Show when={detail()?.mem_private != null}>
                    <Row label="Private" value={formatBytes(detail()!.mem_private!)} meaning={privateMeaning(detail()!.memory_metric)} />
                </Show>
                <Show when={detail()?.mem_commit != null}>
                    <Row
                        label="Committed"
                        value={formatBytes(detail()!.mem_commit!)}
                        meaning="what they have allocated, in RAM or not; the steadiest measure of growth"
                    />
                </Show>
                <Row label="Peak" value={formatBytes(props.stats.peak)} meaning="the highest Memory has been" />
            </div>
            <div class="bst-footer">The shell and everything started in it. Updated every second; hide with term:showstatsbadge.</div>
        </div>
    );
}

export function BlockStatsBadge(props: { blockId: string; noHeader?: boolean }) {
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
                    divClassName={`block-stats-badge ${cpuClass()} ${props.noHeader ? "no-header" : ""}`}
                    content={<BlockStatsTooltip blockId={props.blockId} stats={s()} />}
                >
                    <span class="stats-cpu">{s().cpu.toFixed(1)}%</span>
                    <span class="stats-mem">{formatMem(s().mem)}</span>
                </Tooltip>
            )}
        </Show>
    );
}
