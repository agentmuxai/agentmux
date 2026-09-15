// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// GlobalBundleManager — the Global section of the Armory "Memory" tab (moves to
// the Bundles tab in Phase 4 of SPEC_ARMORY_NAMING_CONSOLIDATION_2026_09_09.md).
// Presents the workspace-wide global bundles (is_global rows) as an ordered
// list of editable memories that compose into every agent's startup
// instructions file (CLAUDE.md, GEMINI.md, or similar, depending on
// provider) at launch.
//
// Context-free: owns its own GlobalBundleViewModel and drives off the
// bundle_* RPCs. Spec: docs/specs/archive/SPEC_TRUST_CENTER_GLOBAL_BRAIN_2026_06_19.md.
//
// Layout restructured twice: first per docs/specs/SPEC_ARMORY_GLOBAL_MEMORY_
// DECLUTTER_2026_09_15.md (nine divergent blocks collapsed into one
// consistently-shaped file-list, "applies to" chips removed), then per
// docs/specs/SPEC_GLOBAL_MEMORY_UNIFY_SYSTEM_AND_ORDINARY_2026_09_15.md —
// the "system tier" (AgentMux-controlled, pinned-first, override-wording
// entries) and ordinary sections used to render as two structurally
// separate lists with two editor components mirroring the backend's own
// write-path isolation. That backend isolation (SPEC_GLOBAL_MEMORY_SYSTEM_
// TIER_2026_08_24.md — is_system, the two upsert/delete RPCs, never wired
// to any MCP tool) is real and unchanged; only the FRONTEND presentation
// unifies here into one list, one MemoryEditor, one "Memory" vocabulary —
// see that unify spec's postmortem for why mirroring the backend split into
// the UI was the actual mistake, not the backend split itself. Creating a
// NEW system-tier entry has no UI trigger anymore; existing ones remain
// fully editable/removable via their own still-isolated RPCs.

import { For, onCleanup, Show, type JSX } from "solid-js";
import { Markdown } from "@/app/element/markdown";
import { showTextInputContextMenu } from "@/app/store/contextmenu";
import { GlobalBundleViewModel, NEW_SECTION_ID } from "./global-bundle-model";
import "./global-bundle.scss";

/** Inline editor card for both ordinary and (existing, `isSystem`) system
 *  memories — one component, not two, per docs/specs/SPEC_GLOBAL_MEMORY_
 *  UNIFY_SYSTEM_AND_ORDINARY_2026_09_15.md. Reads/writes whichever of the
 *  model's two draft-state signal pairs `isSystem` selects, and calls the
 *  correspondingly correct (still backend-isolated) save method — the only
 *  place that distinction survives is which RPC ends up called, never in
 *  what's rendered. */
function MemoryEditor(props: { model: GlobalBundleViewModel; isNew: boolean; isSystem: boolean }): JSX.Element {
    const { model, isSystem } = props;
    const nameValue = () => (isSystem ? model.draftSystemNameAtom() : model.draftNameAtom());
    const setNameValue = (v: string) => (isSystem ? model.setDraftSystemName(v) : model.setDraftName(v));
    const instructionsValue = () => (isSystem ? model.draftSystemInstructionsAtom() : model.draftInstructionsAtom());
    const setInstructionsValue = (v: string) =>
        isSystem ? model.setDraftSystemInstructions(v) : model.setDraftInstructions(v);
    const cancel = () => (isSystem ? model.cancelEditSystem() : model.cancelEdit());
    const save = () => void (isSystem ? model.saveSystemEdit() : model.saveEdit());
    const canSave = () => !model.savingAtom() && nameValue().trim().length > 0;

    return (
        <div class="global-bundle-editor">
            <label class="global-bundle-field">
                <span class="global-bundle-field-label">Name</span>
                <input
                    class="global-bundle-input"
                    type="text"
                    value={nameValue()}
                    onInput={(e) => setNameValue(e.currentTarget.value)}
                    onContextMenu={showTextInputContextMenu}
                    placeholder="e.g. Coding Standards"
                />
            </label>
            <label class="global-bundle-field">
                <span class="global-bundle-field-label">Content</span>
                <textarea
                    class="global-bundle-textarea"
                    rows={8}
                    value={instructionsValue()}
                    onInput={(e) => setInstructionsValue(e.currentTarget.value)}
                    onContextMenu={showTextInputContextMenu}
                    placeholder={
                        isSystem
                            ? "Markdown injected FIRST into every agent's startup instructions file, wrapped in explicit override wording."
                            : "Markdown injected into every agent's startup instructions file under a # [Workspace] heading."
                    }
                    spellcheck={false}
                />
            </label>
            <div class="global-bundle-editor-actions">
                <button class="global-bundle-btn" disabled={model.savingAtom()} onClick={cancel}>
                    Cancel
                </button>
                <button class="global-bundle-btn global-bundle-btn-primary" disabled={!canSave()} onClick={save}>
                    {model.savingAtom() ? "Saving…" : props.isNew ? "Add Memory" : "Save"}
                </button>
            </div>
        </div>
    );
}

export const GlobalBundleManager = (): JSX.Element => {
    const model = new GlobalBundleViewModel();
    onCleanup(() => model.dispose());

    // System rows first, then ordinary — NOT model.sectionsAtom() directly,
    // which sorts by raw sort_order/name and isn't documented as
    // system-first (only the backend's ORDER BY and the composed-file
    // formatter guarantee that independently of this atom's own order —
    // SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md §3.2/§3.4). Building it
    // explicitly from the two split atoms keeps on-screen order visibly
    // matching actual injection order.
    const memoryEntries = () => [...model.systemSectionsAtom(), ...model.ordinarySectionsAtom()];

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

                {/* One list for every memory (system-tier + ordinary) — see
                    this component's own top-of-file comment for why these
                    used to be two lists and why that was the wrong place to
                    carry the backend's write-path isolation. */}
                <For each={memoryEntries()}>
                    {(section) => {
                        const isSystem = !!section.is_system;
                        const isEditing = () =>
                            isSystem ? model.editingSystemIdAtom() === section.id : model.editingIdAtom() === section.id;
                        // Only meaningful for ordinary rows — system rows never
                        // reorder (backend silently no-ops it, see move()'s own
                        // doc comment in the model), so ↑/↓ isn't rendered for
                        // them at all rather than rendered-but-inert.
                        const ordIndex = () => model.ordinarySectionsAtom().findIndex((s) => s.id === section.id);
                        return (
                            <div class="global-bundle-file" classList={{ "is-editing": isEditing() }}>
                                <Show
                                    when={isEditing()}
                                    fallback={
                                        <>
                                            <div class="global-bundle-file-header">
                                                <span class="global-bundle-file-label">{section.name}</span>
                                                <div class="global-bundle-file-actions">
                                                    <Show when={!isSystem}>
                                                        <button
                                                            class="global-bundle-icon-btn"
                                                            title="Move up"
                                                            disabled={ordIndex() === 0}
                                                            onClick={() => void model.move(section.id, -1)}
                                                        >
                                                            ↑
                                                        </button>
                                                        <button
                                                            class="global-bundle-icon-btn"
                                                            title="Move down"
                                                            disabled={ordIndex() === model.ordinarySectionsAtom().length - 1}
                                                            onClick={() => void model.move(section.id, 1)}
                                                        >
                                                            ↓
                                                        </button>
                                                    </Show>
                                                    <button
                                                        class="global-bundle-btn"
                                                        onClick={() =>
                                                            isSystem ? model.startEditSystem(section) : model.startEdit(section)
                                                        }
                                                    >
                                                        Edit
                                                    </button>
                                                    <button
                                                        class="global-bundle-btn global-bundle-btn-danger"
                                                        title="Delete this Memory"
                                                        onClick={() =>
                                                            void (isSystem
                                                                ? model.removeSystem(section.id)
                                                                : model.remove(section.id))
                                                        }
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
                                    <MemoryEditor model={model} isNew={false} isSystem={isSystem} />
                                </Show>
                            </div>
                        );
                    }}
                </For>

                {/* New-memory draft renders at the END — saveEdit appends it
                    to the order, so its draft position matches where it
                    lands. Always ordinary: creating a NEW system-tier entry
                    has no UI trigger anymore (see top-of-file comment) —
                    existing ones stay fully editable/removable above. */}
                <Show when={model.editingIdAtom() === NEW_SECTION_ID}>
                    <div class="global-bundle-file is-editing">
                        <MemoryEditor model={model} isNew={true} isSystem={false} />
                    </div>
                </Show>

                <Show when={memoryEntries().length === 0 && model.editingIdAtom() === null}>
                    <div class="global-bundle-empty">No memories yet.</div>
                </Show>
            </div>

            <button
                class="global-bundle-add-row"
                disabled={model.editingIdAtom() === NEW_SECTION_ID}
                onClick={() => model.startNew()}
            >
                + Add Memory
            </button>

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
