// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentSetupModal — unified tabbed container for per-agent setup surfaces,
 * opened by the single "Agent setup" (id-card) icon in the agent pane
 * header. Consolidates the former two icons (identity + native memory)
 * into one modal with tabs:
 *   - Accounts — the former Identity panel (AgentIdentityModalPanel).
 *   - Memory   — the native-memory browser (AgentNativeMemoryModal).
 *
 * The tab list is a data array so future primitives slot in trivially.
 * Behavior-preserving: the two hosted panels are reused as-is; only the
 * entry point (one icon + tabs) changed.
 *
 * Spec: SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md §3.2b +
 *       EXPLAINER_COMPOSABLE_MODEL_AND_AGENT_PANE_2026_07_02.md §4.
 */

import { createSignal, For, Show, type JSX } from "solid-js";
import { AgentIdentityModalPanel } from "./AgentIdentityModal";
import { AgentNativeMemoryModal } from "./AgentNativeMemoryModal";
import "./AgentSetupModal.scss";

type SetupTabId = "accounts" | "memory";

interface AgentSetupModalProps {
    /** Agent definition for the Accounts tab. Null for quick-launch panes
     *  with no loadable definition — the Accounts tab shows an empty state. */
    agent: AgentDefinition | null;
    agentId: string;
    agentName: string;
    workingDirectory: string;
    blockId: string;
    initialTab?: SetupTabId;
    onClose: () => void;
}

interface SetupTabDef {
    id: SetupTabId;
    label: string;
}

export const AgentSetupModal = (props: AgentSetupModalProps): JSX.Element => {
    // Data-driven tab list — future primitives (Phase 3+): MCP Servers ·
    // Skills · Briefs · Bundle slot in here as additional entries plus a
    // render branch below. Do NOT build those managers yet.
    const tabs: SetupTabDef[] = [
        { id: "accounts", label: "Accounts" },
        { id: "memory", label: "Memory" },
    ];

    const [activeTab, setActiveTab] = createSignal<SetupTabId>(props.initialTab ?? "accounts");

    return (
        <div class="agent-setup-modal">
            <div class="agent-setup-modal-tabs" role="tablist">
                <For each={tabs}>
                    {(tab) => (
                        <button
                            class="agent-setup-modal-tab"
                            classList={{ "is-active": activeTab() === tab.id }}
                            role="tab"
                            aria-selected={activeTab() === tab.id}
                            onClick={() => setActiveTab(tab.id)}
                        >
                            {tab.label}
                        </button>
                    )}
                </For>
            </div>

            <div class="agent-setup-modal-panel">
                <Show when={activeTab() === "accounts"}>
                    <Show
                        when={props.agent}
                        fallback={
                            <div class="agent-setup-modal-empty">
                                Account assignment is unavailable for this pane — it isn't
                                backed by a saved agent definition.
                            </div>
                        }
                    >
                        {(agent) => (
                            <AgentIdentityModalPanel
                                agent={agent()}
                                blockId={props.blockId}
                                onClose={props.onClose}
                            />
                        )}
                    </Show>
                </Show>

                <Show when={activeTab() === "memory"}>
                    <AgentNativeMemoryModal
                        agentId={props.agentId}
                        agentName={props.agentName}
                        workingDirectory={props.workingDirectory}
                        onClose={props.onClose}
                    />
                </Show>

                {/* Future primitives (Phase 3+): MCP Servers · Skills · Briefs · Bundle */}
            </div>
        </div>
    );
};

AgentSetupModal.displayName = "AgentSetupModal";
