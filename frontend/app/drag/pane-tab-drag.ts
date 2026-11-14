// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A pane-tab (pill) drag on the drag session, and its document-tab sibling: begun by the pill's onDragStart,
 * marked released by its onDrop, ended by the cross-window monitor's dragend,
 * which still reads `escaped` from it.
 * docs/specs/SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md §5.1.
 */

import { homeLayoutOf } from "@/layout/lib/homeLayout";
import type { LayoutModel } from "@/layout/lib/layoutModel";
import { getLayoutModelForTabById } from "@/layout/lib/layoutModelHooks";
import { findNode } from "@/layout/lib/layoutNode";
import { landingPictureOf } from "@/layout/lib/paneLandingPicture";
import { pillPlacingIn } from "@/layout/lib/pillDragState";
import { effectiveStack } from "@/layout/lib/stackMembers";
import {
    beginTabTearOff,
    endTabTearOff,
    holdTabTearOff,
    tabTearOffHeld,
    tabTearOffOffered,
    tabTearOffShown,
} from "@/layout/lib/tabTearOff";
import { beginDrag, isUnderway, markReleased, onSessionEnded, session } from "./drag-session";
import { watchEscapeDuringDrag } from "./escape-watch";
import { endedByEscape } from "./native-cancel";
import type { HomeLayout } from "./pane-live-follow";
import { ensureTabPicture } from "./tearoff-snapshot";

/** Stops the window watch of the tab drag that is on (or of the last one, until its `dragend`). */
let stopWindowWatch: (() => void) | undefined;
/** The pointer of the tab drag that is on is outside the window. */
let outside = false;

/**
 * The pointer of a tab's drag left the window: a release out there tears the tab off, so its own place stops
 * offering that (tabTearOff.ts). A `dragleave` to nowhere is the pointer leaving the window and a `dragenter`
 * from nowhere is it coming back, as for a pane's drag (layout/lib/tile-drag.ts). A `dragend` with no drop
 * effect may be Escape: then nothing is torn off, and the tab is back in its pane.
 */
function watchWindowExit(): void {
    stopWindowWatch?.();
    outside = false;
    if (typeof document === "undefined") return;
    const left = (e: DragEvent) => {
        if (e.relatedTarget !== null) return;
        outside = true;
        tabTearOffShown();
    };
    const entered = (e: DragEvent) => {
        if (e.relatedTarget === null) outside = false;
    };
    const ended = (e: DragEvent) => {
        stop();
        if (e.dataTransfer?.dropEffect !== "none") return;
        void endedByEscape("none").then((cancelled) => {
            if (cancelled) endTabTearOff();
        });
    };
    const stop = () => {
        document.removeEventListener("dragleave", left);
        document.removeEventListener("dragenter", entered);
        document.removeEventListener("dragend", ended);
        if (stopWindowWatch === stop) stopWindowWatch = undefined;
    };
    document.addEventListener("dragleave", left);
    document.addEventListener("dragenter", entered);
    document.addEventListener("dragend", ended);
    stopWindowWatch = stop;
}

// The drag is over. A release that tears the tab off leaves the pane showing the tab gone until the tear-off
// is done (pane-tab-tearoff.ts ends it): one outside the window, held here, or one over the tab's own place
// while that offered it, held by the pane that got the release (pane-tab-foreign-drop.ts). Any other end puts
// the pane back as its tabs say.
onSessionEnded(({ session: s }) => {
    if (s.kind !== "pane-tab") return;
    if (outside) holdTabTearOff();
    else if (!tabTearOffHeld()) endTabTearOff();
});

/**
 * The drag of one tab out of a pane that keeps others: its place in the pane offers the tear-off
 * (tabTearOff.ts), and its picture is taken now, of the tab as it stands, before anything of the drag is
 * drawn over it. False for a pane's only tab, which moves the pane.
 */
function liftTab(model: LayoutModel, blockId: string, paneKey: string): boolean {
    const root = model.treeState.rootNode;
    const node = root && findNode(root, paneKey);
    if (!node?.data || effectiveStack(node.data).length <= 1) return false;
    ensureTabPicture(blockId);
    void landingPictureOf(blockId);
    beginTabTearOff(model, blockId, paneKey);
    watchWindowExit();
    return true;
}

/**
 * The home layout of a lifted tab's drag. The layout shows the tab only while its own place offers the tear-off
 * or a drop is shown for it (its picture in the landing place): anywhere else over the layout (a header it
 * would join, its own pane once it has left it) the hand carries the picture, or the tab would be nowhere.
 */
function tabHome(base: HomeLayout, model: LayoutModel, blockId: string): HomeLayout {
    return { ...base, holdsPane: () => base.holdsPane() && (tabTearOffOffered(blockId) || pillPlacingIn(model)) };
}

/** `paneKey` is the pane the pill belongs to; `tabId` its window tab. `blockIds` is the
 *  whole selection when several tabs are dragged together (the grabbed one included). */
export function startPaneTabDrag(blockId: string, paneKey?: string, tabId?: string, blockIds?: readonly string[]): void {
    const several = !!blockIds && blockIds.length > 1;
    beginDrag("pane-tab", { blockId, nodeId: paneKey, tabId, ...(several ? { blockIds } : {}) });
    // Only a single tab with a window tab can tear off, so only that one is followed (pane-live-follow.ts), and
    // its picture is carried outside its own layout's body only, as a dragged pane's is.
    const model = !several && paneKey && tabId ? getLayoutModelForTabById(tabId) : undefined;
    const lifted = !!model && liftTab(model, blockId, paneKey!);
    const base = model && homeLayoutOf(model, () => isUnderway("pane-tab"));
    const home = base && lifted ? tabHome(base, model!, blockId) : base;
    watchEscapeDuringDrag(!several && paneKey && tabId ? { blockId, ...(home ? { home } : {}) } : undefined);
}

export function releasePaneTabDrag(): void {
    if (session()?.kind === "pane-tab") markReleased();
}

/** A document tab (`tabId`, in block `blockId`'s strip) on the drag session.
 *  The same life as a pane-tab drag, without the tear-off. */
export function startDocTabDrag(blockId: string, tabId: string): void {
    beginDrag("doc-tab", { blockId, itemId: tabId });
}

export function releaseDocTabDrag(): void {
    if (session()?.kind === "doc-tab") markReleased();
}

/** Whether document tab `tabId` of block `blockId` is being dragged. Reactive. */
export function isDraggedDocTab(blockId: string, tabId: string): boolean {
    const source = session()?.source;
    return isUnderway("doc-tab") && source?.blockId === blockId && source?.itemId === tabId;
}

/**
 * Whether this pane's pill for `blockId` is being dragged, until its source
 * releases it. The pane matters: the same block can have a pill in more than
 * one strip. Reactive.
 */
export function isDraggedPaneTab(blockId: string, paneKey?: string): boolean {
    const source = session()?.source;
    if (!isUnderway("pane-tab")) return false;
    // A multi-tab drag dims every selected pill, in whichever strip it is.
    if (source?.blockIds?.includes(blockId)) return true;
    return source?.blockId === blockId && source?.nodeId === paneKey;
}
