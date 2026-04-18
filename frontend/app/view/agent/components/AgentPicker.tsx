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
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { atoms, WOS } from "@/app/store/global";
import { waveEventSubscribe } from "@/app/store/wps";
import type { AgentViewModel } from "../agent-model";
import { AgentCard } from "./AgentCard";
import { NewAgentCard } from "./NewAgentCard";
import { AgentCardSettingsPanel, type SettingsTab } from "./AgentCardSettingsPanel";

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

    // Inline settings-panel state: which agent is expanded and which tab
    // (forge/identity) is active. `null` agentId means "create new" mode.
    // Session-local only — not persisted to block meta.
    const [expandedId, setExpandedId] = createSignal<string | null>(null);
    const [expandedTab, setExpandedTab] = createSignal<SettingsTab>("forge");
    const [createMode, setCreateMode] = createSignal(false);

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

    const openForgeFor = (agent: ForgeAgent) => {
        setCreateMode(false);
        // Capture the previous tab BEFORE switching — otherwise we can't
        // tell whether clicking ⚙ on an expanded card means "collapse"
        // (already on forge) or "switch from identity to forge".
        const prevTab = expandedTab();
        const prevId = expandedId();
        setExpandedTab("forge");
        if (prevId === agent.id && prevTab === "forge") {
            setExpandedId(null);
        } else {
            setExpandedId(agent.id);
        }
    };

    const openIdentityFor = (agent: ForgeAgent) => {
        setCreateMode(false);
        const prevTab = expandedTab();
        const prevId = expandedId();
        setExpandedTab("identity");
        if (prevId === agent.id && prevTab === "identity") {
            setExpandedId(null);
        } else {
            setExpandedId(agent.id);
        }
    };

    const openCreateNew = () => {
        setCreateMode(true);
        setExpandedTab("forge");
        setExpandedId("__new__");
    };

    const closePanel = () => {
        setExpandedId(null);
        setCreateMode(false);
    };

    /**
     * Validate + execute an inline rename from AgentCard.
     * Returns null on success, or an error string to display inline.
     */
    const handleRename = async (agent: ForgeAgent, newName: string): Promise<string | null> => {
        const trimmed = newName.trim();
        if (!trimmed) return "Name cannot be empty";
        if (trimmed.length > 64) return "Name must be 64 characters or fewer";

        // Uniqueness check (case-insensitive, exclude self)
        const lc = trimmed.toLowerCase();
        const collision = agents().find(
            (a) => a.id !== agent.id && a.name.toLowerCase() === lc
        );
        if (collision) return `Name "${trimmed}" is already taken`;

        try {
            await RpcApi.UpdateForgeAgentCommand(TabRpcClient, {
                id: agent.id,
                name: trimmed,
                icon: agent.icon,
                provider: agent.provider,
                description: agent.description,
                working_directory: agent.working_directory,
                shell: agent.shell,
                provider_flags: agent.provider_flags,
                auto_start: agent.auto_start,
                restart_on_crash: agent.restart_on_crash,
                idle_timeout_minutes: agent.idle_timeout_minutes,
                agent_type: agent.agent_type,
                environment: agent.environment,
                agent_bus_id: agent.agent_bus_id,
            });
            // Sync the display name in the block meta of any pane running this agent
            void syncAgentNameInOpenPanes(agent.id, trimmed);
            return null;
        } catch (e: any) {
            return String(e?.message ?? e);
        }
    };

    /**
     * After a rename, find any open agent panes running this agent and update
     * their block meta agentName so the pane-frame title refreshes immediately.
     */
    const syncAgentNameInOpenPanes = async (agentId: string, newName: string): Promise<void> => {
        try {
            const tabId = atoms.staticTabId();
            if (!tabId) return;
            const tab = WOS.getWaveObjectAtom<Tab>(`tab:${tabId}`)();
            if (!tab?.blockids) return;
            for (const blockId of tab.blockids) {
                const block = WOS.getWaveObjectAtom<Block>(`block:${blockId}`)();
                if (block?.meta?.["agentId"] === agentId) {
                    await RpcApi.SetMetaCommand(TabRpcClient, {
                        oref: WOS.makeORef("block", blockId),
                        meta: { agentName: newName },
                    });
                }
            }
        } catch {
            // best-effort — pane title will update on next interaction
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
                        <div class="agent-picker-empty-desc">
                            Click the <strong>+ New agent</strong> tile below to create one.
                        </div>
                        <NewAgentCard onClick={openCreateNew} />
                        <Show when={expandedId() === "__new__" && createMode()}>
                            <AgentCardSettingsPanel
                                blockId={props.model.blockId}
                                nodeModel={props.model.nodeModel}
                                agent={undefined}
                                initialTab="forge"
                                onClose={closePanel}
                            />
                        </Show>
                    </div>
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
                                        onOpenIdentity={openIdentityFor}
                                        onRename={handleRename}
                                    />
                                    <Show when={expandedId() === agent.id && !createMode()}>
                                        <AgentCardSettingsPanel
                                            blockId={props.model.blockId}
                                            nodeModel={props.model.nodeModel}
                                            agent={agent}
                                            initialTab={expandedTab()}
                                            onClose={closePanel}
                                        />
                                    </Show>
                                </>
                            )}
                        </For>
                        <NewAgentCard
                            disabled={busy()}
                            onClick={openCreateNew}
                        />
                        <Show when={expandedId() === "__new__" && createMode()}>
                            <AgentCardSettingsPanel
                                blockId={props.model.blockId}
                                nodeModel={props.model.nodeModel}
                                agent={undefined}
                                initialTab="forge"
                                onClose={closePanel}
                            />
                        </Show>
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
            </div>
        </Show>
    );
};

AgentPicker.displayName = "AgentPicker";
