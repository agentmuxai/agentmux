// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentStartupModal — the "Startup" tab body inside AgentStashModal.
 *
 * Lets an agent select an existing Armory Bundle to serve as its Session
 * Context "Startup Instructions" (see buildStartupPayload.ts). This is a
 * single-selection field, not a CRUD primitive like AgentMcpModal/
 * AgentSkillsModal — no draft/edit state, no model class, just a select
 * that saves immediately on change (same convention as
 * AgentIdentityModal.tsx's ambient-login toggle).
 *
 * The selection itself is stored under a new db_agent_content content_type
 * ("startup_bundle_id") rather than a new AgentDefinition column, reusing
 * the existing getagentcontent/setagentcontent RPCs end-to-end — see
 * docs/specs/ARCHITECTURE_ARMORY_2026_07_20.md §5 for why (adding a real
 * schema column would require touching three separate AgentDefinition
 * storage mirrors).
 */

import { createResource, createSignal, For, Show, type JSX } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { openOrFocusPaneByView } from "@/app/store/global";
import "./AgentPrimitiveModal.scss";

const STARTUP_BUNDLE_CONTENT_TYPE = "startup_bundle_id";

interface AgentStartupModalProps {
    agentId: string;
}

export const AgentStartupModal = (props: AgentStartupModalProps): JSX.Element => {
    const [bundles] = createResource(() => RpcApi.ListMemoriesCommand(TabRpcClient, {}));
    const [selectedId, setSelectedId] = createSignal<string | null>(null);
    const [loaded, setLoaded] = createSignal(false);
    const [saving, setSaving] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);

    void RpcApi.GetAgentContentCommand(TabRpcClient, {
        agent_id: props.agentId,
        content_type: STARTUP_BUNDLE_CONTENT_TYPE,
    })
        .then((c) => setSelectedId(c?.content?.trim() || null))
        .catch(() => setSelectedId(null))
        .finally(() => setLoaded(true));

    // Non-blank, non-system bundles only — "blank" is the vanilla-CLI
    // sentinel with no instructions worth surfacing here (same filter
    // convention used by the launch modal's own bundle picker); is_system
    // entries are AgentMux-controlled workspace policy, not a selectable
    // per-agent bundle (reagent P1, PR #2782).
    const selectable = () => (bundles() ?? []).filter((m) => !m.is_blank && !m.is_system);
    const selectedBundle = () => selectable().find((m) => m.id === selectedId());

    const handleChange = async (id: string): Promise<void> => {
        setError(null);
        setSaving(true);
        try {
            await RpcApi.SetAgentContentCommand(TabRpcClient, {
                agent_id: props.agentId,
                content_type: STARTUP_BUNDLE_CONTENT_TYPE,
                content: id,
            });
            setSelectedId(id || null);
        } catch (e) {
            setError(String(e));
        } finally {
            setSaving(false);
        }
    };

    return (
        <div class="agent-primitive-modal-detail">
            <div class="agent-primitive-modal-readonly">
                <span class="agent-primitive-modal-field-label">Startup instructions</span>
                <p class="agent-primitive-modal-global-note">
                    Select an existing Bundle to use as this agent's Session Context
                    "Startup Instructions" — the message it receives at the start of
                    every new session, in addition to its identity and assigned
                    accounts.
                </p>
                <Show when={error()}>
                    <div class="agent-primitive-modal-error">{error()}</div>
                </Show>
                <div class="agent-primitive-modal-bind-row">
                    <select
                        class="agent-primitive-modal-input"
                        disabled={!loaded() || saving()}
                        value={selectedId() ?? ""}
                        onChange={(e) => void handleChange(e.currentTarget.value)}
                    >
                        <option value="">None</option>
                        <For each={selectable()}>
                            {(bundle) => <option value={bundle.id}>{bundle.name}</option>}
                        </For>
                    </select>
                </div>
                <Show when={selectedBundle()}>
                    {(bundle) => (
                        <p class="agent-primitive-modal-global-note">
                            Edited from{" "}
                            <button
                                type="button"
                                class="agent-primitive-modal-link-btn"
                                onClick={() => void openOrFocusPaneByView("armory")}
                            >
                                Armory → ABF
                            </button>
                            . Changing "{bundle().name}" there updates every agent using
                            it, including this one.
                        </p>
                    )}
                </Show>
            </div>
        </div>
    );
};

AgentStartupModal.displayName = "AgentStartupModal";
