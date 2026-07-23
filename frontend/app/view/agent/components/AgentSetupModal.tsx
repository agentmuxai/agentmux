// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentSetupModal — unified tabbed container for per-agent setup surfaces,
 * opened by the single "Agent setup" (vault) icon in the agent pane
 * header. Consolidates the former two icons (identity + native memory)
 * into one modal with tabs:
 *   - Accounts    — read-only linked-accounts view (AgentIdentityLinksPanel).
 *                   New bindings are created from the agent-launch flow;
 *                   see that component's own doc comment for why this tab
 *                   is not a create/edit surface.
 *   - Memory      — the native-memory browser (AgentNativeMemoryModal).
 *   - MCP Servers — the standalone MCP Server primitive (AgentMcpModal).
 *   - Skills      — the standalone Skill primitive (AgentSkillsModal).
 *   - Startup     — select an existing Bundle as Session Context's
 *                   "Startup Instructions" (AgentStartupModal).
 *
 * The tab list is a data array so future primitives slot in trivially.
 * Briefs is not wired here — it has no backend primitive at all yet
 * (tracked separately).
 *
 * Spec: SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md §3.2b +
 *       docs/specs/archive/EXPLAINER_COMPOSABLE_MODEL_AND_AGENT_PANE_2026_07_02.md §4.
 */

import { createSignal, For, Show, type JSX } from "solid-js";
import { AgentIdentityLinksPanel } from "@/app/view/identity/agent-identity-links-panel";
import { AgentMcpModal } from "./AgentMcpModal";
import { AgentNativeMemoryModal } from "./AgentNativeMemoryModal";
import { AgentSkillsModal } from "./AgentSkillsModal";
import { AgentStartupModal } from "./AgentStartupModal";
import "./AgentSetupModal.scss";

type SetupTabId = "accounts" | "memory" | "mcp" | "skills" | "startup";

interface AgentSetupModalProps {
    agentId: string;
    agentName: string;
    workingDirectory: string;
    initialTab?: SetupTabId;
    onClose: () => void;
}

interface SetupTabDef {
    id: SetupTabId;
    label: string;
    /** FontAwesome icon name — same choice as the matching section in the
     *  global Armory pane (armory-view.tsx's RAIL), for visual parity since
     *  this modal is the per-agent-scoped analogue of it. */
    icon: string;
}

export const AgentSetupModal = (props: AgentSetupModalProps): JSX.Element => {
    // Data-driven tab list — Briefs slots in here later (no backend
    // primitive yet).
    const tabs: SetupTabDef[] = [
        { id: "accounts", label: "Accounts", icon: "key" },
        { id: "memory", label: "Memories", icon: "brain" },
        { id: "mcp", label: "MCP Servers", icon: "plug" },
        { id: "skills", label: "Skills", icon: "wand-magic-sparkles" },
        // layer-group: same icon armory-view.tsx's RAIL uses for "Bundles" —
        // this tab picks a Bundle as startup instructions, so it's the same
        // concept scoped to one agent.
        { id: "startup", label: "Startup", icon: "layer-group" },
    ];

    const [activeTab, setActiveTab] = createSignal<SetupTabId>(props.initialTab ?? "accounts");

    return (
        // agent-setup-modal carries container-type so the tabs below (a
        // descendant) can be targeted by @container agent-setup queries —
        // same technique as armory-view.tsx's .armory-container wrapper.
        <div class="agent-setup-modal">
            <div class="agent-setup-modal-tabs" role="tablist">
                <For each={tabs}>
                    {(tab) => (
                        <button
                            class="agent-setup-modal-tab"
                            classList={{ "is-active": activeTab() === tab.id }}
                            role="tab"
                            aria-selected={activeTab() === tab.id}
                            title={tab.label}
                            onClick={() => setActiveTab(tab.id)}
                        >
                            <i class={`fa-sharp fa-solid fa-${tab.icon}`} aria-hidden="true" />
                            <span>{tab.label}</span>
                        </button>
                    )}
                </For>
            </div>

            <div class="agent-setup-modal-panel">
                <Show when={activeTab() === "accounts"}>
                    <AgentIdentityLinksPanel agentId={props.agentId} />
                </Show>

                <Show when={activeTab() === "memory"}>
                    <AgentNativeMemoryModal
                        agentId={props.agentId}
                        agentName={props.agentName}
                        workingDirectory={props.workingDirectory}
                        onClose={props.onClose}
                    />
                </Show>

                <Show when={activeTab() === "mcp"}>
                    <AgentMcpModal agentId={props.agentId} />
                </Show>

                <Show when={activeTab() === "skills"}>
                    <AgentSkillsModal agentId={props.agentId} />
                </Show>

                <Show when={activeTab() === "startup"}>
                    <AgentStartupModal agentId={props.agentId} />
                </Show>

                {/* Future primitives: Briefs */}
            </div>
        </div>
    );
};

AgentSetupModal.displayName = "AgentSetupModal";
