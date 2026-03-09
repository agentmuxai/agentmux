// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { ErrorBoundary } from "@/app/element/errorboundary";
import { CenteredDiv } from "@/app/element/quickelems";
import { ModalsRenderer } from "@/app/modals/modalsrenderer";
import { StatusBar } from "@/app/statusbar/StatusBar";
import { WindowHeader } from "@/app/window/window-header";
import { TabContent } from "@/app/tab/tabcontent";
import { atoms } from "@/store/global";
import { useAtomValue } from "jotai";
import { memo } from "react";

const WorkspaceElem = memo(() => {
    const tabId = useAtomValue(atoms.activeTabId);
    const ws = useAtomValue(atoms.workspace);

    return (
        <div className="flex flex-col w-full flex-grow overflow-hidden">
            <WindowHeader key={ws.oid} workspace={ws} />
            <div className="flex flex-row flex-grow overflow-hidden" style={{ minHeight: 0 }}>
                <ErrorBoundary key={tabId}>
                    {tabId === "" ? (
                        <CenteredDiv>No Active Tab</CenteredDiv>
                    ) : (
                        <div className="flex flex-row h-full w-full">
                            <TabContent key={tabId} tabId={tabId} />
                        </div>
                    )}
                    <ModalsRenderer />
                </ErrorBoundary>
            </div>
            <StatusBar />
        </div>
    );
});

WorkspaceElem.displayName = "WorkspaceElem";

export { WorkspaceElem as Workspace };
