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
 * Spec: SPEC_AGENT_PANE_MEMORY_IDENTITY_MODALS_2026_06_19.md §4.
 */

import { createMemo, createSignal, onCleanup, Show, type JSX } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import {
    IdentityViewModel,
    serializeAgentAccounts,
    type AgentAccounts,
} from "@/app/view/identity/identity-model";
import { AgentIdentityPanel } from "./AgentIdentityPanel";
import { PROVIDERS } from "../providers/catalog";

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

    // ── Model vendor / custom endpoint ─────────────────────────────
    // Only providers that declare `baseUrlEnvVar` (currently just claude)
    // can actually be redirected — see agent_define::validate_vendor_base_url
    // on the backend, which rejects a non-empty override for any other
    // provider.
    const supportsCustomEndpoint = createMemo(
        () => !!PROVIDERS[liveAgent().provider]?.baseUrlEnvVar,
    );
    const [modelVendorBaseUrl, setModelVendorBaseUrl] = createSignal(
        props.agent.model_vendor_base_url ?? "",
    );
    const [vendorSaving, setVendorSaving] = createSignal(false);
    const [vendorError, setVendorError] = createSignal<string | null>(null);

    const handleVendorSave = async (): Promise<void> => {
        const a = liveAgent();
        const url = modelVendorBaseUrl().trim();
        setVendorSaving(true);
        setVendorError(null);
        try {
            await RpcApi.UpdateAgentDefinitionCommand(TabRpcClient, {
                ...basePayload(a),
                accounts: a.accounts,
                model_vendor_base_url: url,
            });
            setLiveAgent({ ...a, model_vendor_base_url: url });
        } catch (e) {
            setVendorError((e as Error)?.message ?? String(e));
        } finally {
            setVendorSaving(false);
        }
    };

    return (
        <div class="agent-identity-modal-body">
            <AgentIdentityPanel
                agent={liveAgent()}
                model={model}
                onUpdate={handleUpdate}
            />
            <Show when={supportsCustomEndpoint()}>
                <div class="agent-identity-vendor-section">
                    <span class="agent-identity-vendor-label">Model Vendor / Custom Endpoint</span>
                    <div class="agent-identity-vendor-row">
                        <input
                            type="text"
                            class="agent-identity-vendor-input"
                            placeholder={`Default (${PROVIDERS[liveAgent().provider]?.baseUrlEnvVar})`}
                            value={modelVendorBaseUrl()}
                            onInput={(e) => setModelVendorBaseUrl(e.currentTarget.value)}
                            disabled={vendorSaving()}
                            data-testid="agent-identity-vendor-base-url-input"
                        />
                        <button
                            class="agent-identity-vendor-save-btn"
                            disabled={vendorSaving() || modelVendorBaseUrl().trim() === (liveAgent().model_vendor_base_url ?? "")}
                            onClick={() => void handleVendorSave()}
                            data-testid="agent-identity-vendor-base-url-save"
                        >
                            {vendorSaving() ? "Saving…" : "Save"}
                        </button>
                    </div>
                    <span class="agent-identity-vendor-hint">
                        Redirect this agent's harness at a custom API endpoint instead
                        of the default vendor. Leave blank to use the default.
                    </span>
                    <Show when={vendorError()}>
                        <div class="agent-identity-vendor-error">{vendorError()}</div>
                    </Show>
                </div>
            </Show>
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
