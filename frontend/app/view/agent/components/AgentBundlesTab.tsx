// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentBundlesTab — the "Bundles" tab inside AgentStashModal: the agent's
 * Bundles list, in order (SPEC_RENAME_KNOWLEDGE_TO_MEMORY_2026_10_06.md §3.6).
 * The agent's own bundle comes first and is fixed; the picks after it are
 * added, removed and reordered here, and saved on each change
 * (`setagentbundles`). From the next launch, their instructions go into the
 * agent's startup file after Global Memory, and their skills and MCP servers
 * are added.
 *
 * Replaces the Startup tab, which picked one bundle to send as the agent's
 * first message; srv folds that old pick into this list.
 */

import { createResource, createSignal, Show, type JSX } from "solid-js";
import { Button } from "@/app/element/ui";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { openMemory } from "@/app/view/section-pane/panes";
import { BundleListEditor } from "./BundleListEditor";
import "./AgentPrimitiveModal.scss";

interface AgentBundlesTabProps {
    agentId: string;
}

export const AgentBundlesTab = (props: AgentBundlesTabProps): JSX.Element => {
    const [bundles] = createResource(() => RpcApi.ListBundlesCommand(TabRpcClient, {}));
    const [picks, setPicks] = createSignal<string[]>([]);
    const [ownId, setOwnId] = createSignal("");
    const [loaded, setLoaded] = createSignal(false);
    const [saving, setSaving] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);

    void RpcApi.GetAgentBundlesCommand(TabRpcClient, { agent_id: props.agentId })
        .then((r) => {
            setPicks(r.bundle_ids);
            setOwnId(r.own_bundle_id);
        })
        .catch((e) => setError(String(e)))
        .finally(() => setLoaded(true));

    // Blank is the vanilla-CLI sentinel and system entries are Global Memory,
    // not something to pick (#2782). The agent's own bundle is shown
    // separately, first.
    const selectable = () => (bundles() ?? []).filter((b) => !b.is_blank && !b.is_system && b.id !== ownId());
    const ownName = () => (bundles() ?? []).find((b) => b.id === ownId())?.name;

    const save = async (ids: string[]): Promise<void> => {
        setError(null);
        setSaving(true);
        const before = picks();
        setPicks(ids);
        try {
            const r = await RpcApi.SetAgentBundlesCommand(TabRpcClient, { agent_id: props.agentId, bundle_ids: ids });
            setPicks(r.bundle_ids);
        } catch (e) {
            setPicks(before);
            setError(String(e));
        } finally {
            setSaving(false);
        }
    };

    return (
        <div class="agent-primitive-modal-detail">
            <div class="agent-primitive-modal-readonly">
                <span class="agent-primitive-modal-field-label">Bundles</span>
                <p class="agent-primitive-modal-global-note">
                    What this agent starts with, in order. Each bundle's instructions go into its startup file
                    after Global Memory, and its skills and MCP servers are added. When two bundles name the same
                    one, the first wins. Changes apply from the next launch.
                </p>
                <Show when={error()}>
                    <div class="agent-primitive-modal-error">{error()}</div>
                </Show>
                <BundleListEditor
                    bundles={selectable()}
                    value={picks()}
                    onChange={(ids) => void save(ids)}
                    own={ownName()}
                    disabled={!loaded() || saving()}
                    testId="agent-bundles-tab-list"
                />
                <p class="agent-primitive-modal-global-note">
                    Bundles are edited in{" "}
                    <Button tone="quiet" density="compact" onClick={() => void openMemory("bundles")}>
                        Memory → Bundles
                    </Button>
                    . A change there reaches every agent using the bundle.
                </p>
            </div>
        </div>
    );
};

AgentBundlesTab.displayName = "AgentBundlesTab";
