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
 * Also hosts the per-agent "Use global CLI login" toggle
 * (`use_ambient_login`) — the explicit opt-in that lets a spawn proceed
 * on the CLI's global login when no oauth-class account resolves.
 * Without it, such spawns fail with a visible error (layer-3 spawn
 * gating, SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md §2.2-§2.3).
 *
 * Spec: SPEC_AGENT_PANE_MEMORY_IDENTITY_MODALS_2026_06_19.md §4.
 */

import { createSignal, onCleanup, Show, type JSX } from "solid-js";
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
    const [ambientSaving, setAmbientSaving] = createSignal(false);
    const [ambientError, setAmbientError] = createSignal<string | null>(null);

    /** Common UpdateAgentDefinitionCommand payload from the live snapshot.
     *  `accounts` / `use_ambient_login` are layered on by the callers. */
    const basePayload = (a: AgentDefinition): CommandUpdateAgentDefinitionData => ({
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
    });

    const handleUpdate = async (newAccounts: AgentAccounts): Promise<void> => {
        const a = liveAgent();
        const serialized = serializeAgentAccounts(newAccounts);
        await RpcApi.UpdateAgentDefinitionCommand(TabRpcClient, {
            ...basePayload(a),
            accounts: serialized,
        });
        setLiveAgent({ ...a, accounts: serialized });
    };

    const ambientOn = () => (liveAgent().use_ambient_login ?? 0) !== 0;

    const handleAmbientToggle = async (): Promise<void> => {
        const a = liveAgent();
        const next = ambientOn() ? 0 : 1;
        setAmbientError(null);
        setAmbientSaving(true);
        try {
            await RpcApi.UpdateAgentDefinitionCommand(TabRpcClient, {
                ...basePayload(a),
                use_ambient_login: next,
            });
            setLiveAgent({ ...a, use_ambient_login: next });
        } catch (e) {
            setAmbientError(String(e));
        } finally {
            setAmbientSaving(false);
        }
    };

    return (
        <div class="agent-identity-modal-body">
            <AgentIdentityPanel
                agent={liveAgent()}
                model={model}
                onUpdate={handleUpdate}
            />
            {/* Layer-3 ambient-login opt-in (spec §2.3). Immediate RPC on
                change — same convention as the account dropdowns above. */}
            <div class="agent-ambient-login-section">
                <div class="agent-ambient-login-row">
                    <div class="agent-ambient-login-text">
                        <span class="agent-ambient-login-label">
                            Use global CLI login when no account is bound
                        </span>
                        <span class="agent-ambient-login-help">
                            When on, this agent may launch with your machine-wide CLI
                            login (e.g. ~/.claude) if no managed account resolves.
                            When off, launching fails instead — deleting an account in
                            the Armory reliably deauthenticates this agent's next start.
                        </span>
                    </div>
                    <button
                        type="button"
                        role="switch"
                        aria-checked={ambientOn()}
                        aria-label="Use global CLI login when no account is bound"
                        class="agent-ambient-login-toggle"
                        classList={{ "agent-ambient-login-toggle--on": ambientOn() }}
                        disabled={ambientSaving()}
                        onClick={() => void handleAmbientToggle()}
                    >
                        <span class="agent-ambient-login-toggle-thumb" />
                    </button>
                </div>
                <Show when={ambientError()}>
                    <div class="agent-ambient-login-error">{ambientError()}</div>
                </Show>
            </div>
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
