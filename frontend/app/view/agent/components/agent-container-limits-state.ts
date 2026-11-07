// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A container agent's limits, the state: the limits srv has, the two drafts being edited, and the load and save calls.

import { createMemo, createSignal, onMount } from "solid-js";

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { AgentContainerLimitsResult } from "@/types/rpc/AgentContainerLimitsResult";
import { cpuDraft, cpuStored, memoryDraft, memoryStored, type LimitDraft } from "../container-limits";

const errorText = (e: unknown): string => (e instanceof Error ? e.message : String(e));

export interface AgentContainerLimitsPanelProps {
    agentId: string;
    agentName: string;
    /** Given when hosted in a modal: shows a Close button. */
    onClose?: () => void;
}

export type ContainerLimitsState = ReturnType<typeof createContainerLimits>;

export function createContainerLimits(props: AgentContainerLimitsPanelProps) {
    const [limits, setLimits] = createSignal<AgentContainerLimitsResult | null>(null);
    const [memory, setMemory] = createSignal<LimitDraft>({ mode: "auto", amount: "" });
    const [cpus, setCpus] = createSignal<LimitDraft>({ mode: "auto", amount: "" });
    const [busy, setBusy] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);
    const [saved, setSaved] = createSignal<string | null>(null);

    const load = async (): Promise<void> => {
        try {
            const result = await RpcApi.GetContainerLimitsCommand(TabRpcClient, { agent_id: props.agentId });
            setLimits(result);
            setMemory(memoryDraft(result.memory_mb));
            setCpus(cpuDraft(result.cpus));
            setError(null);
        } catch (e) {
            setError(errorText(e));
        }
    };
    onMount(() => void load());

    const memoryResult = createMemo(() => memoryStored(memory()));
    const cpuResult = createMemo(() => cpuStored(cpus()));
    const problem = () => {
        const [m, c] = [memoryResult(), cpuResult()];
        return "error" in m ? m.error : "error" in c ? c.error : null;
    };
    const dirty = () => {
        const current = limits();
        const [m, c] = [memoryResult(), cpuResult()];
        return current != null && "value" in m && "value" in c && (m.value !== current.memory_mb || c.value !== current.cpus);
    };

    const save = async (): Promise<void> => {
        const [m, c] = [memoryResult(), cpuResult()];
        if ("error" in m || "error" in c) return;
        setBusy(true);
        setError(null);
        setSaved(null);
        try {
            const result = await RpcApi.SetContainerLimitsCommand(TabRpcClient, {
                agent_id: props.agentId,
                memory_mb: m.value,
                cpus: c.value,
            });
            setLimits(result);
            setSaved(
                result.applied === false
                    ? "Saved, but Docker did not take the new limits. The container keeps the ones it has."
                    : result.applied === true
                      ? "Saved. The running container has the new limits now."
                      : "Saved. The container takes them when it starts."
            );
        } catch (e) {
            setError(errorText(e));
        } finally {
            setBusy(false);
        }
    };

    return { limits, memory, setMemory, cpus, setCpus, busy, error, saved, setSaved, load, problem, dirty, save };
}
