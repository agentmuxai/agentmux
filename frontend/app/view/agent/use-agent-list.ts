// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal, onCleanup, onMount } from "solid-js";

import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi, type AgentDefinition } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

const errorText = (e: unknown): string => (e instanceof Error ? e.message : String(e));

/**
 * Loads this machine's agents (`listagents`) now and again on every
 * `agents:changed`, until the returned function is called.
 */
export function watchAgentList(
    onList: (list: AgentDefinition[]) => void,
    onError?: (text: string) => void
): () => void {
    let stopped = false;
    const load = async () => {
        try {
            const list = await RpcApi.ListAgentDefinitionsCommand(TabRpcClient);
            if (!stopped) onList(list ?? []);
        } catch (e) {
            if (!stopped) onError?.(errorText(e));
        }
    };
    void load();
    const unsub = onMuxEvent(WpsEvent.AgentsChanged, () => void load());
    return () => {
        stopped = true;
        unsub();
    };
}

/**
 * This machine's agents, for a component: what the Seat tab picks a lead
 * from (docs/specs/SPEC_AGENT_SEATS_IN_THE_UI_2026_10_23.md §3). `undefined`
 * until the first list arrives.
 */
export function useAgentList(): [() => AgentDefinition[] | undefined, () => string | null] {
    const [agents, setAgents] = createSignal<AgentDefinition[] | undefined>(undefined);
    const [error, setError] = createSignal<string | null>(null);
    onMount(() => {
        const stop = watchAgentList((list) => {
            setAgents(list);
            setError(null);
        }, setError);
        onCleanup(stop);
    });
    return [agents, error];
}
