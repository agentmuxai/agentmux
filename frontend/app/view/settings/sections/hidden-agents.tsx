// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Hidden from devices: the agents kept off paired devices (agentmux-mobile's
// SPEC_AGENT_STATUS_AND_LIVE_PANE_FEED_2026_10_07 §4.3). `viewer.agent-hidden`
// reads and writes the flag for one definition at a time, so the list is built
// by asking about each.

import { Button, Select } from "@/app/element/ui";
import { createSignal, For, onMount, Show, type JSX } from "solid-js";

import { RpcApi, type AgentDefinition } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { SettingsIndexEntry } from "../settings-model";
import { sortByLastUsed } from "@/app/view/agent/agent-recency";

export const HIDDEN_AGENTS_SETTING = {
    id: "devices.hidden_agents",
    label: "Hidden from devices",
    description:
        "Agents kept off paired devices: they are left out of a device's list and can't be watched from one. Add an agent to hide it, or remove it to show it again.",
    section: "devices",
    keywords: ["hide", "hidden", "agent", "device", "mobile", "paired", "private", "watch", "viewer"],
} satisfies SettingsIndexEntry;

const agentName = (agent: AgentDefinition) => agent.name || agent.slug || agent.id;

export function HiddenAgents(): JSX.Element {
    const [agents, setAgents] = createSignal<AgentDefinition[] | null>(null);
    const [hiddenIds, setHiddenIds] = createSignal<string[]>([]);
    const [pick, setPick] = createSignal("");
    const [busy, setBusy] = createSignal<string | null>(null);
    const [error, setError] = createSignal<string | null>(null);

    const load = async () => {
        try {
            const list = await RpcApi.ListAgentDefinitionsCommand(TabRpcClient, { is_seeded: 0 });
            const flags = await Promise.all(
                list.map((a) => RpcApi.ViewerAgentHiddenCommand(TabRpcClient, { agent_id: a.id }).then((r) => r.hidden))
            );
            setAgents(list);
            setHiddenIds(list.filter((_, i) => flags[i]).map((a) => a.id));
            setError(null);
        } catch (e) {
            setError(`Couldn't list hidden agents: ${e instanceof Error ? e.message : e}`);
        }
    };
    onMount(() => void load());

    const change = async (agent: AgentDefinition, hidden: boolean) => {
        setBusy(agent.id);
        try {
            const r = await RpcApi.ViewerAgentHiddenCommand(TabRpcClient, { agent_id: agent.id, hidden });
            setHiddenIds((ids) => [...ids.filter((id) => id !== agent.id), ...(r.hidden ? [agent.id] : [])]);
            setPick("");
            setError(null);
        } catch (e) {
            setError(`Couldn't change ${agentName(agent)}: ${e instanceof Error ? e.message : e}`);
        } finally {
            setBusy(null);
        }
    };

    const hiddenAgents = () => (agents() ?? []).filter((a) => hiddenIds().includes(a.id));
    const candidates = () => sortByLastUsed((agents() ?? []).filter((a) => !hiddenIds().includes(a.id)), agentName);
    const picked = () => candidates().find((a) => a.id === pick());

    return (
        <div id={`setting-${HIDDEN_AGENTS_SETTING.id}`} class="setting-row">
            <div class="setting-devices-description">{HIDDEN_AGENTS_SETTING.description}</div>
            <Show when={error()}>
                <div class="settings-config-error" role="alert">
                    {error()}
                </div>
            </Show>
            <Show
                when={agents()}
                fallback={
                    <Show when={!error()}>
                        <div class="setting-devices-empty">Loading…</div>
                    </Show>
                }
            >
                <Show
                    when={hiddenAgents().length > 0}
                    fallback={
                        <div class="setting-devices-empty">
                            No agent is hidden. Paired devices can watch every agent here.
                        </div>
                    }
                >
                    <ul class="setting-hidden-agents">
                        <For each={hiddenAgents()}>
                            {(agent) => (
                                <li class="setting-hidden-agent">
                                    <span class="setting-hidden-agent-name">{agentName(agent)}</span>
                                    <Button
                                        disabled={busy() === agent.id}
                                        aria-label={`Remove ${agentName(agent)} from the hidden list`}
                                        onClick={() => void change(agent, false)}
                                    >
                                        Remove
                                    </Button>
                                </li>
                            )}
                        </For>
                    </ul>
                </Show>
                <Show when={candidates().length > 0}>
                    <div class="setting-hidden-agents-add">
                        <Select
                            ariaLabel="Agent to hide from devices"
                            value={pick()}
                            onChange={setPick}
                            options={[
                                { value: "", label: "Choose an agent…" },
                                ...candidates().map((a) => ({ value: a.id, label: agentName(a) })),
                            ]}
                        />
                        <Button
                            disabled={!picked() || busy() !== null}
                            onClick={() => {
                                const agent = picked();
                                if (agent) void change(agent, true);
                            }}
                        >
                            Add agent
                        </Button>
                    </div>
                </Show>
            </Show>
        </div>
    );
}
