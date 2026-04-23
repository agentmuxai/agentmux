// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useProcessCount — reactive accessor for the number of OS processes
 * currently tracked for a given agent block.
 *
 * Drives the `⚙ N` badge on each agent pane's status line. Subscribes
 * to `agent:process-added` / `agent:process-exited` WPS events for the
 * block scope and keeps a local count. Also fetches the initial count
 * once on mount via `RpcApi.AgentProcessListCommand` so the badge
 * doesn't lag the true state when a pane re-opens with an already-
 * running agent.
 *
 * See `backend::process_tracker` + `agentmux-ai/AGENT_SPAWNED_PROCESSES_SPEC.md`.
 */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import * as WOS from "@/app/store/wos";
import { waveEventSubscribe } from "@/app/store/wps";
import { createSignal, onCleanup, onMount, type Accessor } from "solid-js";

export function useProcessCount(blockId: string): Accessor<number> {
    const [count, setCount] = createSignal(0);

    onMount(() => {
        // Order matters: subscribe to deltas BEFORE fetching the
        // snapshot. If we fetched first, events arriving during the
        // RPC round-trip would increment the count, then the snapshot
        // `setCount(res.processes.length)` would overwrite — losing
        // those deltas. Subscribe first, buffer deltas in a local
        // variable until the snapshot lands, then apply snapshot +
        // buffered deltas together.
        let seeded = false;
        let deltaSincePreSeed = 0;

        const unsubAdded = waveEventSubscribe({
            eventType: "agent:process-added",
            scope: WOS.makeORef("block", blockId),
            handler: () => {
                if (seeded) setCount((c) => c + 1);
                else deltaSincePreSeed += 1;
            },
        });
        const unsubExited = waveEventSubscribe({
            eventType: "agent:process-exited",
            scope: WOS.makeORef("block", blockId),
            handler: () => {
                if (seeded) setCount((c) => Math.max(0, c - 1));
                else deltaSincePreSeed -= 1;
            },
        });

        // Seed snapshot. Overlap between the snapshot and buffered
        // deltas might double-count by ±1 in an edge case; the next
        // backend poll tick (~2s) will re-align the count via the
        // same event stream. Worth the cheap correctness trade for
        // a tiny transient window.
        // Silently tolerate RPC failure — older backends without the
        // command fall back to delta-only mode and still converge.
        void RpcApi.AgentProcessListCommand(TabRpcClient, { block_id: blockId })
            .then((res) => {
                setCount(Math.max(0, res.processes.length + deltaSincePreSeed));
                seeded = true;
            })
            .catch(() => {
                setCount(Math.max(0, deltaSincePreSeed));
                seeded = true;
            });

        onCleanup(() => {
            unsubAdded?.();
            unsubExited?.();
        });
    });

    return count;
}
