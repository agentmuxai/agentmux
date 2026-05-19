// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentNewIdentityModalPanel — creates an empty Identity bundle from
 * the Launch modal's "+ New" affordance. Identity bundles are generic
 * credential containers (Claude / Codex / Gemini / OpenClaw OAuth +
 * GitHub + AWS + …), NOT provider-scoped — so the create form is
 * deliberately small: just name + description. Connector setup
 * happens later in the Identity pane.
 *
 * Phase β of SPEC_LAUNCH_MODAL_PROFILE_SECTION_2026_05_18.md.
 */

import { createSignal, type JSX } from "solid-js";

import { Button } from "@/element/button";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

interface AgentNewIdentityModalPanelProps {
    /** Suggested initial name (often empty). */
    initialName?: string;
    onCancel: () => void;
    /** Fires after the bundle is persisted. Caller passes the new id
     *  to the Launch modal so it auto-selects on next render. */
    onCreated: (bundleId: string, bundleName: string) => void;
}

export const AgentNewIdentityModalPanel = (
    props: AgentNewIdentityModalPanelProps,
): JSX.Element => {
    const [name, setName] = createSignal(props.initialName ?? "");
    const [description, setDescription] = createSignal("");
    const [submitting, setSubmitting] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);

    const canSubmit = () => name().trim().length > 0 && !submitting();

    const submit = async () => {
        if (!canSubmit()) return;
        setSubmitting(true);
        setError(null);
        try {
            const id = crypto.randomUUID();
            const now = Date.now();
            const bundle = await RpcApi.UpsertIdentityBundleCommand(TabRpcClient, {
                id,
                name: name().trim(),
                description: description().trim(),
                is_blank: false,
                created_at: now,
                updated_at: now,
            });
            props.onCreated(bundle.id, bundle.name);
            // Reset on success too. In practice the caller unmounts
            // this panel via tabModal.replace, so the next render
            // never observes the reset value — but leaving the flag
            // stuck at true is a fragile invariant (reagent P2 on
            // PR #910): a future caller that fails to replace the
            // modal would strand the inputs disabled.
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
        } else if (e.key === "Escape") {
            e.preventDefault();
            props.onCancel();
        }
    };

    return (
        <>
            <header class="modal-panel-header">
                <h2 class="modal-panel-title">New Identity</h2>
                <p class="modal-panel-description">
                    An Identity holds credentials your agent uses — Claude /
                    Codex / Gemini / OpenClaw OAuth + GitHub / AWS / etc.
                    Same Identity works across providers.
                </p>
            </header>
            <div class="modal-panel-body agent-new-bundle-modal-body">
                <label class="agent-new-bundle-modal-field">
                    <span class="agent-new-bundle-modal-label">Name</span>
                    <input
                        type="text"
                        class="agent-new-bundle-modal-input"
                        autofocus
                        placeholder="Work, Personal, Client X, …"
                        value={name()}
                        onInput={(e) => setName(e.currentTarget.value)}
                        onKeyDown={onKeyDown}
                        disabled={submitting()}
                    />
                </label>
                <label class="agent-new-bundle-modal-field">
                    <span class="agent-new-bundle-modal-label">
                        Description <span class="agent-new-bundle-modal-optional">(optional)</span>
                    </span>
                    <input
                        type="text"
                        class="agent-new-bundle-modal-input"
                        placeholder="What's this Identity for?"
                        value={description()}
                        onInput={(e) => setDescription(e.currentTarget.value)}
                        onKeyDown={onKeyDown}
                        disabled={submitting()}
                    />
                </label>
                <p class="agent-new-bundle-modal-hint">
                    You'll add connections (Claude, GitHub, AWS, …) in the
                    Identity pane after creating this bundle.
                </p>
                {error() && (
                    <div class="agent-new-bundle-modal-error">{error()}</div>
                )}
            </div>
            <footer class="modal-panel-footer">
                <Button onClick={() => props.onCancel()} disabled={submitting()}>
                    Cancel
                </Button>
                <Button
                    onClick={() => void submit()}
                    className="green solid"
                    disabled={!canSubmit()}
                >
                    {submitting() ? "Creating…" : "Create"}
                </Button>
            </footer>
        </>
    );
};

AgentNewIdentityModalPanel.displayName = "AgentNewIdentityModalPanel";
