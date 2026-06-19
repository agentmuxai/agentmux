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
export const MemoryManagerBody = (props: MemoryManagerBodyProps): JSX.Element => {
    const { model } = props;

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
        // Confirm via the most boring possible dialog. Memory bundles can
        // be referenced by running instances; deletion is an explicit user
        // intent we don't want to fast-path.
        const ok = window.confirm("Delete this Memory bundle? Running instances continue with their snapshot, but new launches won't see it.");
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

    return (
        <div class="memory-view">
            <div class="memory-view-rail">
                <div class="memory-view-rail-header">
                    <button class="memory-view-new-btn" onClick={handleNew}>
                        + New Memory
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
                                    {memory.is_blank ? "— Blank (vanilla CLI) —" : memory.name}
                                    <Show when={memory.is_global}>
                                        <span class="memory-view-global-badge" title="Injected into all agents at launch">Global</span>
                                    </Show>
                                </div>
                                <Show when={!memory.is_blank}>
                                    <div class="memory-view-list-item-provider">
                                        {memory.provider || "no provider"}
                                    </div>
                                </Show>
                            </li>
                        )}
                    </For>
                </ul>
            </div>

            <div class="memory-view-detail">
                <Show when={model.errorAtom()}>
                    <div class="memory-view-error">{model.errorAtom()}</div>
                </Show>

                <Show
                    when={model.draftAtom()}
                    fallback={
                        <Show
                            when={model.selectedAtom()}
                            fallback={
                                <div class="memory-view-empty">
                                    <p>Select a Memory bundle from the list, or create a new one.</p>
                                    <p class="memory-view-empty-hint">
                                        A Memory holds an agent's CLI choice, model, system instructions,
                                        context files, MCP servers, and skills. Pick one at launch time
                                        alongside an Identity to compose an agent instance.
                                    </p>
                                </div>
                            }
                        >
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
                                        <dt>Provider</dt>
                                        <dd>{memory().provider || "—"}</dd>
                                        <dt>Model</dt>
                                        <dd>{memory().model || "—"}</dd>
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
                                {draft().id ? "Edit Memory" : "New Memory"}
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

                            <label class="memory-view-field">
                                <span class="memory-view-field-label">Provider</span>
                                <select
                                    class="memory-view-input"
                                    value={draft().provider}
                                    onChange={(e) =>
                                        updateDraft("provider", e.currentTarget.value)
                                    }
                                >
                                    <option value="">(none — vanilla shell)</option>
                                    <option value="claude">Claude Code</option>
                                    <option value="codex">Codex CLI</option>
                                    <option value="gemini">Gemini CLI</option>
                                    <option value="qwen">Qwen Code</option>
                                </select>
                            </label>

                            <label class="memory-view-field">
                                <span class="memory-view-field-label">Model</span>
                                <input
                                    class="memory-view-input"
                                    type="text"
                                    value={draft().model}
                                    onInput={(e) => updateDraft("model", e.currentTarget.value)}
                                    placeholder="e.g. claude-sonnet-4-6"
                                />
                            </label>

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
        </div>
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
