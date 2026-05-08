// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Identity pane view — left rail (bundle list + add) + right detail
// (bundle name/description + per-provider account-binding table).
//
// Compared to the Memory pane, the right side has two major sections:
// the editable bundle metadata, and a binding table where each provider
// row carries a dropdown of available accounts.

import { For, Show, type JSX } from "solid-js";

import type { IdentityPaneViewModel } from "./identity-pane-model";

import "./identity-pane-view.scss";

interface IdentityPaneViewProps {
    model: IdentityPaneViewModel;
}

export const IdentityPaneView = (props: IdentityPaneViewProps): JSX.Element => {
    const { model } = props;

    const handleSelect = (bundle: IdentityBundle) => model.startEdit(bundle);
    const handleNew = () => model.startNew();
    const handleSave = () => void model.saveDraft();
    const handleCancel = () => model.cancelDraft();

    const handleDelete = (id: string) => {
        const ok = window.confirm(
            "Delete this Identity bundle? Running instances continue with their snapshot, but new launches won't see it.",
        );
        if (!ok) return;
        void model.deleteBundle(id);
    };

    const updateDraft = <K extends keyof DraftShape>(key: K, value: DraftShape[K]) => {
        const cur = model.draftAtom();
        if (!cur) return;
        model.setDraft({ ...cur, [key]: value });
    };

    return (
        <div class="identity-pane">
            <div class="identity-pane-rail">
                <div class="identity-pane-rail-header">
                    <button class="identity-pane-new-btn" onClick={handleNew}>
                        + New Identity
                    </button>
                </div>
                <ul class="identity-pane-list">
                    <For each={model.bundlesAtom()}>
                        {(bundle) => (
                            <li
                                class="identity-pane-list-item"
                                classList={{
                                    "is-selected": model.selectedIdAtom() === bundle.id,
                                    "is-blank": !!bundle.is_blank,
                                }}
                                onClick={() => handleSelect(bundle)}
                            >
                                <div class="identity-pane-list-item-name">
                                    {bundle.is_blank ? "— Blank (no creds) —" : bundle.name}
                                </div>
                                <Show when={bundle.description && !bundle.is_blank}>
                                    <div class="identity-pane-list-item-desc">
                                        {bundle.description}
                                    </div>
                                </Show>
                            </li>
                        )}
                    </For>
                </ul>
            </div>

            <div class="identity-pane-detail">
                <Show when={model.errorAtom()}>
                    <div class="identity-pane-error">{model.errorAtom()}</div>
                </Show>

                <Show
                    when={model.draftAtom()}
                    fallback={
                        <Show
                            when={model.selectedBundleAtom()}
                            fallback={
                                <div class="identity-pane-empty">
                                    <p>Select an Identity bundle from the list, or create a new one.</p>
                                    <p class="identity-pane-empty-hint">
                                        An Identity bundles together one account per provider —
                                        e.g. your work GitHub PAT plus a work Anthropic key — so you
                                        can pick the whole bundle at launch instead of selecting
                                        accounts one by one.
                                    </p>
                                </div>
                            }
                        >
                            {(bundle) => (
                                <div class="identity-pane-readonly">
                                    <h2 class="identity-pane-name">{bundle().name}</h2>
                                    <Show when={bundle().description}>
                                        <p class="identity-pane-description">{bundle().description}</p>
                                    </Show>

                                    <h3 class="identity-pane-section-title">Bound accounts</h3>
                                    <Show
                                        when={model.providersForBindingRows().length > 0}
                                        fallback={
                                            <p class="identity-pane-empty-hint">
                                                No accounts available yet — create some via the
                                                Identity tab in any agent pane to bind them here.
                                            </p>
                                        }
                                    >
                                        <table class="identity-pane-bindings">
                                            <thead>
                                                <tr>
                                                    <th>Provider</th>
                                                    <th>Account</th>
                                                </tr>
                                            </thead>
                                            <tbody>
                                                <For each={model.providersForBindingRows()}>
                                                    {(provider) => (
                                                        <tr>
                                                            <td>{model.providerLabel(provider)}</td>
                                                            <td>
                                                                <select
                                                                    class="identity-pane-input"
                                                                    value={
                                                                        model
                                                                            .bindingsResource[0]()
                                                                            ?.find(
                                                                                (b) =>
                                                                                    b.provider ===
                                                                                    provider,
                                                                            )?.account_id ?? ""
                                                                    }
                                                                    disabled={!!bundle().is_blank}
                                                                    onChange={(e) =>
                                                                        void model.setBinding(
                                                                            provider,
                                                                            e.currentTarget.value,
                                                                        )
                                                                    }
                                                                >
                                                                    <option value="">— None —</option>
                                                                    <For
                                                                        each={
                                                                            model
                                                                                .accountsByProvider()
                                                                                .get(provider) ?? []
                                                                        }
                                                                    >
                                                                        {(acc) => (
                                                                            <option value={acc.id}>
                                                                                {acc.display_name?.trim() ||
                                                                                    acc.name}
                                                                            </option>
                                                                        )}
                                                                    </For>
                                                                </select>
                                                            </td>
                                                        </tr>
                                                    )}
                                                </For>
                                            </tbody>
                                        </table>
                                    </Show>

                                    <Show when={!bundle().is_blank}>
                                        <div class="identity-pane-actions">
                                            <button
                                                class="identity-pane-edit-btn"
                                                onClick={() => model.startEdit(bundle())}
                                            >
                                                Edit name / description
                                            </button>
                                            <button
                                                class="identity-pane-delete-btn"
                                                onClick={() => handleDelete(bundle().id)}
                                            >
                                                Delete
                                            </button>
                                        </div>
                                    </Show>
                                </div>
                            )}
                        </Show>
                    }
                >
                    {(draft) => (
                        <form
                            class="identity-pane-form"
                            onSubmit={(e) => {
                                e.preventDefault();
                                handleSave();
                            }}
                        >
                            <h2 class="identity-pane-form-title">
                                {draft().id ? "Edit Identity" : "New Identity"}
                            </h2>

                            <label class="identity-pane-field">
                                <span class="identity-pane-field-label">Name *</span>
                                <input
                                    class="identity-pane-input"
                                    type="text"
                                    value={draft().name}
                                    onInput={(e) => updateDraft("name", e.currentTarget.value)}
                                    placeholder="e.g. Work"
                                    required
                                />
                            </label>

                            <label class="identity-pane-field">
                                <span class="identity-pane-field-label">Description</span>
                                <input
                                    class="identity-pane-input"
                                    type="text"
                                    value={draft().description}
                                    onInput={(e) =>
                                        updateDraft("description", e.currentTarget.value)
                                    }
                                    placeholder="Short label shown in the launch picker"
                                />
                            </label>

                            <div class="identity-pane-form-actions">
                                <button
                                    type="button"
                                    class="identity-pane-cancel-btn"
                                    onClick={handleCancel}
                                    disabled={model.savingAtom()}
                                >
                                    Cancel
                                </button>
                                <button
                                    type="submit"
                                    class="identity-pane-save-btn"
                                    disabled={model.savingAtom() || !draft().name.trim()}
                                >
                                    {model.savingAtom() ? "Saving…" : "Save"}
                                </button>
                            </div>

                            <p class="identity-pane-form-hint">
                                After saving, you'll be able to bind accounts per provider on the
                                detail view.
                            </p>
                        </form>
                    )}
                </Show>
            </div>
        </div>
    );
};

// Local alias mirroring `IdentityBundleDraft` from the model — see the
// memory-view.tsx note for why we don't import it directly.
type DraftShape = {
    id?: string;
    name: string;
    description: string;
};
