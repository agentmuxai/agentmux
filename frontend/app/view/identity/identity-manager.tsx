// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// IdentityManager — the context-free Identity-bundle management UI.
//
// This is the full list / create / edit / delete / per-provider
// binding lifecycle for Identity bundles, extracted out of the
// `view: "identity"` block pane so the exact same UI can render in two
// places without depending on the Agent-pane block, `nodeModel`, or any
// ViewModel-from-BlockRegistry context:
//
//   1. The existing `view: "identity"` settings pane —
//      `identity-pane-view.tsx` renders <IdentityManagerBody/> with the
//      pane's BlockRegistry model.
//   2. The window-scoped bundle manager modal (a later PR) — renders
//      <IdentityManager/>, which owns its own block-free model.
//
// Everything here drives purely off the `bundle_*` RPCs (via
// IdentityPaneViewModel) plus the account-change broadcaster, so two
// live instances stay consistent for free.

import { For, onCleanup, Show, type JSX } from "solid-js";

import { IdentityPaneViewModel } from "./identity-pane-model";

import "./identity-pane-view.scss";

interface IdentityManagerBodyProps {
    model: IdentityPaneViewModel;
}

/**
 * IdentityManagerBody — the rail + detail + bindings UI, driven by an
 * IdentityPaneViewModel.
 *
 * This component is context-free: it reads and writes ONLY through the
 * `model` accessors/methods, none of which require a block. Both the
 * settings-pane wrapper and the standalone <IdentityManager/> render
 * this with their respective models, so the markup lives in exactly
 * one place.
 */
export const IdentityManagerBody = (props: IdentityManagerBodyProps): JSX.Element => {
    const { model } = props;

    // Clicking a list item SELECTS the bundle so the binding table
    // becomes visible. The metadata draft (name + description) opens
    // only via the explicit "Edit name / description" button on the
    // detail view. Reagent + codex P1 / P2 (#748): the previous
    // handler called startEdit which auto-opened the form, blocking
    // the binding table; clicking the blank singleton would surface
    // an error and leave the right panel in the empty state.
    const handleSelect = (bundle: IdentityBundle) => {
        model.setError(null);
        model.cancelDraft();
        model.setSelectedId(bundle.id);
    };
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
                                                                            .bindingsAtom()
                                                                            .find(
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

/**
 * IdentityManager — context-free standalone Identity-bundle manager.
 *
 * Constructs its OWN block-free IdentityPaneViewModel and renders the
 * shared body. Use this wherever Identity CRUD is needed outside an
 * Agent-pane block — e.g. the window-scoped bundle manager modal. Takes
 * no props.
 *
 * The model's only block dependency was the cosmetic header title; with
 * the optional-constructor change in identity-pane-model.ts it
 * constructs cleanly with no block and drives entirely off the
 * `bundle_*` RPCs and the account-change broadcaster.
 */
export const IdentityManager = (): JSX.Element => {
    // Component setup runs once; the model lives for this component's
    // lifetime and is disposed on unmount.
    const model = new IdentityPaneViewModel();
    onCleanup(() => model.dispose());
    return <IdentityManagerBody model={model} />;
};

// Local alias mirroring `IdentityBundleDraft` from the model — see the
// memory-view.tsx history note for why we don't import it directly.
type DraftShape = {
    id?: string;
    name: string;
    description: string;
};
