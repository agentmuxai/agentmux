// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentNativeMemoryModal — the agent pane's brain modal. A two-column
 * browser/editor over the agent's native memory folder
 * (`~/.claude/projects/<sanitized>/memory/`): file list on the left,
 * read/edit view on the right.
 *
 * Replaces the Phase 1 placeholder (AgentMemoryModalPanel). Backend RPCs
 * live in native_memory_handlers.rs.
 *
 * Spec: SPEC_AGENT_PANE_MEMORY_IDENTITY_MODALS_2026_06_19.md §5.
 */

import { createSignal, For, onCleanup, Show, type JSX } from "solid-js";
import { AgentNativeMemoryModel } from "../agent-native-memory-model";
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

    const handleNewFile = () => {
        const raw = window.prompt("New memory file name (e.g. project-notes):");
        if (raw == null) return; // cancelled
        void model.createFile(raw, "");
    };

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

            <div class="agent-memory-modal-body">
                <div class="agent-memory-modal-list">
                    <Show
                        when={model.filesAtom().length > 0}
                        fallback={
                            <Show
                                when={!model.loadingAtom()}
                                fallback={<div class="agent-memory-modal-list-empty">Loading…</div>}
                            >
                                <div class="agent-memory-modal-list-empty">No files</div>
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
                    <button class="agent-memory-modal-new-btn" onClick={handleNewFile}>
                        + New file
                    </button>
                </div>

                <div class="agent-memory-modal-detail">
                    <Show
                        when={model.selectedFilenameAtom()}
                        fallback={<EmptyState model={model} hasFiles={model.filesAtom().length > 0} />}
                    >
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
                    </Show>
                </div>
            </div>

            <div class="agent-memory-modal-footer">
                <button class="agent-memory-modal-btn" data-modal-dismiss onClick={props.onClose}>
                    Close
                </button>
            </div>
        </div>
    );
};

AgentNativeMemoryModal.displayName = "AgentNativeMemoryModal";

/** Right-pane empty state: differs depending on whether the folder has any
 *  files at all. */
function EmptyState(props: { model: AgentNativeMemoryModel; hasFiles: boolean }): JSX.Element {
    const [creating, setCreating] = createSignal(false);
    return (
        <Show
            when={!props.hasFiles}
            fallback={
                <div class="agent-memory-modal-empty">
                    Select a file from the list to view it.
                </div>
            }
        >
            <div class="agent-memory-modal-empty">
                <p class="agent-memory-modal-empty-heading">No memory files yet.</p>
                <p class="agent-memory-modal-empty-desc">
                    Claude Code creates this folder when it first saves a memory for this
                    agent. You can also create files manually — they'll be available at the
                    next session start.
                </p>
                <button
                    class="agent-memory-modal-btn agent-memory-modal-btn-primary"
                    disabled={creating()}
                    onClick={() => {
                        setCreating(true);
                        void props.model.createMemoryIndex().finally(() => setCreating(false));
                    }}
                >
                    + Create MEMORY.md
                </button>
            </div>
        </Show>
    );
}
