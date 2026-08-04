// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { atoms, refocusNode, setActiveTab, WOS } from "@/app/store/global";
import { getLayoutModelForStaticTab, NavigateDirection } from "@/layout/index";
import { fireAndForget } from "@/util/util";
import { triggerTabCloseRequest } from "@/app/tab/tab-close-request";
import { debugLog } from "./keymodel-debuglog";

export function getFocusedBlockInStaticTab() {
    const layoutModel = getLayoutModelForStaticTab();
    const focusedNode = layoutModel.focusedNode?.();
    return focusedNode?.data?.blockId;
}

function getStaticTabBlockCount(): number {
    const tabId = atoms.activeTabId();
    const tabORef = WOS.makeORef("tab", tabId);
    const tabAtom = WOS.getWaveObjectAtom<Tab>(tabORef);
    const tabData = tabAtom();
    return tabData?.blockids?.length ?? 0;
}

export function simpleCloseStaticTab() {
    debugLog("simpleCloseStaticTab called");
    // Route through TabBar's requestClose so the close-confirmation modal
    // (and the tab:skipcloseconfirm setting) is honoured on keyboard close
    // the same way it is on the X-button path. The last-tab guard and the
    // WorkspaceService.CloseTab + deleteLayoutModelForTab calls all live
    // inside handleClose, which requestClose delegates to. (reagent P2 #1636.)
    triggerTabCloseRequest();
}

export function genericClose() {
    debugLog("genericClose called");
    const blockCount = getStaticTabBlockCount();
    debugLog("genericClose blockCount", blockCount);
    if (blockCount === 0) {
        debugLog("genericClose calling simpleCloseStaticTab because blockCount is 0");
        simpleCloseStaticTab();
        return;
    }
    debugLog("genericClose calling closeFocusedNode");
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
    console.log("switchTabAbs", index);
    const ws = atoms.workspace();
    const newTabIdx = index - 1;
    const tabids = getAllTabs(ws);
    if (newTabIdx < 0 || newTabIdx >= tabids.length) {
        return;
    }
    const newActiveTabId = tabids[newTabIdx];
    setActiveTab(newActiveTabId);
}

export function switchTab(offset: number) {
    console.log("switchTab", offset);
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
