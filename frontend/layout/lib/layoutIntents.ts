// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Layout intents: how a layout edit reaches srv. A tree action is applied to
 * the local tree at once (so the edit shows with no round trip) and sent to
 * srv as an intent, never as the tree it produced. srv's reducer applies the
 * same action to its own tree, and what srv stores is the layout: whenever
 * nothing of this window's is in flight, the local tree is compared with the
 * stored one and replaced by it if they differ.
 * SPEC_STRONG_REDUCER_AUTHORITY_LAYOUT_2026_06_30.md §4.
 *
 * Intents and whole-tree pushes (for the edits that are not intents yet) leave
 * in one queue, in order, one at a time: a push sent between two intents can't
 * overtake the first, and the push carries every edit queued behind it.
 */

import { batch } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { LayoutIntent } from "@/types/rpc/LayoutIntent";
import { balanceNode, findNode } from "./layoutNode";
import { migrateLegacyMinimize, rebuildMinimizedSet } from "./layoutMinimize";
import { copyLayoutNode, sameLayoutTree } from "./layoutIntentActions";
import type { LayoutNode } from "./types";
import type { LayoutModel } from "./layoutModel";

/** One item of a model's outbox: an intent, or a whole-tree push. */
export type LayoutOutboxItem = { kind: "intent"; actionId: string; intent: LayoutIntent } | { kind: "push" };

// ── The outbox ──────────────────────────────────────────────────────────────

/** Queue a whole-tree push behind everything already queued. */
export function queueLayoutPush(model: LayoutModel): void {
    if (!model.layoutOutbox.some((item) => item.kind === "push")) model.layoutOutbox.push({ kind: "push" });
    void flushLayoutOutbox(model);
}

const SEND_ATTEMPTS = 3;

/** Send the outbox in order, one item at a time. */
export async function flushLayoutOutbox(model: LayoutModel): Promise<void> {
    if (model.layoutSending) return;
    model.layoutSending = true;
    try {
        while (model.layoutOutbox.length && !model.disposed) {
            const item = model.layoutOutbox.shift();
            if (item.kind === "push") {
                // The push carries the whole local tree, so every edit queued
                // behind it is in it already.
                model.layoutOutbox.length = 0;
                await model.pushLayoutNow();
            } else {
                await sendIntent(model, item.actionId, item.intent);
            }
        }
    } finally {
        model.layoutSending = false;
    }
    reconcileWithStoredLayout(model);
}

async function sendIntent(model: LayoutModel, actionId: string, intent: LayoutIntent): Promise<void> {
    const tabId = model.tabAtom?.()?.oid;
    if (!tabId) return;
    for (let attempt = 1; attempt <= SEND_ATTEMPTS; attempt++) {
        try {
            // The view as it is when this one leaves: srv keeps the latest.
            const res = await RpcApi.LayoutApplyIntentCommand(TabRpcClient, {
                tab_id: tabId,
                action_id: actionId,
                intent,
                focused_node_id: model.treeState.focusedNodeId ?? "",
                magnified_node_id: model.treeState.magnifiedNodeId ?? "",
            });
            if (!res.applied) console.warn(`[layout] srv refused the ${intent.type} intent; showing srv's layout`, res.error);
            model.layoutVersionFloor = Math.max(model.layoutVersionFloor, res.version ?? 0);
            return;
        } catch (e) {
            // A resend of the same action id is a no-op on srv.
            if (attempt === SEND_ATTEMPTS) {
                console.warn(`[layout] sending the ${intent.type} intent failed; showing srv's layout`, e);
                return;
            }
            await new Promise((r) => setTimeout(r, 150 * attempt));
        }
    }
}

/** Nothing of this model's is on its way to srv, and no drag or resize is
 *  holding a preview over the tree. */
export function isLayoutQuiet(model: LayoutModel): boolean {
    return (
        !model.layoutOutbox.length &&
        !model.layoutSending &&
        !model.persistDebounceTimer &&
        !model.processingBackendActions &&
        !model.getter(model.activeDrag) &&
        !model.pendingTreeAction?.currentValueAtom?.()
    );
}

/**
 * Show what srv stores. When the model is quiet and its row includes every
 * intent it sent (`layoutVersionFloor`), a stored tree that differs from the
 * local one replaces it: srv refused an edit, or computed it differently.
 * Focus stays the frontend's while its pane exists.
 */
export function reconcileWithStoredLayout(model: LayoutModel): void {
    if (model.disposed || !isLayoutQuiet(model)) return;
    const row = model.getter(model.muxObjectAtom);
    if (!row || (row.version ?? 0) < model.layoutVersionFloor) return;
    // Normalized as the frontend shows any tree (a tree srv seeded itself may
    // not be), so only a real difference counts.
    let stored = row.rootnode ? copyLayoutNode(row.rootnode as LayoutNode) : undefined;
    try {
        migrateLegacyMinimize(stored);
        if (stored) stored = balanceNode(stored);
    } catch (e) {
        console.warn("[layout] srv's stored layout doesn't normalize; keeping the local one", e);
        return;
    }
    if (sameLayoutTree(stored, model.treeState.rootNode)) return;
    console.info("[layout] the local layout differs from srv's stored one; showing srv's");
    const local = model.treeState;
    local.rootNode = stored;
    const resolves = (id: string | undefined) => !!id && !!local.rootNode && !!findNode(local.rootNode, id);
    const keepOrStored = (mine: string | undefined, theirs: string | undefined) =>
        resolves(mine) ? mine : resolves(theirs) ? theirs : undefined;
    local.focusedNodeId = keepOrStored(local.focusedNodeId, row.focusednodeid);
    local.magnifiedNodeId = keepOrStored(local.magnifiedNodeId, row.magnifiednodeid);
    model.magnifiedNodeId = local.magnifiedNodeId;
    rebuildMinimizedSet(model);
    batch(() => {
        model.updateTree();
        model.setter(model.localTreeStateAtom, { ...model.treeState });
    });
}
