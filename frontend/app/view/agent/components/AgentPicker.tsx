// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentPicker — shown when an agent pane has no agentId in block meta.
 * Lists available Forge agents and launches the selected one into the
 * calling AgentViewModel.
 *
 * Extracted from agent-view.tsx as Step 1 of
 * specs/SPEC_AGENT_VIEW_MODULARIZATION_2026_04_13.md.
 */

import { createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import { waveEventSubscribe } from "@/app/store/wps";
import type { AgentViewModel } from "../agent-model";

// ── useForgeAgents hook ───────────────────────────────────────────────────────

/**
 * Reactive accessor for the current Forge agent list. Subscribes to
 * `forgeagents:changed` and refetches when that event fires.
 */
export function useForgeAgents(): () => ForgeAgent[] {
    const [agents, setAgents] = createSignal<ForgeAgent[]>([]);

    onMount(() => {
        let cancelled = false;

        async function load() {
            try {
                const result = await RpcApi.ListForgeAgentsCommand(TabRpcClient);
                if (!cancelled) setAgents(result ?? []);
            } catch {
                // silently ignore
            }
        }

        load();

        const unsub = waveEventSubscribe({
            eventType: "forgeagents:changed",
            handler: () => load(),
        });

        onCleanup(() => {
            cancelled = true;
            unsub();
        });
    });

    return agents;
}

// ── AgentPicker component ───────────────────────────────────────────────────────

interface AgentPickerProps {
    model: AgentViewModel;
}

export const AgentPicker = (props: AgentPickerProps): JSX.Element => {
    const [launching, setLaunching] = createSignal<string | null>(null);
    const [nodejsError, setNodejsError] = createSignal<string | null>(null);
    const agents = useForgeAgents();

    const handleSelect = async (agent: ForgeAgent) => {
        setNodejsError(null);
        setLaunching(agent.id);
        try {
            await props.model.launchForgeAgent(agent);
            // Check if launch was blocked by missing Node.js
            if (props.model.nodejsError) {
                setNodejsError(props.model.nodejsError);
                props.model.nodejsError = null;
            }
        } catch {
            // model logs internally
        } finally {
            setLaunching(null);
        }
    };

    const busy = () => launching() !== null;

    return (
        <Show
            when={agents().length > 0}
            fallback={
                <div class="agent-view">
                    <div class="agent-picker-empty">
                        <div class="agent-picker-empty-icon">{"\u2726"}</div>
                        <div class="agent-picker-empty-title">No agents configured</div>
                        <div class="agent-picker-empty-desc">Create an agent in the Forge to get started.</div>
                        <button class="agent-picker-forge-btn" disabled>
                            + Create an agent in the Forge
                        </button>
                    </div>
                </div>
            }
        >
            <div class="agent-view">
                <div class="agent-picker">
                    <div class="agent-picker-list">
                        <For each={agents()}>
                            {(agent) => (
                                <button
                                    class={`agent-card${launching() === agent.id ? " agent-card--launching" : ""}`}
                                    onClick={() => handleSelect(agent)}
                                    disabled={busy()}
                                >
                                    <span class="agent-card-icon">{agent.icon}</span>
                                    <span class="agent-card-info">
                                        <span class="agent-card-name">{agent.name}</span>
                                        <Show when={agent.description}>
                                            <span class="agent-card-desc">{agent.description}</span>
                                        </Show>
                                    </span>
                                    <Show when={launching() === agent.id}>
                                        <span class="agent-card-spinner" />
                                    </Show>
                                </button>
                            )}
                        </For>
                    </div>
                    <Show when={nodejsError()}>
                        <div class="agent-nodejs-notice">
                            <div class="nodejs-notice-icon">
                                <i class="fa-solid fa-circle-exclamation" />
                            </div>
                            <div class="nodejs-notice-content">
                                <div class="nodejs-notice-title">Node.js Required</div>
                                <div class="nodejs-notice-text">{nodejsError()}</div>
                                <div class="nodejs-notice-hint">
                                    After installing, restart AgentMux and try again.
                                </div>
                            </div>
                        </div>
                    </Show>
                    <div class="agent-picker-footer">
                        <button class="agent-picker-forge-btn" disabled>
                            + New agent in Forge
                        </button>
                    </div>
                </div>
            </div>
        </Show>
    );
};

AgentPicker.displayName = "AgentPicker";
