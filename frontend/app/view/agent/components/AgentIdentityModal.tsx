// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentIdentityModalPanel — pane-scoped modal shell for per-agent
 * identity (account) assignment.
 *
 * Wraps the existing AgentIdentityPanel component. Opened by the
 * id-card icon in the agent pane header; replaces the former
 * AgentCardSettingsPanel Identity tab. handleAccountsUpdate migrated
 * here from AgentCardSettingsPanel unchanged.
 *
 * Local `liveAgent` signal keeps account state current across multiple
 * saves within the same modal session — each successful save updates
 * the snapshot so the next assignment builds on the latest state rather
 * than the snapshot from when the modal opened (Codex P2 on PR #1587).
 *
 * Spec: SPEC_AGENT_PANE_MEMORY_IDENTITY_MODALS_2026_06_19.md §4.
 */

import { createSignal, onCleanup, type JSX } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import {
    IdentityViewModel,
    serializeAgentAccounts,
    type AgentAccounts,
} from "@/app/view/identity/identity-model";
import { AgentIdentityPanel } from "./AgentIdentityPanel";

interface AgentIdentityModalPanelProps {
    agent: AgentDefinition;
    blockId: string;
    onClose: () => void;
}

export const AgentIdentityModalPanel = (props: AgentIdentityModalPanelProps): JSX.Element => {
    const model = new IdentityViewModel(props.blockId, null);
    onCleanup(() => model.dispose());

    // Track the latest agent snapshot locally so that consecutive saves
    // within the same modal session each build on the prior result.
    // Without this, AgentIdentityPanel's `parseAgentAccounts(props.agent)`
    // memo keeps reading the original snapshot and the second assignment
    // overwrites the first.
    const [liveAgent, setLiveAgent] = createSignal<AgentDefinition>(props.agent);

    const handleUpdate = async (newAccounts: AgentAccounts): Promise<void> => {
        const a = liveAgent();
        const serialized = serializeAgentAccounts(newAccounts);
        await RpcApi.UpdateAgentDefinitionCommand(TabRpcClient, {
            id: a.id,
            name: a.name,
            icon: a.icon,
            provider: a.provider,
            description: a.description,
            working_directory: a.working_directory,
            shell: a.shell,
            provider_flags: a.provider_flags,
            auto_start: a.auto_start,
            restart_on_crash: a.restart_on_crash,
            idle_timeout_minutes: a.idle_timeout_minutes,
            agent_type: a.agent_type,
            environment: a.environment,
            agent_bus_id: a.agent_bus_id,
            accounts: serialized,
        });
        setLiveAgent({ ...a, accounts: serialized });
    };

    return (
        <div class="agent-identity-modal-body">
            <AgentIdentityPanel
                agent={liveAgent()}
                model={model}
                onUpdate={handleUpdate}
            />
            <div class="agent-modal-footer">
                <button
                    class="agent-modal-done-btn"
                    data-modal-dismiss
                    onClick={props.onClose}
                >
                    Done
                </button>
            </div>
        </div>
    );
};

AgentIdentityModalPanel.displayName = "AgentIdentityModalPanel";
