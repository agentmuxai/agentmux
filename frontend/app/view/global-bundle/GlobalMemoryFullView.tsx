// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * GlobalMemoryFullView — what a Global Memory tile expands to
 * (docs/specs/SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.3/§2.4): one
 * entry (view / edit / history), a new-entry draft, the read-only CLAUDE.md,
 * or the combined preview. Every variant uses the pinned layout — actions,
 * name field, banner, history and hints in the top region; the content or
 * editor filling the bottom. The back/breadcrumb header is the caller's
 * (global-bundle-manager.tsx), same pattern as Personal Memory.
 */

import { createEffect, createMemo, on, onCleanup, Show, untrack, type JSX } from "solid-js";
import { Markdown } from "@/app/element/markdown";
import { showTextInputContextMenu } from "@/app/store/contextmenu";
import type { Bundle, GlobalMemoryVersionMeta } from "@/app/store/rpc-api";
import { handleMemoryEditorKeyDown, requestCancel } from "@/app/view/memory-editor/editor-keys";
import { MemoryConflictBanner } from "@/app/view/memory-editor/MemoryConflictBanner";
import { MemoryContent } from "@/app/view/memory-editor/MemoryContent";
import { MemoryHistory } from "@/app/view/memory-editor/MemoryHistory";
import { MemoryHistoryModel } from "@/app/view/memory-editor/memory-history-model";
import { PinnedEditorLayout } from "@/app/view/memory-editor/PinnedEditorLayout";
import { globalMemoryHistorySource, type GlobalBundleViewModel, type GlobalMemoryDraft } from "./global-bundle-model";

const SURFACE = "armory-global";

const draftText = (v: GlobalMemoryDraft) => `# ${v.name}\n\n${v.instructions}`;

/** Name field + editor bar shared by an existing entry and a new one. */
function EditorBar(props: { model: GlobalBundleViewModel; isNew: boolean }): JSX.Element {
    const draft = props.model.draft;
    const value = () => draft.draftAtom() ?? { name: "", instructions: "" };
    return (
        <>
            <div class="memory-editor-actions">
                <button
                    type="button"
                    class="memory-editor-btn is-primary"
                    disabled={draft.savingAtom() || !value().name.trim()}
                    onClick={() => void props.model.save()}
                    title="Save (Ctrl/Cmd+S)"
                >
                    {draft.savingAtom() ? "Saving…" : props.isNew ? "Add memory" : "Save"}
                </button>
                <button
                    type="button"
                    class="memory-editor-btn"
                    disabled={draft.savingAtom()}
                    onClick={() =>
                        requestCancel({
                            isDirty: draft.dirtyAtom,
                            onCancel: () => (props.isNew ? props.model.close() : draft.cancel()),
                        })
                    }
                    title="Cancel (Esc)"
                >
                    Cancel
                </button>
                <span class="memory-editor-status">{draft.dirtyAtom() ? "Unsaved changes" : "No changes"}</span>
            </div>
            <label class="memory-editor-field">
                <span class="memory-editor-field-label">Name</span>
                <input
                    class="memory-editor-input"
                    type="text"
                    value={value().name}
                    onInput={(e) => draft.setDraft({ ...value(), name: e.currentTarget.value })}
                    onContextMenu={showTextInputContextMenu}
                    placeholder="e.g. Coding Standards"
                />
            </label>
        </>
    );
}

/** Mounted once per entry id (not per list refresh — the refreshed Bundle is
 *  a new object every time, and a remount would drop the draft and the open
 *  history selection). If the entry is deleted while open, the last-known
 *  copy stays on screen: a dirty draft gets the "deleted" banner instead of
 *  vanishing with the view. */
function EntryView(props: { model: GlobalBundleViewModel; id: string }): JSX.Element {
    const model = props.model;
    const draft = model.draft;
    const id = props.id;
    const history = new MemoryHistoryModel<GlobalMemoryVersionMeta>(globalMemoryHistorySource(id));
    onCleanup(() => history.dispose());
    // Reload history in place whenever the list refreshes (a save, a revert,
    // an agent's GlobalMemoryWrite arriving via memories:changed).
    createEffect(on(model.refreshNonceAtom, () => void history.loadHistory(), { defer: true }));

    let last: Bundle | null = untrack(model.openEntryAtom);
    const liveEntry = createMemo(() => {
        const e = model.openEntryAtom();
        if (e) last = e;
        return e;
    });
    const entry = (): Bundle => liveEntry() ?? (last as Bundle);
    const isSystem = () => !!entry().is_system;
    const ordIndex = () => model.ordinarySectionsAtom().findIndex((s) => s.id === id);

    const top = (
        <>
            <Show
                when={draft.editingAtom()}
                fallback={
                    <div class="memory-editor-actions">
                        <button type="button" class="memory-editor-btn" onClick={() => model.startEdit()}>
                            Edit
                        </button>
                        {/* ↑/↓ stay here as the keyboard-reachable fallback for the
                            grid's drag-to-reorder. System entries never reorder. */}
                        <Show when={!isSystem()}>
                            <button
                                type="button"
                                class="memory-editor-btn"
                                title="Move earlier in the injection order"
                                disabled={ordIndex() <= 0}
                                onClick={() => void model.move(id, -1)}
                            >
                                ↑
                            </button>
                            <button
                                type="button"
                                class="memory-editor-btn"
                                title="Move later in the injection order"
                                disabled={ordIndex() === -1 || ordIndex() === model.ordinarySectionsAtom().length - 1}
                                onClick={() => void model.move(id, 1)}
                            >
                                ↓
                            </button>
                        </Show>
                        <span class="memory-editor-actions-spacer" />
                        <button
                            type="button"
                            class="memory-editor-btn is-danger"
                            title="Remove this memory from Global Memory"
                            onClick={() => {
                                if (!window.confirm(`Remove "${entry().name}" from Global Memory?`)) return;
                                void (isSystem() ? model.removeSystem(id) : model.remove(id));
                            }}
                        >
                            Remove
                        </button>
                    </div>
                }
            >
                <EditorBar model={model} isNew={false} />
            </Show>
            <Show when={draft.errorAtom()}>
                <div class="memory-editor-error">{draft.errorAtom()}</div>
            </Show>
            <MemoryConflictBanner model={draft} toText={draftText} noun="this memory" />
            <Show when={!liveEntry() && !draft.editingAtom()}>
                <div class="memory-editor-error">This memory no longer exists.</div>
            </Show>
            <Show when={isSystem()}>
                <p class="global-bundle-intro">
                    AgentMux-controlled: injected first, with override wording, into every agent's startup file.
                </p>
            </Show>
            <MemoryHistory model={history} revertDisabled={draft.dirtyAtom()} />
        </>
    );

    const bottom = (
        <MemoryContent
            content={entry().instructions ?? ""}
            editing={draft.editingAtom()}
            draft={draft.draftAtom()?.instructions ?? ""}
            onDraftInput={(v) => draft.setDraft({ ...(draft.draftAtom() ?? { name: "", instructions: "" }), instructions: v })}
            emptyText="(empty)"
            textareaLabel={`Edit ${entry().name}`}
            placeholder={
                isSystem()
                    ? "Markdown injected FIRST into every agent's startup instructions file, wrapped in explicit override wording."
                    : "Markdown injected into every agent's startup instructions file under a # [Workspace] heading."
            }
        />
    );

    return <PinnedEditorLayout surface={SURFACE} top={top} bottom={bottom} onKeyDown={(e) => keys(e, model)} />;
}

function keys(e: KeyboardEvent, model: GlobalBundleViewModel) {
    const draft = model.draft;
    handleMemoryEditorKeyDown(e, {
        isEditing: draft.editingAtom,
        isDirty: draft.dirtyAtom,
        onSave: () => void model.save(),
        onCancel: () => (model.viewAtom()?.kind === "new" ? model.close() : draft.cancel()),
    });
}

function NewEntryView(props: { model: GlobalBundleViewModel }): JSX.Element {
    const draft = props.model.draft;
    const top = (
        <>
            <EditorBar model={props.model} isNew={true} />
            <Show when={draft.errorAtom()}>
                <div class="memory-editor-error">{draft.errorAtom()}</div>
            </Show>
            <p class="global-bundle-intro">Added to the end of the injection order.</p>
        </>
    );
    const bottom = (
        <MemoryContent
            content={null}
            editing={true}
            draft={draft.draftAtom()?.instructions ?? ""}
            onDraftInput={(v) => draft.setDraft({ ...(draft.draftAtom() ?? { name: "", instructions: "" }), instructions: v })}
            textareaLabel="New memory content"
            placeholder="Markdown injected into every agent's startup instructions file under a # [Workspace] heading."
        />
    );
    return <PinnedEditorLayout surface={SURFACE} top={top} bottom={bottom} onKeyDown={(e) => keys(e, props.model)} />;
}

/** Read-only reference display of the CLAUDE.md in the isolated config dir a
 *  default spawned agent launches with (CLAUDE_CONFIG_DIR). NOT part of
 *  AgentMux's own Global Memory composition — see
 *  docs/specs/SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md §7. */
function ClaudeConfigView(props: { model: GlobalBundleViewModel }): JSX.Element {
    const cfg = () => props.model.claudeGlobalConfigAtom();
    const top = (
        <>
            <code
                class="global-bundle-file-label"
                title="Claude Code — shared provider config. Used by default spawned agents; identity-bound agents use a separate dir, not shown here."
            >
                {cfg()?.path}
            </code>
            <p class="global-bundle-intro">
                Read-only. Claude Code's own config file for default spawned agents — not composed from Global
                Memory.
            </p>
        </>
    );
    const bottom = (
        <Show
            when={cfg()?.exists}
            fallback={<p class="memory-content-status global-bundle-file-empty">No file at this path yet.</p>}
        >
            <MemoryContent content={cfg()?.content ?? ""} editing={false} />
        </Show>
    );
    return <PinnedEditorLayout surface={SURFACE} top={top} bottom={bottom} />;
}

/** The exact block every agent's startup instructions file gets. */
function PreviewView(props: { model: GlobalBundleViewModel }): JSX.Element {
    const top = (
        <p class="global-bundle-intro">
            Exactly what every agent's startup instructions file (CLAUDE.md, GEMINI.md, …) receives at launch, in
            injection order.
        </p>
    );
    const bottom = (
        <div class="memory-content">
            <div class="memory-content-markdown">
                <Markdown
                    text={props.model.previewAtom() || "(empty)"}
                    scrollable={true}
                    nativeScrollbar={true}
                    contentClass="memory-content-markdown-content"
                />
            </div>
        </div>
    );
    return <PinnedEditorLayout surface={SURFACE} top={top} bottom={bottom} />;
}

export function GlobalMemoryFullView(props: { model: GlobalBundleViewModel }): JSX.Element {
    // Keyed on kind AND entry id, so opening a different entry remounts —
    // but a list refresh (same view) never does.
    const viewKey = () => {
        const v = props.model.viewAtom();
        if (!v) return null;
        return v.kind === "entry" ? `entry:${v.id}` : v.kind;
    };
    return (
        <Show when={viewKey()} keyed>
            {(key) => {
                switch (key) {
                    case "new":
                        return <NewEntryView model={props.model} />;
                    case "claude-config":
                        return <ClaudeConfigView model={props.model} />;
                    case "preview":
                        return <PreviewView model={props.model} />;
                    default:
                        return (
                            <Show
                                when={untrack(props.model.openEntryAtom)}
                                fallback={<p class="memory-content-status">This memory no longer exists.</p>}
                            >
                                <EntryView model={props.model} id={key.slice("entry:".length)} />
                            </Show>
                        );
                }
            }}
        </Show>
    );
}
