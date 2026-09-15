// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// GlobalBundleManager — the Global section of the Armory "Memory" tab (moves to
// the Bundles tab in Phase 4 of SPEC_ARMORY_NAMING_CONSOLIDATION_2026_09_09.md).
// Presents the workspace-wide global bundles (is_global rows) as an ordered
// list of editable sections that compose into every agent's startup
// instructions file (CLAUDE.md, GEMINI.md, or similar, depending on
// provider) at launch.
//
// Context-free: owns its own GlobalBundleViewModel and drives off the
// bundle_* RPCs. Spec: docs/specs/archive/SPEC_TRUST_CENTER_GLOBAL_BRAIN_2026_06_19.md.
//
// Layout restructured per docs/specs/SPEC_ARMORY_GLOBAL_MEMORY_DECLUTTER_
// 2026_09_15.md: the Claude Code reference file, system-tier entries, and
// ordinary sections used to each render with their own divergent chrome, and
// none of them showed their content outside an "Edit" click. They now share
// one `.global-bundle-file` row shape (label, then an always-visible
// markdown preview) in a single list, and the "applies to" filename-mapping
// block is gone from this view entirely (per that spec's §3 — the user did
// not want it here).

import { createSignal, For, onCleanup, Show, type JSX } from "solid-js";
import { Markdown } from "@/app/element/markdown";
import { showTextInputContextMenu } from "@/app/store/contextmenu";
import { GlobalBundleViewModel, NEW_SECTION_ID } from "./global-bundle-model";
import "./global-bundle.scss";

/** Inline editor card shared by the "new section" and "edit section" flows. */
function SectionEditor(props: { model: GlobalBundleViewModel; isNew: boolean }): JSX.Element {
    const { model } = props;
    return (
        <div class="global-bundle-editor">
            <label class="global-bundle-field">
                <span class="global-bundle-field-label">Name</span>
                <input
                    class="global-bundle-input"
                    type="text"
                    value={model.draftNameAtom()}
                    onInput={(e) => model.setDraftName(e.currentTarget.value)}
                    onContextMenu={showTextInputContextMenu}
                    placeholder="e.g. Coding Standards"
                />
            </label>
            <label class="global-bundle-field">
                <span class="global-bundle-field-label">Content</span>
                <textarea
                    class="global-bundle-textarea"
                    rows={8}
                    value={model.draftInstructionsAtom()}
                    onInput={(e) => model.setDraftInstructions(e.currentTarget.value)}
                    onContextMenu={showTextInputContextMenu}
                    placeholder="Markdown injected into every agent's startup instructions file under a # [Workspace] heading."
                    spellcheck={false}
                />
            </label>
            <div class="global-bundle-editor-actions">
                <button
                    class="global-bundle-btn"
                    disabled={model.savingAtom()}
                    onClick={() => model.cancelEdit()}
                >
                    Cancel
                </button>
                <button
                    class="global-bundle-btn global-bundle-btn-primary"
                    disabled={model.savingAtom() || !model.draftNameAtom().trim()}
                    onClick={() => void model.saveEdit()}
                >
                    {model.savingAtom() ? "Saving…" : props.isNew ? "Add section" : "Save"}
                </button>
            </div>
        </div>
    );
}

/** Inline editor card for the system tier — a separate component (not a
 *  parameterized SectionEditor) so its state is never accidentally wired to
 *  the ordinary draft signals. See
 *  docs/specs/SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md §3.5. */
function SystemSectionEditor(props: { model: GlobalBundleViewModel; isNew: boolean }): JSX.Element {
    const { model } = props;
    return (
        <div class="global-bundle-editor">
            <label class="global-bundle-field">
                <span class="global-bundle-field-label">Name</span>
                <input
                    class="global-bundle-input"
                    type="text"
                    value={model.draftSystemNameAtom()}
                    onInput={(e) => model.setDraftSystemName(e.currentTarget.value)}
                    onContextMenu={showTextInputContextMenu}
                    placeholder="e.g. Global Memory Policy"
                />
            </label>
            <label class="global-bundle-field">
                <span class="global-bundle-field-label">Content</span>
                <textarea
                    class="global-bundle-textarea"
                    rows={8}
                    value={model.draftSystemInstructionsAtom()}
                    onInput={(e) => model.setDraftSystemInstructions(e.currentTarget.value)}
                    onContextMenu={showTextInputContextMenu}
                    placeholder="Markdown injected FIRST into every agent's startup instructions file, wrapped in explicit override wording."
                    spellcheck={false}
                />
            </label>
            <div class="global-bundle-editor-actions">
                <button
                    class="global-bundle-btn"
                    disabled={model.savingAtom()}
                    onClick={() => model.cancelEditSystem()}
                >
                    Cancel
                </button>
                <button
                    class="global-bundle-btn global-bundle-btn-primary"
                    disabled={model.savingAtom() || !model.draftSystemNameAtom().trim()}
                    onClick={() => void model.saveSystemEdit()}
                >
                    {model.savingAtom() ? "Saving…" : props.isNew ? "Add system entry" : "Save"}
                </button>
            </div>
        </div>
    );
}

export const GlobalBundleManager = (): JSX.Element => {
    const model = new GlobalBundleViewModel();
    onCleanup(() => model.dispose());

    const [promoteValue, setPromoteValue] = createSignal("");

    const handlePromote = (id: string) => {
        if (!id) return;
        void model.promote(id);
        setPromoteValue("");
    };

    return (
        <div class="global-bundle">
            <p class="global-bundle-intro">
                Every agent inherits this at launch — takes effect after a restart.
            </p>

            <Show when={model.errorAtom()}>
                <div class="global-bundle-error">{model.errorAtom()}</div>
            </Show>

            <div class="global-bundle-files">
                {/* Read-only reference display of the CLAUDE.md in the
                    isolated config dir a default spawned agent actually
                    launches with (CLAUDE_CONFIG_DIR). NOT part of AgentMux's
                    own Global Memory composition (that's <agent
                    working_directory>/CLAUDE.md, previewed accurately below
                    via "Combined preview"). See
                    docs/specs/SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md §7. */}
                <Show when={model.claudeGlobalConfigAtom()}>
                    {(cfg) => (
                        <div class="global-bundle-file global-bundle-file-readonly">
                            <div class="global-bundle-file-header">
                                <code
                                    class="global-bundle-file-label"
                                    title="Claude Code — shared provider config. Used by default spawned agents; identity-bound agents use a separate dir, not shown here."
                                >
                                    {cfg().path}
                                </code>
                            </div>
                            <Show
                                when={cfg().exists}
                                fallback={<p class="global-bundle-file-empty">No file at this path yet.</p>}
                            >
                                {/* The resize handle lives on this wrapper, not on
                                    <Markdown> itself — Markdown's own root sets
                                    `height: 100%; overflow: hidden`, which would
                                    fight a resize/height override applied
                                    directly to it. Markdown fills 100% of
                                    whatever height this wrapper resizes to and
                                    handles its own internal scrolling
                                    (nativeScrollbar: a plain CSS scrollbar is
                                    plenty for a reference-only preview panel). */}
                                <div class="global-bundle-file-content">
                                    <Markdown
                                        text={cfg().content}
                                        scrollable={true}
                                        nativeScrollbar={true}
                                        contentClass="global-bundle-file-markdown-content"
                                    />
                                </div>
                            </Show>
                        </div>
                    )}
                </Show>

                {/* System tier — pinned above ordinary sections, always
                    injected first with override wording. No move up/down:
                    position is fixed server-side regardless of what a
                    reorder call sends. See
                    docs/specs/SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md. */}
                <For each={model.systemSectionsAtom()}>
                    {(section) => (
                        <div
                            class="global-bundle-file global-bundle-file-system"
                            classList={{ "is-editing": model.editingSystemIdAtom() === section.id }}
                        >
                            <Show
                                when={model.editingSystemIdAtom() === section.id}
                                fallback={
                                    <>
                                        <div class="global-bundle-file-header">
                                            <span class="global-bundle-file-system-tag">Global Memory</span>
                                            <span class="global-bundle-file-label">{section.name}</span>
                                            <div class="global-bundle-file-actions">
                                                <button
                                                    class="global-bundle-btn"
                                                    onClick={() => model.startEditSystem(section)}
                                                >
                                                    Edit
                                                </button>
                                                <button
                                                    class="global-bundle-btn global-bundle-btn-danger"
                                                    title="Delete this system entry"
                                                    onClick={() => void model.removeSystem(section.id)}
                                                >
                                                    Remove
                                                </button>
                                            </div>
                                        </div>
                                        <div class="global-bundle-file-content">
                                            <Markdown
                                                text={section.instructions || "(empty)"}
                                                scrollable={true}
                                                nativeScrollbar={true}
                                                contentClass="global-bundle-file-markdown-content"
                                            />
                                        </div>
                                    </>
                                }
                            >
                                <SystemSectionEditor model={model} isNew={false} />
                            </Show>
                        </div>
                    )}
                </For>

                <Show when={model.editingSystemIdAtom() === NEW_SECTION_ID}>
                    <div class="global-bundle-file global-bundle-file-system is-editing">
                        <SystemSectionEditor model={model} isNew={true} />
                    </div>
                </Show>

                <Show when={model.systemSectionsAtom().length === 0 && model.editingSystemIdAtom() === null}>
                    <button class="global-bundle-add-row" onClick={() => model.startNewSystem()}>
                        + Add Global Memory system entry
                    </button>
                </Show>

                <For each={model.ordinarySectionsAtom()}>
                    {(section, i) => (
                        <div
                            class="global-bundle-file"
                            classList={{ "is-editing": model.editingIdAtom() === section.id }}
                        >
                            <Show
                                when={model.editingIdAtom() === section.id}
                                fallback={
                                    <>
                                        <div class="global-bundle-file-header">
                                            <span class="global-bundle-file-label">{section.name}</span>
                                            <div class="global-bundle-file-actions">
                                                <button
                                                    class="global-bundle-icon-btn"
                                                    title="Move up"
                                                    disabled={i() === 0}
                                                    onClick={() => void model.move(section.id, -1)}
                                                >
                                                    ↑
                                                </button>
                                                <button
                                                    class="global-bundle-icon-btn"
                                                    title="Move down"
                                                    disabled={i() === model.ordinarySectionsAtom().length - 1}
                                                    onClick={() => void model.move(section.id, 1)}
                                                >
                                                    ↓
                                                </button>
                                                <button
                                                    class="global-bundle-btn"
                                                    onClick={() => model.startEdit(section)}
                                                >
                                                    Edit
                                                </button>
                                                <button
                                                    class="global-bundle-btn global-bundle-btn-danger"
                                                    title="Remove from the global bundles (keeps the bundle)"
                                                    onClick={() => void model.remove(section.id)}
                                                >
                                                    Remove
                                                </button>
                                            </div>
                                        </div>
                                        <div class="global-bundle-file-content">
                                            <Markdown
                                                text={section.instructions || "(empty)"}
                                                scrollable={true}
                                                nativeScrollbar={true}
                                                contentClass="global-bundle-file-markdown-content"
                                            />
                                        </div>
                                    </>
                                }
                            >
                                <SectionEditor model={model} isNew={false} />
                            </Show>
                        </div>
                    )}
                </For>

                {/* New-section draft renders at the END — saveEdit appends it
                    to the order, so its draft position matches where it lands. */}
                <Show when={model.editingIdAtom() === NEW_SECTION_ID}>
                    <div class="global-bundle-file is-editing">
                        <SectionEditor model={model} isNew={true} />
                    </div>
                </Show>

                <Show when={model.ordinarySectionsAtom().length === 0 && model.editingIdAtom() === null}>
                    <div class="global-bundle-empty">No global sections yet.</div>
                </Show>
            </div>

            <div class="global-bundle-add-bar">
                <button
                    class="global-bundle-add-row"
                    disabled={model.editingIdAtom() === NEW_SECTION_ID}
                    onClick={() => model.startNew()}
                >
                    + New section
                </button>
                <Show when={model.candidatesAtom().length > 0}>
                    <select
                        class="global-bundle-promote-select"
                        value={promoteValue()}
                        onChange={(e) => handlePromote(e.currentTarget.value)}
                    >
                        <option value="">Promote existing bundle…</option>
                        <For each={model.candidatesAtom()}>
                            {(c) => <option value={c.id}>{c.name}</option>}
                        </For>
                    </select>
                </Show>
            </div>

            <div class="global-bundle-preview">
                <button
                    class="global-bundle-preview-toggle"
                    onClick={() => model.setShowPreview(!model.showPreviewAtom())}
                >
                    {model.showPreviewAtom() ? "▾" : "▸"} Combined preview
                </button>
                <Show when={model.showPreviewAtom()}>
                    {/* See the matching comment on the file-list preview
                        blocks above — same reason this is a wrapper div, not
                        a class applied directly to <Markdown>. */}
                    <div class="global-bundle-preview-content">
                        <Markdown
                            text={model.previewAtom() || "(empty)"}
                            scrollable={true}
                            nativeScrollbar={true}
                            contentClass="global-bundle-preview-markdown-content"
                        />
                    </div>
                </Show>
            </div>
        </div>
    );
};

GlobalBundleManager.displayName = "GlobalBundleManager";
