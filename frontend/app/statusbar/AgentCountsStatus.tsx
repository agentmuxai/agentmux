// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { runtimeCounts, type AgentRuntimeCounts } from "@/app/store/agent-runtime";
import { sharedAgentRuntime } from "@/app/store/agent-runtime-live";
import { openOrFocusPaneByView } from "@/app/store/block-component-registry";
import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import { createMemo, Show, type Accessor, type JSX } from "solid-js";

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

export function agentCountsTip(c: AgentRuntimeCounts): string {
    const running = `${plural(c.running, "agent is", "agents are")} running`;
    return c.waiting > 0
        ? `${running}, ${c.waiting} waiting on you. Click to open the Swarm`
        : `${running}. Click to open the Swarm`;
}

/**
 * How many agents have a process, and how many of those wait on the user. Closing a pane no longer
 * stops an agent, so this is where a running one that costs shows. Click opens the Swarm. Nothing
 * while no agent runs.
 */
export function AgentCountsStatus(props: { rows?: Accessor<AgentRuntime[]> }): JSX.Element {
    const rows = () => (props.rows ?? sharedAgentRuntime().rows)();
    const counts = createMemo(() => runtimeCounts(rows()));
    return (
        <Show when={counts().running > 0}>
            <div
                class="status-bar-item clickable status-agent-counts"
                onClick={() => void openOrFocusPaneByView("swarm")}
                data-tip={agentCountsTip(counts())}
                aria-label={agentCountsTip(counts())}
                role="button"
            >
                <i class="fa-solid fa-robot status-icon" />
                <span class="status-agent-counts__running">{counts().running} running</span>
                <Show when={counts().waiting > 0}>
                    <span class="status-agent-counts__waiting" style={{ color: "var(--warning-text-color)" }}>
                        {counts().waiting} waiting on you
                    </span>
                </Show>
            </div>
        </Show>
    );
}

AgentCountsStatus.displayName = "AgentCountsStatus";
