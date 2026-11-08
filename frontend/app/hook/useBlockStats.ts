// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal, onCleanup, type Accessor } from "solid-js";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";

export interface BlockStats {
    cpu: number; // percentage (0-100+)
    mem: number; // bytes
    /** The highest `mem` since AgentMux started watching this shell. */
    peak: number; // bytes
    pids: number;
}

/** One subscription per pane, and the last figures it heard, shared by every reader of the pane's readout. */
type SharedStats = { stats: Accessor<BlockStats | null>; readers: number; stop: () => void };
const shared = new Map<string, SharedStats>();

function openShared(blockId: string): SharedStats {
    const [stats, setStats] = createSignal<BlockStats | null>(null);
    const stop = onMuxEvent(
        WpsEvent.BlockStats,
        (data) => {
            if (data.values) {
                setStats({
                    cpu: data.values.cpu ?? 0,
                    mem: data.values.mem ?? 0,
                    peak: data.values["mem:peak"] ?? data.values.mem ?? 0,
                    pids: data.values.pids ?? 0,
                });
            }
        },
        `block:${blockId}`
    );
    return { stats, readers: 0, stop };
}

/**
 * A pane's live figures. Every reader of one pane shares a subscription and its last figures, so a reader that
 * mounts later (the top band mounts a copy of the readout for each form it measures or draws, and again whenever
 * the form changes) has them at once, not after the next event a second away. The subscription ends with the
 * last reader.
 */
export function useBlockStats(blockId: string): () => BlockStats | null {
    let entry = shared.get(blockId);
    if (!entry) {
        entry = openShared(blockId);
        shared.set(blockId, entry);
    }
    const mine = entry;
    mine.readers++;
    onCleanup(() => {
        if (--mine.readers > 0) return;
        mine.stop();
        if (shared.get(blockId) === mine) shared.delete(blockId);
    });
    return mine.stats;
}
