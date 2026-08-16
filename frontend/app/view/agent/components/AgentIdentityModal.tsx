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
 * The per-agent "Use global CLI login" toggle (`use_ambient_login`) that
 * used to live here has been removed (reagent P1 on #2262): the layer-3
 * spawn gate (`identity/resolver.rs`'s `gate_oauth_failure`) was changed to
 * unconditionally refuse an oauth-class spawn with no bound account — every
 * provider, no per-agent opt-out — per the explicit "single point, not
 * global... close it everywhere, now" policy
 * (PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_2026_07_20.md §7). The toggle kept
 * rendering as fully interactive with help text promising a working
 * fallback that the backend had already stopped honoring, misleading users
 * into believing they'd configured something that did nothing. The
 * `use_ambient_login` DB column/field itself is untouched (still read and
 * logged, just never gates) — only this now-dead UI is removed.
 *
 * The "Model Vendor / Custom Endpoint" editor that used to live here was
 * also removed (issue #2594, 2026-08-16): `model_vendor_base_url` is a
 * set-once-at-creation field under the Mandatory ABF architecture
 * (`ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md` §7.4.1) — same
 * immutability contract `provider` already has, which this modal has
 * never let a user edit either. Editing it here wrote straight onto
 * `AgentDefinition` post-creation with no bundle-side field to keep in
 * sync (unlike `provider`, the bundle has no base-url column at all), so
 * every save silently diverged the agent from whatever its bundle was
 * actually provisioned with. The definition-time editor in
 * `AgentCreateFromTemplateModal.tsx` is unaffected — it still sets the
 * value once, before/at bundle provisioning, which is the correct place
 * for it.
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

    /** Common UpdateAgentDefinitionCommand payload from the live snapshot.
     *  `accounts` is layered on by the caller. */
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
