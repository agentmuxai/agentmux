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
import { ForgeViewModel } from "@/app/view/forge/forge-model";
import { ForgeDetail } from "@/app/view/forge/components/ForgeDetail";
import { ForgeForm } from "@/app/view/forge/components/ForgeForm";
import {
    type AgentAccounts,
    IdentityViewModel,
    serializeAgentAccounts,
} from "@/app/view/identity/identity-model";
import { AgentIdentityPanel } from "./AgentIdentityPanel";

export type SettingsTab = "forge" | "identity";

interface AgentCardSettingsPanelProps {
    blockId: string;
    nodeModel: BlockNodeModel;
    /** The agent being edited. Undefined = create mode (new agent). */
    agent: ForgeAgent | undefined;
    initialTab: SettingsTab;
    onClose: () => void;
    onTabChange?: (tab: SettingsTab) => void;
}

export const AgentCardSettingsPanel = (props: AgentCardSettingsPanelProps): JSX.Element => {
    const [tab, setTab] = createSignal<SettingsTab>(props.initialTab);

    // Dedicated Forge model for this panel session. Disposed on unmount
    // so the wave-event subscriptions are cleaned up.
    const forgeModel = new ForgeViewModel(props.blockId, props.nodeModel);

    // Dedicated Identity model for the Identity tab. Currently shows the
    // global account list (same as the old identity widget); per-agent
    // scoping is deferred to SPEC_FORGE_AGENT_IDENTITY_2026_04_13.md.
    const identityModel = new IdentityViewModel(props.blockId, props.nodeModel);

    // Guard for the auto-close effect below. ForgeViewModel initializes
    // viewAtom to "list" synchronously in its constructor, and createEffect
    // fires once during component creation (before onMount). Without this
    // flag the effect would see v === "list" immediately and close the
    // panel before we've had a chance to call openDetail / startCreate.
    let mounted = false;

    onMount(() => {
        if (props.agent) {
            void forgeModel.openDetail(props.agent);
        } else {
            forgeModel.startCreate();
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
        forgeModel.dispose();
        identityModel.dispose();
    });

    // When the ForgeViewModel view flips back to "list" (e.g. user clicked
    // the in-panel Back button inside ForgeDetail, or saved a create/edit)
    // we want to close the whole settings panel — the user explicitly
    // asked to exit. Only fires after mount to avoid the synchronous
    // initial-run problem described above.
    createEffect(() => {
        const v = forgeModel.viewAtom();
        if (mounted && v === "list") {
            props.onClose();
        }
    });

    // Persist updated account assignments to the backend. Sends a full
    // UpdateForgeAgentCommand with the existing agent fields + new accounts.
    const handleAccountsUpdate = async (newAccounts: AgentAccounts): Promise<void> => {
        const agent = props.agent;
        if (!agent) return;
        await RpcApi.UpdateForgeAgentCommand(TabRpcClient, {
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
                        class={`agent-card-settings-tab${tab() === "forge" ? " active" : ""}`}
                        onClick={() => { setTab("forge"); props.onTabChange?.("forge"); }}
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
                <Show when={tab() === "forge"}>
                    <Show
                        when={forgeModel.viewAtom() === "create" || forgeModel.viewAtom() === "edit"}
                        fallback={<ForgeDetail model={forgeModel} />}
                    >
                        <ForgeForm model={forgeModel} />
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
