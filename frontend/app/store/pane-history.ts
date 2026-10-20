// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// This window's pane selection history, and Back / Forward through it: select
// pane B in tab 1, then pane G in tab 4; Back selects B again (its tab, its
// highlight, its caret), Forward selects G again. In memory, one per window.
// SPEC_MOUSE_SIDE_BUTTONS_PANE_AND_TAB_NAV_2026_10_19.md §4.

import { createEffect, createRoot } from "solid-js";
import { atoms, setActiveTab } from "@/app/store/global";
import { getObjectValue, makeORef } from "@/app/store/mos";
import { focusManager } from "@/app/store/focusManager";
import { whenTabContentSettled } from "@/app/tab/tab-content-settled";
import { getLayoutModelForTabById, setActiveBlockInStack } from "@/layout/index";
import { findNodeByBlockId } from "@/layout/lib/layoutNode";
import {
    dropEntry,
    emptyHistory,
    moveTo,
    recordPane,
    stepHistory,
    type PaneHistory,
    type PaneRef,
} from "./pane-history-core";

/** How long a step waits for the tab it switches to; it goes ahead after this. */
const TAB_SETTLE_CAP_MS = 1500;
/** Presses queued while a tab switch is in flight; the rest are dropped. */
const MAX_PENDING = 3;

let history: PaneHistory = emptyHistory();
/** Above 0 while a step is moving the selection, which must not be recorded. */
let navigating = 0;
/** Bumped by each selection the user makes, so steps queued before it are dropped. */
let epoch = 0;
let pending = 0;
let queue: Promise<void> = Promise.resolve();

export function paneHistorySnapshot(): PaneHistory {
    return history;
}

export function resetPaneHistory(): void {
    history = emptyHistory();
    navigating = 0;
    epoch = 0;
    pending = 0;
    queue = Promise.resolve();
}

/** Whether `ref` can still be selected: its tab is in this window and still holds the block. */
function isLive(ref: PaneRef): boolean {
    const ws = atoms.workspace();
    const tabIds = [...(ws?.pinnedtabids ?? []), ...(ws?.tabids ?? [])];
    if (!tabIds.includes(ref.tabId)) return false;
    // A tab whose object isn't loaded can't be checked here; selecting it settles the question.
    const tab = getObjectValue<Tab>(makeORef("tab", ref.tabId));
    return tab == null || (tab.blockids ?? []).includes(ref.blockId);
}

function nextFrame(): Promise<void> {
    return new Promise((resolve) => requestAnimationFrame(() => resolve()));
}

/** Select `ref`'s pane and put the caret in it. False when the pane isn't there. */
async function selectPane(ref: PaneRef): Promise<boolean> {
    if (atoms.activeTabId() !== ref.tabId) {
        await setActiveTab(ref.tabId);
        if (atoms.activeTabId() !== ref.tabId) return false;
        await whenTabContentSettled(ref.tabId, TAB_SETTLE_CAP_MS);
    }
    const model = getLayoutModelForTabById(ref.tabId);
    const root = model?.treeState?.rootNode;
    const node = root && findNodeByBlockId(root, ref.blockId);
    if (!node) return false;
    // The pane may hold the block as a dormant pane tab.
    if (node.data?.blockId !== ref.blockId) setActiveBlockInStack(model, node.id, ref.blockId);
    model.focusNode(node.id);
    // A frame later, once the view is mounted, as the layout's own caret hand-off does.
    await nextFrame();
    focusManager.refocusNode();
    return true;
}

async function step(dir: -1 | 1, startEpoch: number): Promise<void> {
    // The user selected something since this press: it no longer applies.
    if (startEpoch !== epoch) return;
    navigating++;
    try {
        for (let tries = 0; tries < 10; tries++) {
            const { history: pruned, target } = stepHistory(history, dir, isLive);
            history = pruned;
            if (!target) return;
            if (await selectPane(target)) {
                history = moveTo(history, target);
                return;
            }
            history = dropEntry(history, target);
        }
    } finally {
        // Let the selection effect see the settled pane before recording resumes.
        await nextFrame();
        navigating--;
    }
}

function enqueue(dir: -1 | 1): void {
    if (pending >= MAX_PENDING) return;
    pending++;
    const startEpoch = epoch;
    queue = queue
        .then(() => step(dir, startEpoch))
        .catch((e: unknown) => console.error("[pane-history] step failed:", e))
        .finally(() => {
            pending--;
        });
}

/** Select the pane selected before this one, switching tab if it is in another. */
export function paneHistoryBack(): void {
    enqueue(-1);
}

/** Select the pane selected after this one, after going Back. */
export function paneHistoryForward(): void {
    enqueue(1);
}

/**
 * Record every pane the window selects, from the one place that knows: the
 * active tab and its focused pane. Called once at window start.
 */
export function installPaneHistory(): () => void {
    return createRoot((dispose) => {
        createEffect(() => {
            const tabId = atoms.activeTabId();
            const blockId = focusManager.blockFocusAtom();
            if (!tabId || !blockId || navigating > 0) return;
            const next = recordPane(history, { tabId, blockId });
            if (next !== history) {
                history = next;
                epoch++;
            }
        });
        return dispose;
    });
}
