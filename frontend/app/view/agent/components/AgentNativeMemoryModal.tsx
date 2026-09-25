// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentNativeMemoryModal — the agent pane's brain modal. Single-pane
 * browser/editor over the agent's native memory folder
 * (`~/.claude/projects/<sanitized>/memory/`) — list, or one file's
 * read/edit view, never both at once (PrimitiveListDetail; matches the MCP
 * Servers / Skills tabs' convention). Was a fixed two-column split with a
 * 220px list rail; migrated to eliminate horizontal scroll on narrow panes
 * — see SPEC_AGENT_PANE_ARMORY_HEADER_ICON_2026_07_20.md §7.2.
 *
 * Replaces the Phase 1 placeholder (AgentMemoryModalPanel). Backend RPCs
 * live in native_memory_handlers.rs.
 *
 * Spec: SPEC_AGENT_PANE_MEMORY_IDENTITY_MODALS_2026_06_19.md §5.
 *
 * The file view is a PinnedEditorLayout since 2026-09-24: the Edit/History,
 * Save/Cancel and Close bars moved from below the content to the top, and
 * the content (or editor) fills the bottom of the pane. Ctrl/Cmd+S saves,
 * Esc cancels (asking first when dirty), and a save carries its base hash.
 * docs/specs/SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.4.
 */

import { createEffect, createSignal, For, onCleanup, Show, type JSX } from "solid-js";
import { PrimitiveListDetail } from "@/app/element/primitive-list-detail";
import { handleMemoryEditorKeyDown, requestCancel } from "@/app/view/memory-editor/editor-keys";
import { MemoryConflictBanner } from "@/app/view/memory-editor/MemoryConflictBanner";
import { MemoryContent } from "@/app/view/memory-editor/MemoryContent";
import { MemoryHistory } from "@/app/view/memory-editor/MemoryHistory";
import { PinnedEditorLayout } from "@/app/view/memory-editor/PinnedEditorLayout";
import { AgentNativeMemoryModel, normalizeMemoryFilename, validateMemoryFilename } from "../agent-native-memory-model";
import { NativeMemoryHistoryModel } from "../native-memory-history-model";
import "./AgentNativeMemoryModal.scss";
import type { NativeMemoryFileMeta } from "@/app/store/rpc-api";

interface AgentNativeMemoryModalProps {
    agentId: string;
    agentName: string;
    workingDirectory: string;
    /**
     * Omitted when this renders somewhere with no "close me" affordance of
     * its own to drive — the Stash DRAWER
     * (SPEC_AGENT_STASH_PANE_MIGRATION_2026_09_22.md §3.4), where closing is
     * the header icon's job and a Close button inside one of six tabs would
     * be meaningless. The footer is hidden entirely in that case rather than
     * rendering a dead button.
     */
    onClose?: () => void;
}

/**
 * Display-only approximation of the memory folder path. The real path is
 * resolved backend-side (memory_dir_for_cwd in native_memory_handlers.rs);
 * this mirrors the character-replacement step for the subtitle so the user
 * sees roughly where edits land. Long paths differ from disk — the backend
 * appends a hash suffix past 200 chars, which this does not reproduce.
 */
function previewMemoryPath(workDir: string): string {
    if (!workDir) return "~/.claude/projects/…/memory/";
    const sanitized = workDir.replace(/[^a-zA-Z0-9]/g, "-");
    const display = sanitized.length > 64 ? sanitized.slice(0, 64) + "…" : sanitized;
    return `~/.claude/projects/${display}/memory/`;
}

/** Human label for a file's role, per spec §5.3. */
function fileRoleLabel(file: NativeMemoryFileMeta): string {
    if (file.is_index) return "Index · loaded every session";
    return file.metadata_type ? file.metadata_type : "topic";
}

export const AgentNativeMemoryModal = (props: AgentNativeMemoryModalProps): JSX.Element => {
    const model = new AgentNativeMemoryModel(props.agentId, props.agentName);
    onCleanup(() => model.dispose());

    const [newFileName, setNewFileName] = createSignal("");
    const [showNewInput, setShowNewInput] = createSignal(false);
    const [newFileError, setNewFileError] = createSignal<string | null>(null);

    const openNewInput = () => {
        setNewFileName("");
        setNewFileError(null);
        setShowNewInput(true);
    };

    const cancelNewInput = () => {
        setShowNewInput(false);
        setNewFileName("");
        setNewFileError(null);
    };

    const commitNewFile = () => {
        const normalized = normalizeMemoryFilename(newFileName());
        const err = validateMemoryFilename(normalized);
        if (err) {
            setNewFileError(err);
            return;
        }
        setShowNewInput(false);
        setNewFileName("");
        setNewFileError(null);
        void model.createFile(normalized, "");
    };

    const onNewFileKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Enter") { e.preventDefault(); commitNewFile(); }
        if (e.key === "Escape") { e.preventDefault(); cancelNewInput(); }
    };

    // Single-pane — see docs/specs/SPEC_ARMORY_RESPONSIVE_SINGLE_PANE_LAYOUT_2026_07_15.md,
    // adopted here per SPEC_AGENT_PANE_ARMORY_HEADER_ICON_2026_07_20.md §7.2.
    const inDetail = () => model.selectedFilenameAtom() !== null;
    const [creatingIndex, setCreatingIndex] = createSignal(false);

    // Version history toggle (SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md
    // §4.3) — a third view alongside "read" and "edit" within the same
    // detail pane, not a separate modal/route. Reset whenever the selected
    // file changes so switching files doesn't leave a stale file's history
    // showing under a new filename.
    const [showHistory, setShowHistory] = createSignal(false);
    createEffect(() => {
        model.selectedFilenameAtom();
        setShowHistory(false);
    });

    const listView = (
        <div class="agent-memory-modal-list">
            <Show
                when={model.filesAtom().length > 0}
                fallback={
                    <Show
                        when={!model.loadingAtom()}
                        fallback={<div class="agent-memory-modal-list-empty">Loading…</div>}
                    >
                        <div class="agent-memory-modal-empty">
                            <p class="agent-memory-modal-empty-heading">No memory files yet.</p>
                            <p class="agent-memory-modal-empty-desc">
                                Claude Code creates this folder when it first saves a memory for
                                this agent. You can also create files manually — they'll be
                                available at the next session start.
                            </p>
                            <button
                                class="agent-memory-modal-btn agent-memory-modal-btn-primary"
                                disabled={creatingIndex()}
                                onClick={() => {
                                    setCreatingIndex(true);
                                    void model.createMemoryIndex().finally(() => setCreatingIndex(false));
                                }}
                            >
                                + Create MEMORY.md
                            </button>
                        </div>
                    </Show>
                }
            >
                <For each={model.filesAtom()}>
                    {(file) => (
                        <button
                            class="agent-memory-modal-list-item"
                            classList={{
                                "is-selected": model.selectedFilenameAtom() === file.filename,
                                "is-index": file.is_index,
                            }}
                            onClick={() => void model.selectFile(file.filename)}
                            title={fileRoleLabel(file)}
                        >
                            <span class="agent-memory-modal-list-item-name">
                                {file.filename}
                            </span>
                            <Show when={file.is_index}>
                                <span
                                    class="agent-memory-modal-list-item-badge"
                                    title="Loaded into every new Claude session for this agent. Edits take effect on the next session start."
                                >
                                    index
                                </span>
                            </Show>
                            <Show when={!file.is_index && file.metadata_type}>
                                <span class="agent-memory-modal-list-item-type">
                                    {file.metadata_type}
                                </span>
                            </Show>
                        </button>
                    )}
                </For>
            </Show>

            <Show when={showNewInput()}>
                <div class="agent-memory-modal-new-input-row">
                    <input
                        class="agent-memory-modal-new-input"
                        classList={{ "is-error": newFileError() !== null }}
                        type="text"
                        placeholder="filename.md"
                        value={newFileName()}
                        autofocus
                        onInput={(e) => { setNewFileName(e.currentTarget.value); setNewFileError(null); }}
                        onKeyDown={onNewFileKeyDown}
                    />
                    <Show when={newFileError()}>
                        <div class="agent-memory-modal-new-input-error">{newFileError()}</div>
                    </Show>
                    <div class="agent-memory-modal-new-input-actions">
                        <button class="agent-memory-modal-btn" onClick={cancelNewInput}>Cancel</button>
                        <button class="agent-memory-modal-btn agent-memory-modal-btn-primary" onClick={commitNewFile}>Create</button>
                    </div>
                </div>
            </Show>

            <button
                class="agent-memory-modal-new-btn"
                classList={{ "is-hidden": showNewInput() }}
                onClick={openNewInput}
            >
                + New file
            </button>
        </div>
    );

    // Only rendered when inDetail() is true (a file is selected) —
    // PrimitiveListDetail never shows list and detail at once, so there's
    // no "nothing selected" case to handle here anymore.
    //
    // Pinned layout (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.4): every
    // bar that used to sit below the content — Edit/History, Cancel/Save,
    // Back to content — is in the top region, with the history (when shown)
    // under it; the content or editor fills the bottom.
    const detailView = (
        <div class="agent-memory-modal-detail">
            <PinnedEditorLayout
                surface="stash"
                onKeyDown={(e) =>
                    handleMemoryEditorKeyDown(e, {
                        isEditing: model.editingAtom,
                        isDirty: model.draft.dirtyAtom,
                        onSave: () => void model.saveEdit(),
                        onCancel: () => model.cancelEdit(),
                    })
                }
                top={
                    <>
                        <div class="memory-editor-actions">
                            <Show
                                when={model.editingAtom()}
                                fallback={
                                    <button
                                        class="memory-editor-btn"
                                        disabled={model.contentAtom() === null}
                                        onClick={() => model.startEdit()}
                                    >
                                        Edit
                                    </button>
                                }
                            >
                                <button
                                    class="memory-editor-btn is-primary"
                                    disabled={model.savingAtom()}
                                    onClick={() => void model.saveEdit()}
                                    title="Save (Ctrl/Cmd+S)"
                                >
                                    {model.savingAtom() ? "Saving…" : "Save"}
                                </button>
                                <button
                                    class="memory-editor-btn"
                                    disabled={model.savingAtom()}
                                    onClick={() =>
                                        requestCancel({ isDirty: model.draft.dirtyAtom, onCancel: () => model.cancelEdit() })
                                    }
                                    title="Cancel (Esc)"
                                >
                                    Cancel
                                </button>
                            </Show>
                            <button
                                class="memory-editor-btn"
                                classList={{ "is-active": showHistory() }}
                                disabled={model.contentAtom() === null}
                                aria-pressed={showHistory()}
                                onClick={() => setShowHistory(!showHistory())}
                            >
                                {showHistory() ? "Hide history" : "History"}
                            </button>
                        </div>
                        <Show when={model.draft.errorAtom()}>
                            <div class="memory-editor-error">{model.draft.errorAtom()}</div>
                        </Show>
                        <MemoryConflictBanner
                            model={model.draft}
                            toText={(v) => v}
                            noun="this file"
                            onDiscarded={() => {
                                const filename = model.selectedFilenameAtom();
                                if (filename) void model.selectFile(filename);
                            }}
                        />
                        <Show when={showHistory() && model.selectedFilenameAtom()} keyed>
                            {(filename) => {
                                const history = new NativeMemoryHistoryModel(props.agentId, filename);
                                onCleanup(() => history.dispose());
                                history.onReverted = (content) => {
                                    model.applyExternalContent(content);
                                    void model.loadFiles();
                                };
                                return <MemoryHistory model={history} revertDisabled={model.savingAtom()} />;
                            }}
                        </Show>
                    </>
                }
                bottom={
                    <MemoryContent
                        content={model.contentAtom()}
                        loading={model.contentAtom() === null}
                        editing={model.editingAtom()}
                        draft={model.draftContentAtom()}
                        onDraftInput={(v) => model.setDraftContent(v)}
                        view="plain"
                        textareaLabel={`Edit ${model.selectedFilenameAtom() ?? "memory file"}`}
                    />
                }
            />
        </div>
    );

    return (
        <div class="agent-memory-modal">
            <div class="agent-memory-modal-header">
                {/* Close sits at the top with every other bar (it was a
                    footer below the content — SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md
                    §2.4). Hidden in the Stash drawer, which has no "close
                    me" of its own to drive (see `onClose`'s doc comment). */}
                <div class="agent-memory-modal-title-row">
                    <div class="agent-memory-modal-title">Memory — {props.agentName}</div>
                    <Show when={props.onClose}>
                        <button class="agent-memory-modal-btn" data-modal-dismiss onClick={() => props.onClose?.()}>
                            Close
                        </button>
                    </Show>
                </div>
                <code class="agent-memory-modal-path" title={props.workingDirectory}>
                    {previewMemoryPath(props.workingDirectory)}
                </code>
                <div class="agent-memory-modal-path-note">
                    Mirrored path — edits write directly to disk.
                </div>
            </div>

            <Show when={model.errorAtom()}>
                <div class="agent-memory-modal-error">{model.errorAtom()}</div>
            </Show>

            <PrimitiveListDetail
                showDetail={inDetail()}
                backLabel="Personal Memory"
                onBack={() => model.clearSelection()}
                list={listView}
                detail={detailView}
            />

        </div>
    );
};

AgentNativeMemoryModal.displayName = "AgentNativeMemoryModal";
