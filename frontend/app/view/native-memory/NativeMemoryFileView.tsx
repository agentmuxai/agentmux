// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * NativeMemoryFileView — Armory → Memory → Personal's full view of one
 * memory file (the third drill-down screen). Replaces the read-only
 * NativeMemoryHistoryPanel there, per
 * docs/specs/SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.3/§2.4:
 *
 *   - Editing is added, through the existing `agent:memory:write_file` RPC
 *     (reversing SPEC_ARMORY_PERSONAL_MEMORY_CONTENT_VIEW_2026_09_11.md §6,
 *     approved by the repo owner);
 *   - pinned layout: action bar, banner and history in the top region, the
 *     content/editor filling the bottom;
 *   - a save carries `base_sha256`;
 *   - a live change no longer remounts this view (it used to be keyed on a
 *     `refreshNonce` bumped by every `agent:memory:changed`, which would wipe
 *     an open draft). `refreshNonce` is a prop now: each bump reloads history
 *     and content IN PLACE, and the draft model decides whether that's a
 *     silent update, a rebase, or the "changed since you started editing"
 *     banner (only when the file's hash differs from the draft's base).
 *
 * Remounts per (agentId, filename) — the caller keys it, as before.
 */

import { createEffect, on, onCleanup, Show, type Accessor, type JSX } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { NativeMemoryHistoryModel } from "@/app/view/agent/native-memory-history-model";
import { handleMemoryEditorKeyDown, requestCancel } from "@/app/view/memory-editor/editor-keys";
import { MemoryConflictBanner } from "@/app/view/memory-editor/MemoryConflictBanner";
import { MemoryContent } from "@/app/view/memory-editor/MemoryContent";
import { MemoryDraftModel } from "@/app/view/memory-editor/memory-draft-model";
import { MemoryHistory } from "@/app/view/memory-editor/MemoryHistory";
import { PinnedEditorLayout } from "@/app/view/memory-editor/PinnedEditorLayout";
import { sha256Hex } from "@/util/sha256";

interface NativeMemoryFileViewProps {
    agentId: string;
    filename: string;
    /** Bumped by the manager on every `agent:memory:changed` refresh for
     *  this agent. */
    refreshNonce: Accessor<number>;
    /** Lets the manager guard its Back button / file switch. */
    onDirtyChange?: (dirty: boolean) => void;
}

export function NativeMemoryFileView(props: NativeMemoryFileViewProps): JSX.Element {
    const agentId = props.agentId;
    const filename = props.filename;
    const history = new NativeMemoryHistoryModel(agentId, filename);
    onCleanup(() => history.dispose());

    const readCurrent = async (): Promise<string | null> => {
        try {
            const res = await RpcApi.NativeMemoryReadFileCommand(TabRpcClient, { agent_id: agentId, filename });
            return res.content;
        } catch (e) {
            if (/not found/i.test((e as Error)?.message ?? "")) return null;
            throw e;
        }
    };

    const draft = new MemoryDraftModel<string>({
        hash: sha256Hex,
        equals: (a, b) => a === b,
        save: async (content, base) => {
            await RpcApi.NativeMemoryWriteFileCommand(TabRpcClient, {
                agent_id: agentId,
                filename,
                content,
                // A human edit in the Armory — without this the backend
                // defaults to "agent_inferred" (reagent P1 on PR #2678).
                provenance: { source: "human" },
                ...(base !== null ? { base_sha256: base } : {}),
            });
        },
        fetchCurrent: readCurrent,
    });

    createEffect(() => props.onDirtyChange?.(draft.dirtyAtom()));
    onCleanup(() => props.onDirtyChange?.(false));

    // In-place refresh on every change event (never on mount — the models
    // already loaded once in their constructors).
    createEffect(
        on(
            props.refreshNonce,
            () => {
                void history.loadHistory();
                void refreshContent();
            },
            { defer: true }
        )
    );

    const refreshContent = async () => {
        const applied = await history.loadContent();
        if (applied) {
            await draft.observeExternal(history.contentAtom());
        } else if (/not found/i.test(history.contentErrorAtom() ?? "")) {
            await draft.observeExternal(null);
        }
    };

    // A revert is an external change as far as an open draft is concerned.
    history.onReverted = (content) => void draft.observeExternal(content);

    const startEdit = () => {
        const content = history.contentAtom();
        if (content === null) return;
        draft.startEdit(content);
    };

    const save = async () => {
        if (await draft.save()) {
            void history.loadHistory();
            void history.loadContent();
        }
    };

    const cancel = () => draft.cancel();

    const onKeyDown = (e: KeyboardEvent) =>
        handleMemoryEditorKeyDown(e, {
            isEditing: draft.editingAtom,
            isDirty: draft.dirtyAtom,
            onSave: () => void save(),
            onCancel: cancel,
        });

    const top = (
        <>
            <div class="memory-editor-actions">
                <Show
                    when={draft.editingAtom()}
                    fallback={
                        <button
                            type="button"
                            class="memory-editor-btn"
                            disabled={history.contentAtom() === null}
                            onClick={startEdit}
                        >
                            Edit
                        </button>
                    }
                >
                    <button
                        type="button"
                        class="memory-editor-btn is-primary"
                        disabled={draft.savingAtom()}
                        onClick={() => void save()}
                        title="Save (Ctrl/Cmd+S)"
                    >
                        {draft.savingAtom() ? "Saving…" : "Save"}
                    </button>
                    <button
                        type="button"
                        class="memory-editor-btn"
                        disabled={draft.savingAtom()}
                        onClick={() => requestCancel({ isDirty: draft.dirtyAtom, onCancel: cancel })}
                        title="Cancel (Esc)"
                    >
                        Cancel
                    </button>
                    <span class="memory-editor-status">{draft.dirtyAtom() ? "Unsaved changes" : "No changes"}</span>
                </Show>
            </div>
            <Show when={draft.errorAtom()}>
                <div class="memory-editor-error">{draft.errorAtom()}</div>
            </Show>
            <MemoryConflictBanner
                model={draft}
                toText={(v) => v}
                noun="this file"
                onDiscarded={() => void history.loadContent()}
            />
            <MemoryHistory model={history} revertDisabled={draft.savingAtom()} />
        </>
    );

    const bottom = (
        <MemoryContent
            content={history.contentAtom()}
            loading={history.contentLoadingAtom()}
            error={history.contentErrorAtom()}
            editing={draft.editingAtom()}
            draft={draft.draftAtom() ?? ""}
            onDraftInput={(v) => draft.setDraft(v)}
            textareaLabel={`Edit ${filename}`}
        />
    );

    return <PinnedEditorLayout surface="armory-personal" top={top} bottom={bottom} onKeyDown={onKeyDown} />;
}
