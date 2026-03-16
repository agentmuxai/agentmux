// Copyright 2024, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { createMemo, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import type { AgentViewModel } from "./agent-model";
import { getProvider } from "./providers";
import { createAgentAtoms } from "./state";
import { useAgentStream } from "./useAgentStream";
import { AgentDocumentView } from "./components/AgentDocumentView";
import { AgentFooter } from "./components/AgentFooter";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import { waveEventSubscribe } from "@/app/store/wps";
import "./agent-view.scss";

// ── useForgeAgents hook ───────────────────────────────────────────────────────

function useForgeAgents(): () => ForgeAgent[] {
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

/**
 * Top-level wrapper — switches between agent picker and presentation view.
 */
export const AgentViewWrapper = ({ model }: { model: AgentViewModel }): JSX.Element => {
    const block = model.blockAtom;
    const agentId = () => block()?.meta?.["agentId"];

    return (
        <Show
            when={agentId()}
            fallback={<AgentPicker model={model} />}
        >
            <AgentPresentationView model={model} agentId={agentId()} />
        </Show>
    );
};

AgentViewWrapper.displayName = "AgentViewWrapper";

// ── Agent Picker ────────────────────────────────────────────────────────────────

const AgentPicker = ({ model }: { model: AgentViewModel }): JSX.Element => {
    const [launching, setLaunching] = createSignal<string | null>(null);
    const agents = useForgeAgents();

    const handleSelect = async (agent: ForgeAgent) => {
        setLaunching(agent.id);
        try {
            await model.launchForgeAgent(agent);
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

// ── Presentation View ───────────────────────────────────────────────────────────

const AgentPresentationView = ({ model, agentId }: { model: AgentViewModel; agentId: string }): JSX.Element => {
    const block = model.blockAtom;
    // agentProvider stores the provider id (claude/codex/gemini) — set when launching
    const providerKey = (): string => block()?.meta?.["agentProvider"] ?? agentId;
    const provider = () => getProvider(providerKey());
    const outputFormat = (): string => block()?.meta?.["agentOutputFormat"] ?? "claude-stream-json";

    // Create per-instance signals (stable across re-renders via createMemo keyed on blockId)
    const agentAtoms = createMemo(() => createAgentAtoms(model.blockId));

    // Subscribe to subprocess output and parse into DocumentNodes
    useAgentStream({
        blockId: model.blockId,
        outputFormat: outputFormat(),
        documentAtom: agentAtoms().documentAtom,
        streamingStateAtom: agentAtoms().streamingStateAtom,
        enabled: true,
    });

    // Send user message — spawns a new subprocess turn (or resumes existing session)
    const handleSendMessage = (message: string) => {
        RpcApi.AgentInputCommand(TabRpcClient, {
            blockid: model.blockId,
            message: message,
        }).catch(() => {
            // logged by RPC layer
        });
    };

    const handleBack = async () => {
        const { WOS } = await import("@/app/store/global");
        const oref = WOS.makeORef("block", model.blockId);
        try {
            await RpcApi.SetMetaCommand(TabRpcClient, {
                oref,
                meta: {
                    agentId: null,
                    agentProvider: null,
                    agentOutputFormat: null,
                    agentName: null,
                    agentIcon: null,
                    agentCliPath: null,
                    agentCliArgs: null,
                    agentBinDir: null,
                    controller: null,
                },
            });
        } catch {
            // model logs internally
        }
    };

    return (
        <div class="agent-view agent-view--presentation">
            <div class="agent-pres-header">
                <span class="agent-pres-icon">{block()?.meta?.["agentIcon"] ?? "\u26A1"}</span>
                <span class="agent-pres-name">{block()?.meta?.["agentName"] ?? provider()?.displayName ?? agentId}</span>
                <button class="agent-pres-back" onClick={handleBack} title="Back to agents">
                    {"\u2715"}
                </button>
            </div>

            <AgentDocumentView
                documentAtom={agentAtoms().documentAtom}
                documentStateAtom={agentAtoms().documentStateAtom}
            />

            <AgentFooter agentId={agentId} onSendMessage={handleSendMessage} />
        </div>
    );
};

AgentPresentationView.displayName = "AgentPresentationView";
