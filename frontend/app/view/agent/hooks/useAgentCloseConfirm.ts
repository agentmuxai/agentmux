// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useAgentCloseConfirm — pane-close confirmation when tracked processes
 * are still alive.
 *
 * Extracted verbatim from agent-view.tsx. Wraps `nodeModel.onClose` in
 * place: when the user closes a pane with tracked processes still running,
 * the wrapper raises a ConfirmModal instead of closing immediately. Accept
 * → `agent.kill-tree` RPC then proceed with close. Cancel → abort, pane
 * stays open. Zero tracked processes → original close path, no prompt.
 *
 * We wrap `nodeModel.onClose` in place rather than adding a new ViewModel
 * hook — ViewModel has no `beforeClose` / `canClose` surface today.
 */

import { createSignal, onCleanup, onMount } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { AgentViewModel } from "../agent-model";

export interface UseAgentCloseConfirmOptions {
    blockId: string;
    model: AgentViewModel;
    /** Live count of tracked OS processes for this block. */
    processCount: () => number;
}

export interface CloseConfirmInfo {
    count: number;
    originalClose: () => void;
}

export interface UseAgentCloseConfirmResult {
    closeConfirm: () => CloseConfirmInfo | null;
    setCloseConfirm: (info: CloseConfirmInfo | null) => void;
    handleCloseConfirmAccept: () => Promise<void>;
}

export function useAgentCloseConfirm(opts: UseAgentCloseConfirmOptions): UseAgentCloseConfirmResult {
    const [closeConfirm, setCloseConfirm] = createSignal<CloseConfirmInfo | null>(null);

    onMount(() => {
        const original = opts.model.nodeModel.onClose;
        const wrapped = () => {
            const count = opts.processCount();
            if (count <= 0) {
                original?.();
                return;
            }
            // Stash the original close so the modal can invoke it on
            // confirm. Not calling original() here keeps the pane open
            // until the user decides.
            setCloseConfirm({ count, originalClose: () => original?.() });
        };
        opts.model.nodeModel.onClose = wrapped;
        onCleanup(() => {
            // Only restore if we're still the wrapper — avoids
            // clobbering a later wrapper set by someone else.
            if (opts.model.nodeModel.onClose === wrapped) {
                opts.model.nodeModel.onClose = original;
            }
        });
    });

    const handleCloseConfirmAccept = async () => {
        const info = closeConfirm();
        if (!info) return;
        try {
            // Kill first, then proceed with layout close. The tracker's
            // Drop impl in `delete_controller` will nuke what survived
            // if the RPC errors — we've already committed to closing.
            await RpcApi.AgentKillTreeCommand(TabRpcClient, {
                block_id: opts.blockId,
            });
        } catch {
            // swallow — close proceeds regardless
        } finally {
            setCloseConfirm(null);
            info.originalClose();
        }
    };

    return { closeConfirm, setCloseConfirm, handleCloseConfirmAccept };
}
