// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { atoms, createTab, setActiveTab } from "@/store/global";
import { fireAndForget } from "@/util/util";
import { useAtomValue } from "jotai";
import { memo, useCallback } from "react";
import { WorkspaceService } from "../store/services";
import { deleteLayoutModelForTab } from "@/layout/index";
import { Tab } from "./tab";
import "./tabbar.scss";

interface TabBarProps {
    workspace: Workspace;
}

const TabBar = memo(({ workspace }: TabBarProps) => {
    const activeTabId = useAtomValue(atoms.activeTabId);

    const pinnedTabIds = workspace?.pinnedtabids ?? [];
    const regularTabIds = workspace?.tabids ?? [];
    const allTabIds = [...pinnedTabIds, ...regularTabIds];

    const handleSelect = useCallback((tabId: string) => {
        if (tabId !== activeTabId) {
            setActiveTab(tabId);
        }
    }, [activeTabId]);

    const handleClose = useCallback((tabId: string) => {
        const allTabs = [...pinnedTabIds, ...regularTabIds];
        if (allTabs.length <= 1) return; // never close last tab

        fireAndForget(async () => {
            // If closing the active tab, switch to an adjacent tab first
            // and await the backend round-trip to prevent race conditions
            if (tabId === activeTabId) {
                const idx = allTabs.indexOf(tabId);
                const nextTab = allTabs[idx + 1] ?? allTabs[idx - 1];
                if (nextTab) {
                    await setActiveTab(nextTab);
                }
            }
            await WorkspaceService.CloseTab(workspace.oid, tabId);
            deleteLayoutModelForTab(tabId);
        });
    }, [workspace?.oid, pinnedTabIds, regularTabIds, activeTabId]);

    const handlePinChange = useCallback((tabId: string) => {
        const isPinned = pinnedTabIds.includes(tabId);
        const newPinnedIds = isPinned
            ? pinnedTabIds.filter((id) => id !== tabId)
            : [...pinnedTabIds, tabId];
        const newRegularIds = isPinned
            ? [...regularTabIds, tabId]
            : regularTabIds.filter((id) => id !== tabId);
        fireAndForget(() => WorkspaceService.UpdateTabIds(workspace.oid, newRegularIds, newPinnedIds));
    }, [workspace?.oid, pinnedTabIds, regularTabIds]);

    const handleAddTab = useCallback(() => {
        createTab();
    }, []);

    // noop for drag (no drag-and-drop in v1)
    const noop = useCallback(() => {}, []);

    if (!workspace) return null;

    const activeIndex = allTabIds.indexOf(activeTabId);

    return (
        <div className="tab-bar" data-tauri-drag-region="false">
            <button className="add-tab-btn" onClick={handleAddTab} title="New Tab">
                <i className="fa fa-plus" />
            </button>
            <div className="tab-bar-scroll">
                {pinnedTabIds.map((tabId, i) => {
                    const idx = i;
                    const isActive = tabId === activeTabId;
                    const isBeforeActive = idx === activeIndex - 1;
                    return (
                        <Tab
                            key={tabId}
                            ref={null}
                            id={tabId}
                            active={isActive}
                            isFirst={i === 0}
                            isBeforeActive={isBeforeActive}
                            isDragging={false}
                            tabWidth={0}
                            isNew={false}
                            isPinned={true}
                            onSelect={() => handleSelect(tabId)}
                            onClose={() => handleClose(tabId)}
                            onDragStart={noop}
                            onLoaded={noop}
                            onPinChange={() => handlePinChange(tabId)}
                        />
                    );
                })}
                {pinnedTabIds.length > 0 && <div className="pinned-tab-spacer" />}
                {regularTabIds.map((tabId, i) => {
                    const idx = pinnedTabIds.length + i;
                    const isActive = tabId === activeTabId;
                    const isBeforeActive = idx === activeIndex - 1;
                    return (
                        <Tab
                            key={tabId}
                            ref={null}
                            id={tabId}
                            active={isActive}
                            isFirst={pinnedTabIds.length === 0 && i === 0}
                            isBeforeActive={isBeforeActive}
                            isDragging={false}
                            tabWidth={0}
                            isNew={false}
                            isPinned={false}
                            onSelect={() => handleSelect(tabId)}
                            onClose={() => handleClose(tabId)}
                            onDragStart={noop}
                            onLoaded={noop}
                            onPinChange={() => handlePinChange(tabId)}
                        />
                    );
                })}
            </div>
        </div>
    );
});

TabBar.displayName = "TabBar";

export { TabBar };
