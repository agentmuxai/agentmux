// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { atoms, refocusNode, setActiveTab, MOS } from "@/app/store/global";
import { getLayoutModelForStaticTab, NavigateDirection } from "@/layout/index";
import { fireAndForget } from "@/util/util";
import { triggerTabCloseRequest } from "@/app/tab/tab-close-request";
import { WorkspaceService } from "./services";

export function getFocusedBlockInStaticTab() {
    const layoutModel = getLayoutModelForStaticTab();
    const focusedNode = layoutModel.focusedNode?.();
    return focusedNode?.data?.blockId;
}

function getStaticTabBlockCount(): number {
    const tabId = atoms.activeTabId();
    const tabORef = MOS.makeORef("tab", tabId);
    const tabAtom = MOS.getMuxObjectAtom<Tab>(tabORef);
    const tabData = tabAtom();
    return tabData?.blockids?.length ?? 0;
}

export function simpleCloseStaticTab() {
    // Route through TabBar's requestClose so the close-confirmation modal
    // (and the tab:confirmclose setting) is honoured on keyboard close
    // the same way it is on the X-button path. The last-tab guard and the
    // WorkspaceService.CloseTab + deleteLayoutModelForTab calls all live
    // inside handleClose, which requestClose delegates to. (reagent P2 #1636.)
    triggerTabCloseRequest();
}

export function genericClose() {
    const blockCount = getStaticTabBlockCount();
    if (blockCount === 0) {
        simpleCloseStaticTab();
        return;
    }
    const layoutModel = getLayoutModelForStaticTab();
    fireAndForget(layoutModel.closeFocusedNode.bind(layoutModel));
}

export function switchBlockByBlockNum(index: number) {
    const layoutModel = getLayoutModelForStaticTab();
    if (!layoutModel) {
        return;
    }
    layoutModel.switchNodeFocusByBlockNum(index);
    setTimeout(() => {
        globalRefocus();
    }, 10);
}

export function cyclePaneFocus(direction: "forward" | "backward") {
    const layoutModel = getLayoutModelForStaticTab();
    const spiralOrder = layoutModel.spiralLeafOrder?.() ?? [];
    if (spiralOrder.length <= 1) return;

    const focusedNode = layoutModel.focusedNode?.();
    const currentIndex = spiralOrder.findIndex((entry) => entry.nodeid === focusedNode?.id);

    let nextIndex: number;
    if (direction === "forward") {
        nextIndex = (currentIndex + 1) % spiralOrder.length;
    } else {
        nextIndex = (currentIndex - 1 + spiralOrder.length) % spiralOrder.length;
    }

    const nextEntry = spiralOrder[nextIndex];
    layoutModel.focusNode(nextEntry.nodeid);
    setTimeout(() => globalRefocus(), 10);
}

export function switchBlockInDirection(direction: NavigateDirection) {
    const layoutModel = getLayoutModelForStaticTab();
    layoutModel.switchNodeFocusInDirection(direction);
    setTimeout(() => {
        globalRefocus();
    }, 10);
}

function getAllTabs(ws: Workspace): string[] {
    return [...(ws.pinnedtabids ?? []), ...(ws.tabids ?? [])];
}

export function switchTabAbs(index: number) {
    const ws = atoms.workspace();
    const newTabIdx = index - 1;
    const tabids = getAllTabs(ws);
    if (newTabIdx < 0 || newTabIdx >= tabids.length) {
        return;
    }
    const newActiveTabId = tabids[newTabIdx];
    setActiveTab(newActiveTabId);
}

/** Moves the active tab one place left (-1) or right (+1) in the strip. */
export function moveActiveTab(offset: -1 | 1) {
    const ws = atoms.workspace();
    const tabids = getAllTabs(ws);
    const idx = tabids.indexOf(atoms.activeTabId());
    const to = idx + offset;
    if (idx < 0 || to < 0 || to >= tabids.length) return;
    fireAndForget(() => WorkspaceService.ReorderTab(ws.oid, tabids[idx], to));
}

/** The last tab: Chrome's "9 means last" rule for the go-to-tab keys. */
export function switchTabLast() {
    const tabids = getAllTabs(atoms.workspace());
    if (tabids.length > 0) {
        setActiveTab(tabids[tabids.length - 1]);
    }
}

export function switchTab(offset: number) {
    const ws = atoms.workspace();
    const curTabId = atoms.activeTabId();
    let tabIdx = -1;
    const tabids = getAllTabs(ws);
    for (let i = 0; i < tabids.length; i++) {
        if (tabids[i] == curTabId) {
            tabIdx = i;
            break;
        }
    }
    if (tabIdx == -1) {
        return;
    }
    const newTabIdx = (tabIdx + offset + tabids.length) % tabids.length;
    const newActiveTabId = tabids[newTabIdx];
    setActiveTab(newActiveTabId);
}

export function handleCmdI() {
    globalRefocus();
}

export function globalRefocusWithTimeout(timeoutVal: number) {
    setTimeout(() => {
        globalRefocus();
    }, timeoutVal);
}

export function globalRefocus() {
    const layoutModel = getLayoutModelForStaticTab();
    const focusedNode = layoutModel.focusedNode?.();
    if (focusedNode == null) {
        // focus a node
        layoutModel.focusFirstNode();
        return;
    }
    const blockId = focusedNode?.data?.blockId;
    if (blockId == null) {
        return;
    }
    refocusNode(blockId);
}
