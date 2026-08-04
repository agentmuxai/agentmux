// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentAddAccountModalPanel — adds a manual/API-key account from the
 * Launch modal's "+ Add account" affordance. Issue #1624 PR-C Part B:
 * accounts are provider-scoped (unlike the old provider-agnostic
 * Identity bundle this modal used to create), so the form only needs
 * a name + API key for the caller-supplied provider. OAuth Connect no
 * longer routes through this modal — it starts directly from the
 * launch modal's auth panel, which mints the account backend-side.
 *
 * Mirrors the key-entry flow in identity-view.tsx's Armory Accounts
 * tab (`AccountKeyVerifyCommand`, `validate: true`).
 */

import { createSignal, type JSX } from "solid-js";

import { Button } from "@/element/button";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

interface AddAccountFormData {
    accountId: string;
}

interface AgentAddAccountModalPanelProps {
    /** Provider the new account is scoped to. */
    provider: string;
    /** Suggested initial name (often empty). */
    initialName?: string;
    onCancel: () => void;
    /** Runs the RPC + downstream chaining. Owned by the layer so its
     *  `submitting()` signal correctly gates ESC / backdrop while the
     *  RPC is in flight (mirrors the Launch modal pattern). */
    onSubmit: (formData: AddAccountFormData) => Promise<void>;
}

export const AgentAddAccountModalPanel = (
    props: AgentAddAccountModalPanelProps,
): JSX.Element => {
    const [name, setName] = createSignal(props.initialName ?? "");
    const [apiKey, setApiKey] = createSignal("");
    const [submitting, setSubmitting] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);

    const canSubmit = () =>
        name().trim().length > 0 && apiKey().trim().length > 0 && !submitting();

    const submit = async () => {
        if (!canSubmit()) return;
        setSubmitting(true);
        setError(null);
        try {
            const res = await RpcApi.AccountKeyVerifyCommand(TabRpcClient, {
                provider: props.provider,
                name: name().trim(),
                apiKey: apiKey().trim(),
                validate: true,
            });
            if (!res.valid || !res.accountId) {
                setError(res.error ?? "Validation failed");
                setSubmitting(false);
                return;
            }
            // Drop the plaintext from the form immediately.
            setApiKey("");
            await props.onSubmit({ accountId: res.accountId });
            // On success the layer unmounts this panel via the
            // request's chained modalLayer.replace. The reset is for
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
                <h2 class="modal-panel-title">Add Account</h2>
                <p class="modal-panel-description">
                    Stores an API key in the OS keychain and validates it with a
                    single live probe.
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
                    <span class="agent-new-bundle-modal-label">API key</span>
                    <input
                        type="password"
                        class="agent-new-bundle-modal-input"
                        placeholder="sk-…"
                        value={apiKey()}
                        onInput={(e) => setApiKey(e.currentTarget.value)}
                        onKeyDown={onKeyDown}
                        disabled={submitting()}
                    />
                </label>
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
                    {submitting() ? "Validating…" : "Add"}
                </Button>
            </footer>
        </>
    );
};

AgentAddAccountModalPanel.displayName = "AgentAddAccountModalPanel";
