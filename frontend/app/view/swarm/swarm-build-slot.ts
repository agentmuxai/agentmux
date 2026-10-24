// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Who holds this machine's build slot, for the Swarm row note
 * (docs/specs/SPEC_BUILD_SLOT_ENFORCEMENT_2026_10_20.md §6). One lock per machine, so one answer
 * for every row: `GetBuildSlotCommand` returns the holder (and the block of the local agent that
 * matches it), or null. The poll runs only while a row note is on screen.
 */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { BuildSlotView } from "@/types/rpc/BuildSlotView";
import { createSignal, onCleanup, onMount, type Accessor } from "solid-js";

/** How often the holder is asked for while a row note is mounted. */
export const BUILD_SLOT_POLL_MS = 15_000;

const sameHolder = (a: BuildSlotView | null, b: BuildSlotView | null): boolean =>
    a === b ||
    (a !== null && b !== null && a.agent === b.agent && a.block_id === b.block_id && a.since_ms === b.since_ms && a.reason === b.reason && a.command === b.command);

const [holder, setHolder] = createSignal<BuildSlotView | null>(null, { equals: sameHolder });
let subscribers = 0;
let timer: ReturnType<typeof setInterval> | undefined;

async function refresh(): Promise<void> {
    try {
        setHolder((await RpcApi.GetBuildSlotCommand(TabRpcClient, {})) ?? null);
    } catch {
        // Information only: keep showing what we had until the next poll.
    }
}

/** The current holder, polled while the calling component is mounted. */
export function useBuildSlotHolder(): Accessor<BuildSlotView | null> {
    onMount(() => {
        if (subscribers++ === 0) {
            void refresh();
            timer = setInterval(() => void refresh(), BUILD_SLOT_POLL_MS);
        }
    });
    onCleanup(() => {
        if (--subscribers === 0) {
            if (timer !== undefined) clearInterval(timer);
            timer = undefined;
            setHolder(null);
        }
    });
    return holder;
}

/** Whole minutes since the holder took the slot, 0 under a minute. */
export function holdMinutes(sinceMs: number, now: number): number {
    return Math.max(0, Math.floor((now - sinceMs) / 60_000));
}

/** `Holding the build slot · 12 min · cargo test`; the minutes appear from the first full one. */
export function buildSlotText(holderView: Pick<BuildSlotView, "since_ms" | "reason">, now: number): string {
    const minutes = holdMinutes(holderView.since_ms, now);
    const parts = ["Holding the build slot"];
    if (minutes >= 1) parts.push(`${minutes} min`);
    const reason = holderView.reason.trim().replace(/\s+/g, " ");
    if (reason) parts.push(reason.length > 24 ? `${reason.slice(0, 23)}…` : reason);
    return parts.join(" · ");
}

/** The tooltip: the whole command, and what the slot is. */
export function buildSlotTitle(holderView: Pick<BuildSlotView, "command" | "reason">): string {
    const what = holderView.command.trim() || holderView.reason.trim();
    return `${what ? `Running ${what}. ` : ""}Other builds and tests on this computer wait for this one to finish.`;
}
