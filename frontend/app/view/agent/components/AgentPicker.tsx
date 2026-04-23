// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentPicker — shown when an agent pane has no agentId in block meta.
 * Lists available Forge definitions as cards; clicking a card opens
 * the Launch modal (name + runtime), which submits back through
 * `AgentViewModel.launchForgeAgent(agent, overrides)`.
 *
 * See docs/specs/SPEC_AGENT_DEFINITIONS_MODAL_2026_04_23.md.
 */

import { createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { waveEventSubscribe } from "@/app/store/wps";
import type { AgentViewModel } from "../agent-model";
import { AgentCard } from "./AgentCard";
import { AgentCardSettingsPanel } from "./AgentCardSettingsPanel";
import { AgentActionBar } from "./AgentActionBar";
import { AgentLaunchModal, type LaunchOverrides } from "./AgentLaunchModal";

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
    const [launchModalAgent, setLaunchModalAgent] = createSignal<ForgeAgent | null>(null);
    const agents = useForgeAgents();

    // Inline Forge-settings panel: which definition is expanded.
    // Session-local only — not persisted to block meta.
    const [expandedId, setExpandedId] = createSignal<string | null>(null);

    // Clicking the card opens the Launch modal. The actual launch
    // happens on modal submit.
    const handleSelect = (agent: ForgeAgent) => {
        setNodejsError(null);
        setLaunchModalAgent(agent);
    };

    const handleLaunchSubmit = async (overrides: LaunchOverrides) => {
        const agent = launchModalAgent();
        if (!agent) return;
        setLaunching(agent.id);
        try {
            await props.model.launchForgeAgent(agent, overrides);
            if (props.model.nodejsError) {
                setNodejsError(props.model.nodejsError);
                props.model.nodejsError = null;
            }
        } finally {
            setLaunching(null);
            setLaunchModalAgent(null);
        }
    };

    const openForgeFor = (agent: ForgeAgent) => {
        // Toggle the inline settings panel for this definition.
        setExpandedId((prev) => (prev === agent.id ? null : agent.id));
    };

    const closePanel = () => {
        setExpandedId(null);
    };

    /**
     * Delete a Forge definition after confirmation. The backend
     * `DeleteForgeAgent` RPC removes the row and emits a
     * `forgeagents:changed` event, which `useForgeAgents` re-fetches
     * on — so the list updates without manual refresh.
     */
    const handleDelete = async (agent: ForgeAgent) => {
        const ok = window.confirm(
            `Delete definition "${agent.name}"?\n\nThis removes it permanently. Any open panes running instances of it will stay connected until you close them.`
        );
        if (!ok) return;
        try {
            await RpcApi.DeleteForgeAgentCommand(TabRpcClient, { id: agent.id });
        } catch (e: any) {
            alert(`Delete failed: ${e?.message ?? String(e)}`);
        }
    };

    const busy = () => launching() !== null;

    return (
        <>
            <Show
                when={agents().length > 0}
                fallback={
                    <div class="agent-view">
                        <div class="agent-picker-empty">
                            <div class="agent-picker-empty-icon">{"\u2726"}</div>
                            <div class="agent-picker-empty-title">No definitions configured</div>
                            <div class="agent-picker-empty-desc">
                                Use the Forge pane to add your first definition.
                            </div>
                        </div>
                        <AgentActionBar />
                    </div>
                }
            >
                <div class="agent-view">
                    <div class="agent-picker">
                        <div class="agent-picker-list">
                            <For each={agents()}>
                                {(agent) => (
                                    <>
                                        <AgentCard
                                            agent={agent}
                                            launching={launching() === agent.id}
                                            disabled={busy()}
                                            onLaunch={handleSelect}
                                            onOpenForge={openForgeFor}
                                            onDelete={handleDelete}
                                        />
                                        <Show when={expandedId() === agent.id}>
                                            <AgentCardSettingsPanel
                                                blockId={props.model.blockId}
                                                nodeModel={props.model.nodeModel}
                                                agent={agent}
                                                initialTab="forge"
                                                onClose={closePanel}
                                            />
                                        </Show>
                                    </>
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
                    </div>
                    <AgentActionBar />
                </div>
            </Show>
            <Show when={launchModalAgent()}>
                {(agent) => (
                    <AgentLaunchModal
                        agent={agent()}
                        onCancel={() => setLaunchModalAgent(null)}
                        onSubmit={handleLaunchSubmit}
                    />
                )}
            </Show>
        </>
    );
};

AgentPicker.displayName = "AgentPicker";
