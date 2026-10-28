// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentStoppedBanner: an agent that was stopped (`agent.stop`, or a stop from the Swarm) keeps its
 * pane and its conversation, and the pane says so, with a Start button that runs it again from
 * where it left off. The agent's own state, not the pane's: it is `agent:desired` on the block
 * (`SPEC_ALWAYS_AVAILABLE_AGENTS_2026_10_27.md` §3.3 rule 5), which a start sets back to `running`,
 * so the banner goes away when the start lands.
 */

import { createSignal, Show, type JSX } from "solid-js";
import { Button } from "@/app/element/ui";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

/** The block meta that says whether the agent should be running (`running` or `stopped`). */
export const DESIRED_META_KEY = "agent:desired";

type BlockAccessor = () => { meta?: Record<string, unknown> } | undefined;

/** True when the agent was explicitly stopped and has not been started since. */
export const isAgentStopped = (block: BlockAccessor): boolean => block()?.meta?.[DESIRED_META_KEY] === "stopped";

interface AgentStoppedBannerProps {
    block: BlockAccessor;
    /** The agent definition's id, which `agent.start` takes. */
    agentId: string;
    /** Starts the agent; resolves to why srv refused, or nothing. Defaults to `agent.start`. */
    start?: (agentId: string) => Promise<string | undefined>;
}

const startOnSrv = async (agentId: string): Promise<string | undefined> => {
    try {
        await RpcApi.AgentStartCommand(TabRpcClient, { agent_id: agentId });
        return undefined;
    } catch (e) {
        return e instanceof Error ? e.message : String(e);
    }
};

export const AgentStoppedBanner = (props: AgentStoppedBannerProps): JSX.Element => {
    const [busy, setBusy] = createSignal(false);
    const [refused, setRefused] = createSignal<string | undefined>(undefined);

    const onStart = async () => {
        if (busy()) return;
        setBusy(true);
        setRefused(undefined);
        try {
            setRefused(await (props.start ?? startOnSrv)(props.agentId));
        } finally {
            setBusy(false);
        }
    };

    return (
        <Show when={isAgentStopped(props.block) && props.agentId}>
            <div class="agent-stopped-banner" role="status">
                <span class="agent-stopped-label">
                    This agent is stopped. Its conversation is kept; Start runs it again from where it left off.
                </span>
                <Show when={refused()}>
                    <span class="agent-stopped-refused" role="alert">
                        {refused()}
                    </span>
                </Show>
                <Button
                    tone="accent"
                    density="compact"
                    busy={busy()}
                    onClick={() => void onStart()}
                    title="Start this agent and resume its session"
                >
                    Start
                </Button>
            </div>
        </Show>
    );
};

AgentStoppedBanner.displayName = "AgentStoppedBanner";
