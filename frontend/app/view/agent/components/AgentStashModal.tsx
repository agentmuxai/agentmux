// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentStashModal — unified tabbed container for per-agent setup surfaces,
 * opened by the single "Stash" (backpack) icon in the agent pane header.
 * This is the per-agent-scoped analogue of the global Armory pane
 * (frontend/app/view/armory/) — "Stash" vs. "Armory" is a deliberate naming
 * split so the two are never confused for the same surface (see
 * docs/reports/REPORT_ARMORY_STASH_NAMING_2026_07_27.md). Consolidates
 * the former two icons (identity + native memory) into one modal with tabs:
 *   - Accounts    — read-only linked-accounts view (AgentIdentityLinksPanel).
 *                   New bindings are created from the agent-launch flow;
 *                   see that component's own doc comment for why this tab
 *                   is not a create/edit surface.
 *   - Memory      — the native-memory browser (AgentNativeMemoryModal).
 *   - MCP Servers — the standalone MCP Server primitive (AgentMcpModal).
 *   - Skills      — the standalone Skill primitive (AgentSkillsModal).
 *   - Startup     — select an existing Bundle as Session Context's
 *                   "Startup Instructions" (AgentStartupModal).
 *   - Registration — this agent's live jekt/muxbus delivery status:
 *                   local registration, any OTHER instance/channel on this
 *                   host also claiming the same identity, and any recent
 *                   delivery rejected by the identity-mismatch guard
 *                   (AgentRegistrationPanel — issues #2694/#2695/#2696).
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
import { AgentRegistrationPanel } from "./AgentRegistrationPanel";
import { AgentSkillsModal } from "./AgentSkillsModal";
import { AgentStartupModal } from "./AgentStartupModal";
import "./AgentStashModal.scss";

type StashTabId = "accounts" | "memory" | "mcp" | "skills" | "startup" | "registration";

interface AgentStashModalProps {
    agentId: string;
    agentName: string;
    workingDirectory: string;
    initialTab?: StashTabId;
    onClose: () => void;
}

interface StashTabDef {
    id: StashTabId;
    label: string;
    /** FontAwesome icon name — same choice as the matching section in the
     *  global Armory pane (armory-view.tsx's RAIL), for visual parity since
     *  this modal is the per-agent-scoped analogue of it. */
    icon: string;
}

export const AgentStashModal = (props: AgentStashModalProps): JSX.Element => {
    // Data-driven tab list — Briefs slots in here later (no backend
    // primitive yet).
    const tabs: StashTabDef[] = [
        { id: "accounts", label: "Accounts", icon: "key" },
        { id: "memory", label: "Memories", icon: "brain" },
        { id: "mcp", label: "MCP Servers", icon: "plug" },
        { id: "skills", label: "Skills", icon: "wand-magic-sparkles" },
        // layer-group: same icon armory-view.tsx's RAIL uses for "ABF" —
        // this tab picks a bundle as startup instructions, so it's the same
        // concept scoped to one agent.
        { id: "startup", label: "Startup", icon: "layer-group" },
        // tower-broadcast: distinct from "key" (Accounts, auth identity) —
        // this tab is about jekt/muxbus delivery identity, a different
        // concept (issue #2696).
        { id: "registration", label: "Registration", icon: "tower-broadcast" },
    ];

    const [activeTab, setActiveTab] = createSignal<StashTabId>(props.initialTab ?? "accounts");

    return (
        // agent-stash-modal carries container-type so the tabs below (a
        // descendant) can be targeted by @container agent-stash queries —
        // same technique as armory-view.tsx's .armory-container wrapper.
        <div class="agent-stash-modal">
            <div class="agent-stash-modal-tabs" role="tablist">
                <For each={tabs}>
                    {(tab) => (
                        <button
                            class="agent-stash-modal-tab"
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

            <div class="agent-stash-modal-panel">
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

                <Show when={activeTab() === "registration"}>
                    <AgentRegistrationPanel agentId={props.agentId} />
                </Show>

                {/* Future primitives: Briefs */}
            </div>
        </div>
    );
};

AgentStashModal.displayName = "AgentStashModal";
