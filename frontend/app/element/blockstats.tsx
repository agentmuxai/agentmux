// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { Show, createMemo } from "solid-js";
import { useBlockStats } from "@/app/hook/useBlockStats";
import "./blockstats.scss";

function formatMem(bytes: number): string {
    if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)}K`;
    if (bytes < 1024 * 1024 * 1024) return `${Math.round(bytes / (1024 * 1024))}M`;
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)}G`;
}

export function BlockStatsBadge(props: { blockId: string }) {
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
                <div class={`block-stats-badge ${cpuClass()}`}>
                    <span class="stats-cpu">{s().cpu.toFixed(1)}%</span>
                    <span class="stats-mem">{formatMem(s().mem)}</span>
                </div>
            )}
        </Show>
    );
}
