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
        // Seed the count from the current tracker state so the badge
        // reflects reality at mount time (important when re-opening a
        // pane with an already-running agent). Silently tolerate
        // failure — older backends won't have the RPC and that's fine.
        void RpcApi.AgentProcessListCommand(TabRpcClient, { block_id: blockId })
            .then((res) => setCount(res.processes.length))
            .catch(() => {});

        // Incremental updates via the delta events the backend emits
        // every ~2s from its poller.
        const unsubAdded = waveEventSubscribe({
            eventType: "agent:process-added",
            scope: WOS.makeORef("block", blockId),
            handler: () => setCount((c) => c + 1),
        });
        const unsubExited = waveEventSubscribe({
            eventType: "agent:process-exited",
            scope: WOS.makeORef("block", blockId),
            handler: () => setCount((c) => Math.max(0, c - 1)),
        });

        onCleanup(() => {
            unsubAdded?.();
            unsubExited?.();
        });
    });

    return count;
}
