// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The state of Memory → Personal's full view of one memory file (NativeMemoryFileView.tsx): the file's history and
 * content, the draft being edited, and what its controls do. Reads and writes go through the `agent:memory:*`
 * commands; a save carries `base_sha256`, and a live change reloads history and content in place while the draft
 * model decides whether that is a silent update, a rebase, or the "changed since you started editing" banner.
 * docs/specs/SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.3/§2.4.
 */

import { createEffect, on, onCleanup, type Accessor } from "solid-js";
import type { PaneConfirm } from "@/app/element/create-confirm";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { NativeMemoryHistoryModel } from "@/app/view/agent/native-memory-history-model";
import { askDiscard, handleMemoryEditorKeyDown } from "@/app/view/memory-editor/editor-keys";
import { MemoryDraftModel } from "@/app/view/memory-editor/memory-draft-model";
import { sha256Hex } from "@/util/sha256";

/** What the view is given. */
export interface NativeMemoryFileViewProps {
    agentId: string;
    filename: string;
    /** Bumped by the manager on every `agent:memory:changed` refresh for
     *  this agent. */
    refreshNonce: Accessor<number>;
    /** Lets the manager guard its Back button / file switch. */
    onDirtyChange?: (dirty: boolean) => void;
    /** Where "Discard your unsaved changes?" is asked: the manager's, drawn in the pane. */
    confirm: PaneConfirm;
}

/** The view's state: the file's history and content, the draft, and the actions that edit them. */
export function createNativeMemoryFile(props: NativeMemoryFileViewProps) {
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
                // A human edit in Memory — without this the backend
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
            onCancel: cancel, confirm: askDiscard(props.confirm),
        });

    return { filename, history, draft, startEdit, save, cancel, onKeyDown };
}

/** What the view's markup reads: its state. */
export type NativeMemoryFile = ReturnType<typeof createNativeMemoryFile>;
