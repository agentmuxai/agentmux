// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentCardSettingsPanel — inline settings panel that expands below a
 * clicked AgentCard. Hosts the Forge and Identity tabs for a single
 * agent.
 *
 * PR 2 of specs/SPEC_CONSOLIDATE_FORGE_IDENTITY_INTO_AGENT_2026_04_13.md.
 * Identity tab upgraded to agent-scoped view in Step 4 of
 * SPEC_AGENT_IDENTITY_RESTRUCTURE_2026_04_14.md.
 */

import { createEffect, createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";
import type { BlockNodeModel } from "@/app/block/blocktypes";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { AgentDefViewModel } from "@/app/view/agent-def/agent-def-model";
import { AgentDefDetail } from "@/app/view/agent-def/components/AgentDefDetail";
import { AgentDefForm } from "@/app/view/agent-def/components/AgentDefForm";
import {
    type AgentAccounts,
    IdentityViewModel,
    serializeAgentAccounts,
} from "@/app/view/identity/identity-model";
import { AgentIdentityPanel } from "./AgentIdentityPanel";

export type SettingsTab = "agent" | "identity";

interface AgentCardSettingsPanelProps {
    blockId: string;
    nodeModel: BlockNodeModel;
    /** The agent being edited. Undefined = create mode (new agent). */
    agent: AgentDefinition | undefined;
    initialTab: SettingsTab;
    onClose: () => void;
    onTabChange?: (tab: SettingsTab) => void;
}

export const AgentCardSettingsPanel = (props: AgentCardSettingsPanelProps): JSX.Element => {
    const [tab, setTab] = createSignal<SettingsTab>(props.initialTab);

    // Dedicated Forge model for this panel session. Disposed on unmount
    // so the wave-event subscriptions are cleaned up.
    const agentDefModel = new AgentDefViewModel(props.blockId, props.nodeModel);

    // Dedicated Identity model for the Identity tab. Currently shows the
    // global account list (same as the old identity widget); per-agent
    // scoping is deferred to SPEC_FORGE_AGENT_IDENTITY_2026_04_13.md.
    const identityModel = new IdentityViewModel(props.blockId, props.nodeModel);

    // Guard for the auto-close effect below. AgentDefViewModel initializes
    // viewAtom to "list" synchronously in its constructor, and createEffect
    // fires once during component creation (before onMount). Without this
    // flag the effect would see v === "list" immediately and close the
    // panel before we've had a chance to call openDetail / startCreate.
    let mounted = false;

    onMount(() => {
        if (props.agent) {
            void agentDefModel.openDetail(props.agent);
        } else {
            agentDefModel.startCreate();
        }
        mounted = true;

        const handleKey = (e: KeyboardEvent) => {
            if (e.key === "Escape") {
                e.preventDefault();
                props.onClose();
            }
        };
        window.addEventListener("keydown", handleKey);
        onCleanup(() => window.removeEventListener("keydown", handleKey));
    });

    onCleanup(() => {
        agentDefModel.dispose();
        identityModel.dispose();
    });

    // When the AgentDefViewModel view flips back to "list" (e.g. user clicked
    // the in-panel Back button inside AgentDefDetail, or saved a create/edit)
    // we want to close the whole settings panel — the user explicitly
    // asked to exit. Only fires after mount to avoid the synchronous
    // initial-run problem described above.
    createEffect(() => {
        const v = agentDefModel.viewAtom();
        if (mounted && v === "list") {
            props.onClose();
        }
    });

    // Persist updated account assignments to the backend. Sends a full
    // UpdateAgentDefinitionCommand with the existing agent fields + new accounts.
    const handleAccountsUpdate = async (newAccounts: AgentAccounts): Promise<void> => {
        const agent = props.agent;
        if (!agent) return;
        await RpcApi.UpdateAgentDefinitionCommand(TabRpcClient, {
            id: agent.id,
            name: agent.name,
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
            accounts: serializeAgentAccounts(newAccounts),
        });
    };

    return (
        <div class="agent-card-settings-panel">
            <div class="agent-card-settings-header">
                <div class="agent-card-settings-tabs">
                    <button
                        class={`agent-card-settings-tab${tab() === "agent" ? " active" : ""}`}
                        onClick={() => { setTab("agent"); props.onTabChange?.("agent"); }}
                    >
                        {"\u2699"} Forge
                    </button>
                    <button
                        class={`agent-card-settings-tab${tab() === "identity" ? " active" : ""}`}
                        onClick={() => { setTab("identity"); props.onTabChange?.("identity"); }}
                    >
                        {"\uD83D\uDC64"} Identity
                    </button>
                </div>
                <button
                    class="agent-card-settings-close"
                    onClick={() => props.onClose()}
                    title="Close (Esc)"
                >
                    {"\u2715"}
                </button>
            </div>

            <div class="agent-card-settings-body">
                <Show when={tab() === "agent"}>
                    <Show
                        when={agentDefModel.viewAtom() === "create" || agentDefModel.viewAtom() === "edit"}
                        fallback={<AgentDefDetail model={agentDefModel} />}
                    >
                        <AgentDefForm model={agentDefModel} />
                    </Show>
                </Show>
                <Show when={tab() === "identity"}>
                    <Show
                        when={props.agent}
                        fallback={
                            <div class="agent-identity-unsaved">
                                Save the agent first to assign accounts.
                            </div>
                        }
                    >
                        <AgentIdentityPanel
                            agent={props.agent!}
                            model={identityModel}
                            onUpdate={handleAccountsUpdate}
                        />
                    </Show>
                </Show>
            </div>
        </div>
    );
};

AgentCardSettingsPanel.displayName = "AgentCardSettingsPanel";
