// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { WaveAIModel } from "@/app/aipanel/agentai-model";
import { AIPanel } from "@/app/aipanel/aipanel";
import { ErrorBoundary } from "@/app/element/errorboundary";
import { CenteredDiv } from "@/app/element/quickelems";
import { ModalsRenderer } from "@/app/modals/modalsrenderer";
import { StatusBar } from "@/app/statusbar/StatusBar";
import { WindowHeader } from "@/app/window/window-header";
import { TabContent } from "@/app/tab/tabcontent";
import { WorkspaceLayoutModel } from "@/app/workspace/workspace-layout-model";
import { getLayoutModelForStaticTab } from "@/layout/lib/layoutModelHooks";
import { atoms, refocusNode } from "@/store/global";
import { globalStore } from "@/app/store/jotaiStore";
import { useAtomValue } from "jotai";
import { memo, useEffect, useMemo, useRef } from "react";
import {
    ImperativePanelGroupHandle,
    ImperativePanelHandle,
    Panel,
    PanelGroup,
    PanelResizeHandle,
} from "react-resizable-panels";

const WorkspaceElem = memo(() => {
    const workspaceLayoutModel = WorkspaceLayoutModel.getInstance();
    const tabId = useAtomValue(atoms.activeTabId);
    const ws = useAtomValue(atoms.workspace);
    const isAIPanelVisible = useAtomValue(workspaceLayoutModel.panelVisibleAtom);
    const defaultSize = useMemo(() => workspaceLayoutModel.getDefaultSize(), []);

    const panelGroupRef = useRef<ImperativePanelGroupHandle>(null);
    const aiPanelRef = useRef<ImperativePanelHandle>(null);

    // Pass panelGroupRef to model for window resize handling
    useEffect(() => {
        workspaceLayoutModel.setPanelGroupRef(panelGroupRef.current);
        return () => workspaceLayoutModel.setPanelGroupRef(null);
    }, []);

    // Expand/collapse via useEffect — runs after React commit,
    // guaranteeing the library's panel registry is populated.
    // This is the ONLY place expand()/collapse() is called.
    useEffect(() => {
        if (!aiPanelRef.current) return;
        if (isAIPanelVisible) {
            aiPanelRef.current.expand();
        } else {
            aiPanelRef.current.collapse();
        }
    }, [isAIPanelVisible]);

    // Focus management after visibility changes
    useEffect(() => {
        if (isAIPanelVisible) {
            const opts = workspaceLayoutModel._lastSetVisibleOpts;
            if (!opts?.nofocus) {
                const timer = setTimeout(() => {
                    WaveAIModel.getInstance().focusInput();
                }, 300);
                return () => clearTimeout(timer);
            }
        } else {
            const layoutModel = getLayoutModelForStaticTab();
            const focusedNode = globalStore.get(layoutModel.focusedNode);
            if (focusedNode == null) {
                layoutModel.focusFirstNode();
            } else {
                const blockId = focusedNode?.data?.blockId;
                if (blockId != null) {
                    refocusNode(blockId);
                }
            }
        }
    }, [isAIPanelVisible]);

    // Window resize listener
    useEffect(() => {
        window.addEventListener("resize", workspaceLayoutModel.handleWindowResize);
        return () => window.removeEventListener("resize", workspaceLayoutModel.handleWindowResize);
    }, []);

    return (
        <div className="flex flex-col w-full flex-grow overflow-hidden">
            <WindowHeader key={ws.oid} workspace={ws} />
            <div className="flex flex-row flex-grow overflow-hidden" style={{ minHeight: 0 }}>
                <ErrorBoundary key={tabId}>
                    <PanelGroup direction="horizontal" ref={panelGroupRef}>
                        <Panel
                            ref={aiPanelRef}
                            collapsible
                            collapsedSize={0}
                            minSize={0}
                            defaultSize={defaultSize}
                            order={1}
                            className="overflow-hidden"
                            onResize={(size) => workspaceLayoutModel.captureResize(size)}
                            onCollapse={() => workspaceLayoutModel.onCollapsed()}
                            onExpand={() => workspaceLayoutModel.onExpanded()}
                        >
                            <AIPanel onClose={() => workspaceLayoutModel.setAIPanelVisible(false)} />
                        </Panel>
                        <PanelResizeHandle className="w-0.5 bg-transparent hover:bg-gray-500/20 transition-colors" />
                        <Panel order={2} defaultSize={100 - defaultSize}>
                            {tabId === "" ? (
                                <CenteredDiv>No Active Tab</CenteredDiv>
                            ) : (
                                <div className="flex flex-row h-full w-full">
                                    <TabContent key={tabId} tabId={tabId} />
                                </div>
                            )}
                        </Panel>
                    </PanelGroup>
                    <ModalsRenderer />
                </ErrorBoundary>
            </div>
            <StatusBar />
        </div>
    );
});

WorkspaceElem.displayName = "WorkspaceElem";

export { WorkspaceElem as Workspace };
