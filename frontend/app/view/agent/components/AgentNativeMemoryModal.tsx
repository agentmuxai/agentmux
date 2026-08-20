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
 */

import { createEffect, createSignal, For, onCleanup, Show, type JSX } from "solid-js";
import { PrimitiveListDetail } from "@/app/element/primitive-list-detail";
import { AgentNativeMemoryModel, normalizeMemoryFilename, validateMemoryFilename } from "../agent-native-memory-model";
import { NativeMemoryHistoryPanel } from "./NativeMemoryHistoryPanel";
import "./AgentNativeMemoryModal.scss";

interface AgentNativeMemoryModalProps {
    agentId: string;
    agentName: string;
    workingDirectory: string;
    onClose: () => void;
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
    const detailView = (
        <div class="agent-memory-modal-detail">
            <Show
                when={showHistory()}
                fallback={
                    <Show
                        when={model.editingAtom()}
                        fallback={
                            <div class="agent-memory-modal-view">
                                <pre class="agent-memory-modal-content">
                                    {model.contentAtom() ?? "Loading…"}
                                </pre>
                                <div class="agent-memory-modal-detail-actions">
                                    <button
                                        class="agent-memory-modal-btn"
                                        disabled={model.contentAtom() === null}
                                        onClick={() => setShowHistory(true)}
                                    >
                                        History
                                    </button>
                                    <button
                                        class="agent-memory-modal-btn"
                                        disabled={model.contentAtom() === null}
                                        onClick={() => model.startEdit()}
                                    >
                                        Edit
                                    </button>
                                </div>
                            </div>
                        }
                    >
                        <div class="agent-memory-modal-edit">
                            <textarea
                                class="agent-memory-modal-textarea"
                                value={model.draftContentAtom()}
                                onInput={(e) => model.setDraftContent(e.currentTarget.value)}
                                spellcheck={false}
                            />
                            <div class="agent-memory-modal-detail-actions">
                                <button
                                    class="agent-memory-modal-btn"
                                    disabled={model.savingAtom()}
                                    onClick={() => model.cancelEdit()}
                                >
                                    Cancel
                                </button>
                                <button
                                    class="agent-memory-modal-btn agent-memory-modal-btn-primary"
                                    disabled={model.savingAtom()}
                                    onClick={() => void model.saveEdit()}
                                >
                                    {model.savingAtom() ? "Saving…" : "Save"}
                                </button>
                            </div>
                        </div>
                    </Show>
                }
            >
                <div class="agent-memory-modal-view">
                    <Show when={model.selectedFilenameAtom()} keyed>
                        {(filename) => (
                            <NativeMemoryHistoryPanel
                                agentId={props.agentId}
                                filename={filename}
                                onContentReverted={(content) => {
                                    model.setContent(content);
                                    void model.loadFiles();
                                }}
                            />
                        )}
                    </Show>
                    <div class="agent-memory-modal-detail-actions">
                        <button class="agent-memory-modal-btn" onClick={() => setShowHistory(false)}>
                            Back to content
                        </button>
                    </div>
                </div>
            </Show>
        </div>
    );

    return (
        <div class="agent-memory-modal">
            <div class="agent-memory-modal-header">
                <div class="agent-memory-modal-title">Memory — {props.agentName}</div>
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
                backLabel="Memories"
                onBack={() => model.clearSelection()}
                list={listView}
                detail={detailView}
            />

            <div class="agent-memory-modal-footer">
                <button class="agent-memory-modal-btn" data-modal-dismiss onClick={props.onClose}>
                    Close
                </button>
            </div>
        </div>
    );
};

AgentNativeMemoryModal.displayName = "AgentNativeMemoryModal";
