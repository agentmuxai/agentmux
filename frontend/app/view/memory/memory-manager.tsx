// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// MemoryManager — the context-free Memory-bundle management UI.
//
// This is the full list / create / edit / delete lifecycle for Memory
// bundles, extracted out of the `view: "memory"` block pane so the exact
// same UI can render in two places without depending on the Agent-pane
// block, `nodeModel`, or any ViewModel-from-BlockRegistry context:
//
//   1. The existing `view: "memory"` settings pane — `memory-view.tsx`
//      renders <MemoryManagerBody/> with the pane's BlockRegistry model.
//   2. The window-scoped bundle manager modal (a later PR) — renders
//      <MemoryManager/>, which owns its own block-free model.
//
// Everything here drives purely off the `bundle_*` RPCs (via
// MemoryViewModel) plus the `memories:changed` WPS event, so two live
// instances stay consistent for free.

import { For, onCleanup, Show, type JSX } from "solid-js";

import { PrimitiveListDetail } from "@/app/element/primitive-list-detail";
import { useModalLayer } from "@/app/element/modal-layer";
import { type MemoryDraft, MemoryViewModel } from "./memory-model";

import "./memory-view.scss";

interface MemoryManagerBodyProps {
    model: MemoryViewModel;
}

/**
 * MemoryManagerBody — the rail + detail UI, driven by a MemoryViewModel.
 *
 * This component is context-free: it reads and writes ONLY through the
 * `model` accessors/methods, none of which require a block. Both the
 * settings-pane wrapper and the standalone <MemoryManager/> render this
 * with their respective models, so the markup lives in exactly one place.
 */
const MemoryManagerBody = (props: MemoryManagerBodyProps): JSX.Element => {
    const { model } = props;
    const modalLayer = useModalLayer();

    // Starts the 3-step ABF import chain (SPEC_ABF_IMPORT_UI_PHASE3_2026_08_02.md
    // §4) -- each step's request builds the next via modalLayer.replace(),
    // mirroring the install→launch chain precedent. The commit RPC
    // publishes `memories:changed`, which this model already subscribes
    // to, so the newly-imported bundle appears in the list with no extra
    // glue here.
    const handleImportBundle = () => {
        modalLayer.open({
            kind: "bundle-import-select",
            onPreviewed: (filePath, preview) => {
                modalLayer.replace({
                    kind: "bundle-import-preview",
                    filePath,
                    preview,
                    onNext: (selection) => {
                        modalLayer.replace({
                            kind: "bundle-import-confirm",
                            filePath,
                            contentDigest: preview.content_digest,
                            bundleDisplayName: selection.bundleName,
                            selection,
                            onImported: () => modalLayer.close(),
                            onCancel: modalLayer.close,
                        });
                    },
                    onCancel: modalLayer.close,
                });
            },
            onCancel: modalLayer.close,
        });
    };

    // Clicking a list item SELECTS the memory so the read-only detail
    // view appears (including for the blank singleton, which can't be
    // edited but should still be inspectable). The edit form opens via
    // the explicit "Edit" button on the read-only view. Reagent P2
    // (#749) — previously this called startEdit which refused on the
    // blank singleton with an error and left the detail pane showing
    // the previously-selected memory, mismatching the banner.
    const handleSelect = (memory: Memory) => {
        model.setError(null);
        model.cancelDraft();
        model.setSelectedId(memory.id);
    };

    const handleNew = () => model.startNew();

    const handleSave = () => {
        void model.saveDraft();
    };

    const handleCancel = () => model.cancelDraft();

    const handleDelete = (id: string) => {
        // Confirm via the most boring possible dialog. Presets can be
        // referenced by running instances; deletion is an explicit user
        // intent we don't want to fast-path.
        const ok = window.confirm("Delete this preset? Running instances continue with their snapshot, but new launches won't see it.");
        if (!ok) return;
        void model.deleteMemory(id);
    };

    const updateDraft = <K extends keyof MemoryDraft>(
        key: K,
        value: MemoryDraft[K],
    ) => {
        const current = model.draftAtom();
        if (!current) return;
        model.setDraft({ ...current, [key]: value });
    };

    // Single-pane: the detail view (read-only or edit form) shows instead of
    // the list, never alongside it — see
    // docs/specs/SPEC_ARMORY_RESPONSIVE_SINGLE_PANE_LAYOUT_2026_07_15.md.
    // "In detail" whenever something is selected OR a draft (new or
    // editing-existing) is open; otherwise the list shows. The old
    // no-selection empty-state message ("Select a preset...") is gone — with
    // nothing selected you're just looking at the list, there's no third
    // empty detail state to explain.
    const inDetail = () => model.selectedIdAtom() !== null || model.draftAtom() !== null;

    const handleBack = () => {
        model.setError(null);
        model.setSelectedId(null);
        model.setDraft(null);
    };

    const listView = (
        <div class="memory-view-rail">
            <Show when={model.errorAtom()}>
                <div class="memory-view-error">{model.errorAtom()}</div>
            </Show>
            <div class="memory-view-rail-header">
                <button class="memory-view-new-btn" onClick={handleNew}>
                    + New Preset
                </button>
                <button class="memory-view-new-btn" onClick={handleImportBundle}>
                    Import Bundle
                </button>
            </div>
            <ul class="memory-view-list">
                <For each={model.memoriesAtom()}>
                    {(memory) => (
                        <li
                            class="memory-view-list-item"
                            classList={{
                                "is-selected": model.selectedIdAtom() === memory.id,
                                "is-blank": !!memory.is_blank,
                                "is-global": !!memory.is_global,
                            }}
                            onClick={() => handleSelect(memory)}
                        >
                            <div class="memory-view-list-item-name">
                                <span class="memory-view-list-item-name-text">
                                    {memory.is_blank ? "— Blank (vanilla CLI) —" : memory.name}
                                </span>
                                <Show when={memory.is_global}>
                                    <span class="memory-view-global-badge" title="Injected into all agents at launch">Global</span>
                                </Show>
                            </div>
                            {/* Subtitle shows the description now that presets
                                are provider-agnostic (§4.1a). Class name kept
                                to avoid CSS churn. */}
                            <Show when={!memory.is_blank && memory.description}>
                                <div class="memory-view-list-item-provider">
                                    {memory.description}
                                </div>
                            </Show>
                        </li>
                    )}
                </For>
            </ul>
        </div>
    );

    const detailView = (
        <div class="memory-view-detail">
            <Show when={model.errorAtom()}>
                <div class="memory-view-error">{model.errorAtom()}</div>
            </Show>

            <Show
                when={model.draftAtom()}
                fallback={
                    <Show when={model.selectedAtom()}>
                        {(memory) => (
                            <div class="memory-view-readonly">
                                <h2 class="memory-view-name">{memory().name}</h2>
                                <Show when={memory().description}>
                                    <p class="memory-view-description">{memory().description}</p>
                                </Show>
                                <dl class="memory-view-fields">
                                    <Show when={memory().is_global}>
                                        <dt>Scope</dt>
                                        <dd>
                                            <span class="memory-view-global-badge" title="Injected into all agents at launch">Global</span>
                                            {" "}— injected into every agent at launch
                                        </dd>
                                    </Show>
                                    <dt>Instructions</dt>
                                    <dd class="memory-view-instructions-readonly">
                                        <pre>{memory().instructions || "(none)"}</pre>
                                    </dd>
                                </dl>
                                <Show when={!memory().is_blank}>
                                    <div class="memory-view-actions">
                                        <button
                                            class="memory-view-edit-btn"
                                            onClick={() => model.startEdit(memory())}
                                        >
                                            Edit
                                        </button>
                                        <button
                                            class="memory-view-delete-btn"
                                            onClick={() => handleDelete(memory().id)}
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
                            class="memory-view-form"
                            onSubmit={(e) => {
                                e.preventDefault();
                                handleSave();
                            }}
                        >
                            <h2 class="memory-view-form-title">
                                {draft().id ? "Edit Preset" : "New Preset"}
                            </h2>

                            <label class="memory-view-field">
                                <span class="memory-view-field-label">Name *</span>
                                <input
                                    class="memory-view-input"
                                    type="text"
                                    value={draft().name}
                                    onInput={(e) => updateDraft("name", e.currentTarget.value)}
                                    placeholder="e.g. Claude-coder"
                                    required
                                />
                            </label>

                            <label class="memory-view-field">
                                <span class="memory-view-field-label">Description</span>
                                <input
                                    class="memory-view-input"
                                    type="text"
                                    value={draft().description}
                                    onInput={(e) =>
                                        updateDraft("description", e.currentTarget.value)
                                    }
                                    placeholder="Short label shown in the launch picker"
                                />
                            </label>

                            {/* Provider + model intentionally omitted: presets are
                                provider-agnostic; the CLI + model belong to the
                                agent. See SPEC_MEMORY_IDENTITY_ARCH §4.1a. */}

                            <label class="memory-view-field">
                                <span class="memory-view-field-label">Instructions</span>
                                <textarea
                                    class="memory-view-textarea"
                                    rows={8}
                                    value={draft().instructions}
                                    onInput={(e) =>
                                        updateDraft("instructions", e.currentTarget.value)
                                    }
                                    placeholder="System prompt. The agent's soul."
                                />
                            </label>

                            <div class="memory-view-form-actions">
                                <button
                                    type="button"
                                    class="memory-view-cancel-btn"
                                    onClick={handleCancel}
                                    disabled={model.savingAtom()}
                                >
                                    Cancel
                                </button>
                                <button
                                    type="submit"
                                    class="memory-view-save-btn"
                                    disabled={model.savingAtom() || !draft().name.trim()}
                                >
                                    {model.savingAtom() ? "Saving…" : "Save"}
                                </button>
                            </div>

                            <p class="memory-view-form-hint">
                                Context files, MCP servers, and skills will be editable in a follow-up — the
                                fields are persisted as JSON today and round-trip cleanly through the form.
                            </p>
                        </form>
                    )}
            </Show>
        </div>
    );

    return (
        <PrimitiveListDetail
            showDetail={inDetail()}
            backLabel="Bundles"
            onBack={handleBack}
            list={listView}
            detail={detailView}
        />
    );
};

/**
 * MemoryManager — context-free standalone Memory-bundle manager.
 *
 * Constructs its OWN block-free MemoryViewModel and renders the shared
 * body. Use this wherever Memory CRUD is needed outside an Agent-pane
 * block — e.g. the window-scoped bundle manager modal. Takes no props.
 *
 * The model's only block dependency was the cosmetic header title; with
 * the optional-constructor change in memory-model.ts it constructs
 * cleanly with no block and drives entirely off the `bundle_*` RPCs.
 */
export const MemoryManager = (): JSX.Element => {
    // Component setup runs once; the model lives for this component's
    // lifetime and is disposed on unmount.
    const model = new MemoryViewModel();
    onCleanup(() => model.dispose());
    return <MemoryManagerBody model={model} />;
};
