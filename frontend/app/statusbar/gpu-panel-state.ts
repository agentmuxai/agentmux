// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The GPU panel's state: which readout it is open from (the "GPU n%" meter or the "GFX HW" item: one panel, opened
 * from either), and its sample, asked of srv (`gpu.sample`) only while the panel is open. srv reads the counters on
 * its own thread, and says how often to ask again: two seconds, or longer when a read cost too much.
 */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { GpuSampleResult } from "@/types/rpc/GpuSampleResult";
import { createSignal, onCleanup, onMount, type Accessor } from "solid-js";

const [anchor, setAnchor] = createSignal<HTMLElement | null>(null);

/** The readout the panel is open from, or null when it is closed. */
export const gpuPanelAnchor: Accessor<HTMLElement | null> = anchor;

/** Open the panel from `el`, or close it if it is open from there; from the other readout it moves. */
export function toggleGpuPanel(el: HTMLElement): void {
    setAnchor(anchor() === el ? null : el);
}

export function closeGpuPanel(): void {
    setAnchor(null);
}

/** The shortest wait between two samples, whatever srv says. */
export const MIN_POLL_MS = 2000;
/** The wait after a sample that was only the first of two (the counters need two reads to mean anything). */
export const WARMING_POLL_MS = 1200;
/** The wait after a failed call. */
export const RETRY_POLL_MS = 5000;

/** How long to wait before asking again. */
export function pollDelay(last: GpuSampleResult | null): number {
    if (last == null) return RETRY_POLL_MS;
    if (last.state === "warming") return WARMING_POLL_MS;
    return Math.max(MIN_POLL_MS, last.interval_ms);
}

export type GpuSampleState = {
    sample: Accessor<GpuSampleResult | null>;
    /** The last call failed (srv is restarting, or gone). */
    failed: Accessor<boolean>;
};

/** Ask for a sample now and then, for as long as the owner is mounted: the panel's lifetime. */
export function createGpuSample(): GpuSampleState {
    const [sample, setSample] = createSignal<GpuSampleResult | null>(null);
    const [failed, setFailed] = createSignal(false);
    let timer: ReturnType<typeof setTimeout> | undefined;
    let stopped = false;

    const ask = async (): Promise<void> => {
        let delay = RETRY_POLL_MS;
        // A window nobody can see doesn't need a sample: ask again when it is shown.
        if (document.visibilityState === "visible") {
            try {
                const result = await RpcApi.GpuSampleCommand(TabRpcClient, {});
                if (stopped) return;
                setSample(result);
                setFailed(false);
                delay = pollDelay(result);
            } catch {
                if (stopped) return;
                setFailed(true);
            }
        }
        if (!stopped) timer = setTimeout(ask, delay);
    };

    onMount(() => void ask());
    onCleanup(() => {
        stopped = true;
        clearTimeout(timer);
    });
    return { sample, failed };
}
