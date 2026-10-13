// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal, createEffect, onCleanup } from "solid-js";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";

export interface BlockStats {
    cpu: number; // percentage (0-100+)
    mem: number; // bytes
    /** The highest `mem` since AgentMux started watching this shell. */
    peak: number; // bytes
    pids: number;
}

export function useBlockStats(blockId: string): () => BlockStats | null {
    const [stats, setStats] = createSignal<BlockStats | null>(null);

    createEffect(() => {
        const unsub = onMuxEvent(WpsEvent.BlockStats, (data) => {
                if (data.values) {
                    setStats({
                        cpu: data.values.cpu ?? 0,
                        mem: data.values.mem ?? 0,
                        peak: data.values["mem:peak"] ?? data.values.mem ?? 0,
                        pids: data.values.pids ?? 0,
                    });
                }
            }, `block:${blockId}`);
        onCleanup(() => unsub());
    });

    return stats;
}
