// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Swarm's agents that have no pane and no process: one row each, from the agent's definition
// (name, provider) and its runtime state (store/agent-runtime.ts). They are listed apart from the live
// agents' rows: those are keyed by block and carry a Supervisor, a selection and a stream of
// activity, none of which a stopped agent has. Start runs it with no pane and Open starts it in one.

import { Button } from "@/app/element/ui/Button";
import { MOS } from "@/app/store/global";
import { AgentRuntimeChip } from "@/app/view/agent/components/AgentRuntimeChip";
import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import { createSignal, For, Show, type Accessor, type JSX } from "solid-js";
import { LOCAL_PROVIDER_UNKNOWN, SwarmProviderIcon } from "./swarm-provider-icon";
import { HeldNote } from "./swarm-row-notes";

/** The meta of the block a stopped agent keeps its desired state and held input in, once it has one. */
function blockMeta(blockId: string | undefined): Record<string, any> | undefined {
    return blockId ? MOS.getMuxObjectAtom<Block>(MOS.makeORef("block", blockId))()?.meta : undefined;
}

export function StoppedAgents(props: {
    agents: Accessor<AgentRuntime[]>;
    /** Whether live agents are listed above: the list starts folded then, open when it is all there is. */
    hasLive: Accessor<boolean>;
    /** Resolves to why srv refused, shown under the row. */
    onOpen: (agent: AgentRuntime) => Promise<string | undefined> | void;
    /** Runs the agent with no pane; resolves to why srv refused, shown under the row. */
    onStart: (agent: AgentRuntime) => Promise<string | undefined> | void;
}): JSX.Element {
    const [folded, setFolded] = createSignal<boolean | null>(null);
    const [refused, setRefused] = createSignal<Record<string, string>>({});
    // The agents with a start or an open out: the row says so, and a second press waits for the first.
    const [busy, setBusy] = createSignal<ReadonlySet<string>>(new Set());
    const run = async (agent: AgentRuntime, action: (a: AgentRuntime) => Promise<string | undefined> | void) => {
        if (busy().has(agent.agent_id)) return;
        setRefused((prev) => ({ ...prev, [agent.agent_id]: "" }));
        setBusy((prev) => new Set(prev).add(agent.agent_id));
        try {
            const why = await action(agent);
            if (why) setRefused((prev) => ({ ...prev, [agent.agent_id]: why }));
        } finally {
            setBusy((prev) => {
                const next = new Set(prev);
                next.delete(agent.agent_id);
                return next;
            });
        }
    };
    const isFolded = () => folded() ?? props.hasLive();
    return (
        <Show when={props.agents().length > 0}>
            <section class="swarm-stopped" aria-label="Stopped agents">
                <Button
                    tone="quiet"
                    density="compact"
                    class="swarm-stopped__heading"
                    icon={isFolded() ? "chevron-right" : "chevron-down"}
                    aria-expanded={!isFolded()}
                    onClick={() => setFolded(!isFolded())}
                >
                    Stopped agents
                    <span class="swarm-stopped__count">{props.agents().length}</span>
                </Button>
                <Show when={!isFolded()}>
                    <For each={props.agents()}>
                        {(agent) => (
                            <>
                                {/* A click on the row is its Open button (a press on a control inside it is that control's own). */}
                                <div
                                    class="swarm-stopped__row"
                                    data-agent-id={agent.agent_id}
                                    role="button"
                                    tabIndex={0}
                                    title={`${agent.name}: click to open`}
                                    onClick={(e) => {
                                        if ((e.target as Element).closest("button")) return;
                                        void run(agent, props.onOpen);
                                    }}
                                    onKeyDown={(e) => {
                                        if (e.target !== e.currentTarget || (e.key !== "Enter" && e.key !== " ")) return;
                                        e.preventDefault();
                                        void run(agent, props.onOpen);
                                    }}
                                >
                                    <SwarmProviderIcon
                                        provider={agent.provider}
                                        unknownTitle={LOCAL_PROVIDER_UNKNOWN}
                                    />
                                    <span class="swarm-stopped__name" title={agent.name}>
                                        {agent.name}
                                    </span>
                                    <HeldNote meta={blockMeta(agent.block_id)} />
                                    <AgentRuntimeChip state={agent.state} detail={agent.detail} />
                                    <Button
                                        density="compact"
                                        aria-label={`Start ${agent.name}`}
                                        title="Run it with no pane, resuming its session"
                                        disabled={busy().has(agent.agent_id)}
                                        onClick={() => void run(agent, props.onStart)}
                                    >
                                        {busy().has(agent.agent_id) ? "Starting…" : "Start"}
                                    </Button>
                                    <Button
                                        density="compact"
                                        aria-label={`Open ${agent.name}`}
                                        title="Open it in a pane (starts it)"
                                        disabled={busy().has(agent.agent_id)}
                                        onClick={() => void run(agent, props.onOpen)}
                                    >
                                        Open
                                    </Button>
                                </div>
                                <Show when={refused()[agent.agent_id]}>
                                    {(why) => (
                                        <div class="swarm-stopped__error" role="alert">
                                            {why()}
                                        </div>
                                    )}
                                </Show>
                            </>
                        )}
                    </For>
                </Show>
            </section>
        </Show>
    );
}
