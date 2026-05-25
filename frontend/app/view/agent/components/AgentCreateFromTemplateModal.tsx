// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentCreateFromTemplateModalPanel — Phase 1 two-tier picker modal
 * (SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md).
 *
 * Opens when the user clicks a card in the picker's Templates section.
 * Collects a name + identity + memory, then in `onSubmit` the layer
 * clones the seeded template into a new user-owned definition via
 * `agentdefcreatefromtemplate`, immediately launches it with the
 * picked bindings, and closes.
 *
 * Why a separate modal (not a flag on AgentLaunchModal): the launch
 * modal carries a lot of additional UX (runtime/container picker,
 * continuation dropdown, OAuth pre-launch panel, "+ New bundle"
 * affordances, NamedAgents resource). For a template-create the
 * minimum surface is name + bindings; mixing it into the launch modal
 * would push its complexity onto a flow that doesn't need it. The
 * canonical modal panel styles (`modal-panel-*`) and shared bundle
 * styles (`agent-new-bundle-modal-*`) are reused so this fits the
 * universal modal system per `feedback_use_universal_modal_system`.
 */

import { createEffect, createMemo, createSignal, For, onMount, Show, type JSX } from "solid-js";

import { Button } from "@/element/button";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

export interface CreateFromTemplateFormData {
    name: string;
    identityId: string;
    memoryId: string;
}

interface AgentCreateFromTemplateModalPanelProps {
    /** Seeded template the user clicked. */
    template: AgentDefinition;
    /** Suggested initial name (defaults to template name). */
    initialName?: string;
    /** Called when the user clicks Create with valid data. The layer
     *  wraps this with the create-then-launch RPC chain. */
    onSubmit: (formData: CreateFromTemplateFormData) => Promise<void>;
    onCancel: () => void;
}

export const AgentCreateFromTemplateModalPanel = (
    props: AgentCreateFromTemplateModalPanelProps,
): JSX.Element => {
    const [name, setName] = createSignal(props.initialName ?? props.template.name);
    const [identityId, setIdentityId] = createSignal("");
    const [memoryId, setMemoryId] = createSignal("");
    const [identities, setIdentities] = createSignal<IdentityBundle[]>([]);
    const [memories, setMemories] = createSignal<Memory[]>([]);
    const [submitting, setSubmitting] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);

    // Load bundle lists on mount. Bindings are stored on the
    // db_agent_instances row at launch time; the empty string sentinel
    // means "ambient creds / vanilla CLI" so an empty selection is OK.
    onMount(() => {
        void (async () => {
            try {
                const list = await RpcApi.ListIdentityBundlesCommand(TabRpcClient, {});
                setIdentities(list ?? []);
            } catch {
                /* non-fatal; user can still create without binding */
            }
            try {
                const list = await RpcApi.ListMemoriesCommand(TabRpcClient, {});
                setMemories(list ?? []);
            } catch {
                /* non-fatal */
            }
        })();
    });

    // Default-pick the first real bundle once the lists land, matching
    // the launch modal's UX (saves a click for users with existing
    // bundles). `is_blank` rows are back-compat singletons we filter
    // out — empty string is the "ambient" sentinel and is the empty-
    // list default anyway.
    const realIdentities = createMemo(() => identities().filter((b) => !b.is_blank));
    const realMemories = createMemo(() => memories().filter((m) => !m.is_blank));
    createEffect(() => {
        if (identityId()) return;
        const first = realIdentities()[0];
        if (first) setIdentityId(first.id);
    });
    createEffect(() => {
        if (memoryId()) return;
        const first = realMemories()[0];
        if (first) setMemoryId(first.id);
    });

    const canSubmit = () =>
        name().trim().length > 0
        && name().trim().length <= 200
        && !submitting();

    const submit = async () => {
        if (!canSubmit()) return;
        setSubmitting(true);
        setError(null);
        try {
            await props.onSubmit({
                name: name().trim(),
                identityId: identityId(),
                memoryId: memoryId(),
            });
            // Layer unmounts via close-on-success. Reset is defensive.
            setSubmitting(false);
        } catch (e) {
            setError((e as Error)?.message ?? String(e));
            setSubmitting(false);
        }
    };

    const onKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
            e.preventDefault();
            void submit();
        }
    };

    return (
        <>
            <header class="modal-panel-header">
                <h2 class="modal-panel-title">Create new agent from {props.template.name}</h2>
                <p class="modal-panel-description">
                    A new user-owned agent will be cloned from this template.
                    The template stays untouched and can be used again.
                </p>
            </header>
            <div class="modal-panel-body agent-new-bundle-modal-body">
                <label class="agent-new-bundle-modal-field">
                    <span class="agent-new-bundle-modal-label">Name</span>
                    <input
                        type="text"
                        class="agent-new-bundle-modal-input"
                        autofocus
                        placeholder={props.template.name}
                        value={name()}
                        onInput={(e) => setName(e.currentTarget.value)}
                        onKeyDown={onKeyDown}
                        maxlength={200}
                        disabled={submitting()}
                        data-testid="create-from-template-name-input"
                    />
                </label>
                <label class="agent-new-bundle-modal-field">
                    <span class="agent-new-bundle-modal-label">Identity</span>
                    <select
                        class="agent-new-bundle-modal-input"
                        value={identityId()}
                        onChange={(e) => setIdentityId(e.currentTarget.value)}
                        disabled={submitting()}
                        data-testid="create-from-template-identity-select"
                    >
                        <option value="">(ambient credentials)</option>
                        <For each={realIdentities()}>
                            {(b) => <option value={b.id}>{b.name}</option>}
                        </For>
                    </select>
                </label>
                <label class="agent-new-bundle-modal-field">
                    <span class="agent-new-bundle-modal-label">Memory</span>
                    <select
                        class="agent-new-bundle-modal-input"
                        value={memoryId()}
                        onChange={(e) => setMemoryId(e.currentTarget.value)}
                        disabled={submitting()}
                        data-testid="create-from-template-memory-select"
                    >
                        <option value="">(vanilla CLI)</option>
                        <For each={realMemories()}>
                            {(m) => <option value={m.id}>{m.name}</option>}
                        </For>
                    </select>
                </label>
                <Show when={error()}>
                    <div class="agent-new-bundle-modal-error" data-testid="create-from-template-error">
                        {error()}
                    </div>
                </Show>
            </div>
            <footer class="modal-panel-footer">
                <Button onClick={() => props.onCancel()} disabled={submitting()} data-modal-dismiss>
                    Cancel
                </Button>
                <Button
                    onClick={() => void submit()}
                    className="green solid"
                    disabled={!canSubmit()}
                    data-testid="create-from-template-submit"
                >
                    {submitting() ? "Creating…" : "Create"}
                </Button>
            </footer>
        </>
    );
};

AgentCreateFromTemplateModalPanel.displayName = "AgentCreateFromTemplateModalPanel";
