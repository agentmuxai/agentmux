// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Swarm's agents that have no pane and no process: one row each, from the agent's definition
// (name, provider) and its runtime state (swarm-runtime.ts). They are listed apart from the live
// agents' rows: those are keyed by block and carry a Supervisor, a selection and a stream of
// activity, none of which a stopped agent has. Open starts the agent in a pane, as opening it does.

import { Button } from "@/app/element/ui/Button";
import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import { createSignal, For, Show, type Accessor, type JSX } from "solid-js";
import { LOCAL_PROVIDER_UNKNOWN, SwarmProviderIcon } from "./swarm-provider-icon";

export function StoppedAgents(props: {
    agents: Accessor<AgentRuntime[]>;
    /** Whether live agents are listed above: the list starts folded then, open when it is all there is. */
    hasLive: Accessor<boolean>;
    onOpen: (agent: AgentRuntime) => void;
}): JSX.Element {
    const [folded, setFolded] = createSignal<boolean | null>(null);
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
                            <div class="swarm-stopped__row" data-agent-id={agent.agent_id}>
                                <SwarmProviderIcon provider={agent.provider} unknownTitle={LOCAL_PROVIDER_UNKNOWN} />
                                <span class="swarm-stopped__name" title={agent.name}>
                                    {agent.name}
                                </span>
                                <span class={`swarm-stopped__state swarm-stopped__state--${agent.state}`}>
                                    {agent.state}
                                </span>
                                <Button
                                    density="compact"
                                    aria-label={`Open ${agent.name}`}
                                    title="Open it in a pane (starts it)"
                                    onClick={() => props.onOpen(agent)}
                                >
                                    Open
                                </Button>
                            </div>
                        )}
                    </For>
                </Show>
            </section>
        </Show>
    );
}
