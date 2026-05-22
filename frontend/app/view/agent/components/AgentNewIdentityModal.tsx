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

export interface NewIdentityFormData {
    name: string;
    description: string;
}

interface AgentNewIdentityModalPanelProps {
    /** Suggested initial name (often empty). */
    initialName?: string;
    onCancel: () => void;
    /** Runs the RPC + downstream chaining. Owned by the layer so its
     *  `submitting()` signal correctly gates ESC / backdrop while the
     *  RPC is in flight (mirrors the Launch modal pattern). Reagent
     *  P1 on PR #911 — panel-local submitting wasn't visible to the
     *  layer's safeClose, so users could ESC mid-RPC and the resolved
     *  promise would re-open Launch unexpectedly. */
    onSubmit: (formData: NewIdentityFormData) => Promise<void>;
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
            await props.onSubmit({
                name: name().trim(),
                description: description().trim(),
            });
            // On success the layer unmounts this panel via the
            // request's chained tabModal.replace. The reset is for
            // the fragile case where the caller's chain doesn't fire.
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
        // Escape handled by the layer's overlay handler so the
        // submitting() gate (layer-level) takes effect uniformly.
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
                <Button onClick={() => props.onCancel()} disabled={submitting()} data-modal-dismiss>
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
