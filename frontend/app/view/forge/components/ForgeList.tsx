// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { JSX } from "solid-js";
import { For, Show } from "solid-js";
import type { ForgeViewModel } from "../forge-model";
import { ForgeAgentCard } from "./ForgeAgentCard";

export function ForgeList(props: { model: ForgeViewModel }): JSX.Element {
    const agents = props.model.agentsAtom;

    const hostAgents = () => agents().filter((a) => a.agent_type === "host");
    const containerAgents = () => agents().filter((a) => a.agent_type === "container");
    const customAgents = () => agents().filter((a) => a.agent_type !== "host" && a.agent_type !== "container");

    return (
        <div class="forge-pane">
            <div class="forge-header">
                <span class="forge-title">Forge</span>
                <button class="forge-new-btn forge-new-btn-primary" onClick={() => props.model.startCreate()}>
                    + New Agent
                </button>
            </div>
            <div class="forge-divider" />
            <Show when={agents().length > 0} fallback={
                <div class="forge-empty">
                    <span class="forge-empty-icon">&#10022;</span>
                    <span class="forge-empty-label">No agents yet</span>
                    <span class="forge-empty-sub">Create your first agent</span>
                </div>
            }>
                <div class="forge-list">
                    <Show when={hostAgents().length > 0}>
                        <div class="forge-group-header">Host Agents</div>
                        <For each={hostAgents()}>{(agent) =>
                            <ForgeAgentCard agent={agent} model={props.model} />
                        }</For>
                    </Show>
                    <Show when={containerAgents().length > 0}>
                        <div class="forge-group-header">Container Agents</div>
                        <For each={containerAgents()}>{(agent) =>
                            <ForgeAgentCard agent={agent} model={props.model} />
                        }</For>
                    </Show>
                    <Show when={customAgents().length > 0}>
                        <div class="forge-group-header">Custom Agents</div>
                        <For each={customAgents()}>{(agent) =>
                            <ForgeAgentCard agent={agent} model={props.model} />
                        }</For>
                    </Show>
                </div>
            </Show>
        </div>
    );
}
