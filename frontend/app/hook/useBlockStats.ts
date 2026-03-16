// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { createSignal, createEffect, onCleanup } from "solid-js";
import { waveEventSubscribe } from "@/app/store/wps";

export interface BlockStats {
    cpu: number; // percentage (0-100+)
    mem: number; // bytes
}

export function useBlockStats(blockId: string): () => BlockStats | null {
    const [stats, setStats] = createSignal<BlockStats | null>(null);

    createEffect(() => {
        const unsub = waveEventSubscribe({
            eventType: "blockstats",
            scope: `block:${blockId}`,
            handler: (event: any) => {
                const data = event.data;
                if (data?.values) {
                    setStats({
                        cpu: data.values.cpu ?? 0,
                        mem: data.values.mem ?? 0,
                    });
                }
            },
        });
        onCleanup(() => unsub());
    });

    return stats;
}
