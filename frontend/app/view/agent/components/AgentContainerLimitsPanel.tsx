// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentContainerLimitsPanel: the "Container limits…" modal, opened from a container agent's pane menu and from the
 * "Raise the limit" button on an out-of-memory row. A container agent's CPU and memory limits, what is in force now
 * and what Docker has (docs/specs/SPEC_CONTAINER_AGENT_RESOURCE_LIMITS_2026_10_29.md §3). Saving puts the new limits
 * on the running container at once: nothing restarts.
 */

import { createMemo, createSignal, For, onMount, Show, type JSX } from "solid-js";

import { Button, Field, Select, TextInput } from "@/app/element/ui";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { AgentContainerLimitsResult } from "@/types/rpc/AgentContainerLimitsResult";
import {
    cpuDraft,
    cpuNotes,
    cpuStored,
    memoryDraft,
    memoryNotes,
    memoryStored,
    type LimitDraft,
    type LimitMode,
} from "../container-limits";
import { AgentContainerCaches } from "./AgentContainerCaches";
import "./AgentContainerLimitsPanel.scss";
import "./AgentPrimitiveModal.scss";

const errorText = (e: unknown): string => (e instanceof Error ? e.message : String(e));

interface AgentContainerLimitsPanelProps {
    agentId: string;
    agentName: string;
    /** Given when hosted in a modal: shows a Close button. */
    onClose?: () => void;
}

const MODES: { value: LimitMode; label: string }[] = [
    { value: "auto", label: "Automatic" },
    { value: "custom", label: "Set a limit" },
    { value: "none", label: "No limit" },
];

export const AgentContainerLimitsPanel = (props: AgentContainerLimitsPanelProps): JSX.Element => {
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

    const row = (
        label: string,
        draft: () => LimitDraft,
        set: (d: LimitDraft) => void,
        suffix: string,
        notes: () => string[],
        testid: string
    ): JSX.Element => (
        <Field label={label} layout="stacked">
            <div class="agent-container-limits-row">
                <Select
                    value={draft().mode}
                    disabled={limits() == null || busy()}
                    onChange={(v) => {
                        set({ ...draft(), mode: v as LimitMode });
                        setSaved(null);
                    }}
                    data-testid={`${testid}-mode`}
                >
                    <For each={MODES}>{(m) => <option value={m.value}>{m.label}</option>}</For>
                </Select>
                <Show when={draft().mode === "custom"}>
                    <TextInput
                        class="setting-number"
                        inputmode="decimal"
                        value={draft().amount}
                        disabled={busy()}
                        onInput={(e) => {
                            set({ ...draft(), amount: e.currentTarget.value });
                            setSaved(null);
                        }}
                        data-testid={`${testid}-amount`}
                    />
                    <span>{suffix}</span>
                </Show>
            </div>
            <For each={notes()}>
                {(note) => (
                    <p class="agent-primitive-modal-global-note" data-testid={`${testid}-note`}>
                        {note}
                    </p>
                )}
            </For>
        </Field>
    );

    return (
        <div class="agent-primitive-modal-detail agent-container-limits-panel" data-testid="agent-container-limits-panel">
            <div class="agent-primitive-modal-readonly">
                <p class="agent-primitive-modal-global-note">
                    How much of Docker's memory and CPU {props.agentName}'s container may use. A change reaches the
                    running container at once; nothing restarts.
                </p>
                {row(
                    "Memory",
                    memory,
                    setMemory,
                    "GB",
                    () => memoryNotes(memory(), limits()?.effective_memory_mb, limits()?.daemon_memory_mb),
                    "container-limits-memory"
                )}
                {row(
                    "CPUs",
                    cpus,
                    setCpus,
                    "CPUs",
                    () => cpuNotes(cpus(), limits()?.effective_cpus, limits()?.daemon_cpus),
                    "container-limits-cpus"
                )}
                <AgentContainerCaches agentId={props.agentId} agentName={props.agentName} />
                <Show when={problem()}>
                    <div class="agent-primitive-modal-error" role="alert" data-testid="container-limits-problem">
                        {problem()}
                    </div>
                </Show>
                <Show when={error()}>
                    <div class="agent-primitive-modal-error" role="alert">
                        {error()}
                    </div>
                </Show>
                <Show when={limits() == null && error()}>
                    <div class="agent-primitive-modal-actions">
                        <Button onClick={() => void load()}>Retry</Button>
                    </div>
                </Show>
                <Show when={saved()}>
                    <p class="agent-primitive-modal-global-note" role="status" data-testid="container-limits-saved">
                        {saved()}
                    </p>
                </Show>
                <div class="agent-primitive-modal-actions">
                    <Show when={props.onClose}>{(close) => <Button onClick={() => close()()}>Close</Button>}</Show>
                    <Button tone="accent" busy={busy()} disabled={!dirty() || busy()} onClick={() => void save()}>
                        Save
                    </Button>
                </div>
            </div>
        </div>
    );
};

AgentContainerLimitsPanel.displayName = "AgentContainerLimitsPanel";
