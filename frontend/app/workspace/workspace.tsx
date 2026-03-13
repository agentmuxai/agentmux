// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { ErrorBoundary } from "@/app/element/errorboundary";
import { CenteredDiv } from "@/app/element/quickelems";
import { ModalsRenderer } from "@/app/modals/modalsrenderer";
import { StatusBar } from "@/app/statusbar/StatusBar";
import { WindowHeader } from "@/app/window/window-header";
import { TabContent } from "@/app/tab/tabcontent";
import { atoms } from "@/store/global";
import { Show } from "solid-js";
import type { JSX } from "solid-js";

function WorkspaceElem(): JSX.Element {
    const tabId = atoms.activeTabId;
    const ws = atoms.workspace;

    return (
        <div class="flex flex-col w-full flex-grow overflow-hidden">
            <WindowHeader workspace={ws()} />
            <div class="flex flex-row flex-grow overflow-hidden" style={{ "min-height": 0 }}>
                <ErrorBoundary>
                    <Show
                        when={tabId()}
                        keyed
                        fallback={<CenteredDiv>No Active Tab</CenteredDiv>}
                    >
                        {(currentTabId) => (
                            <div class="flex flex-row h-full w-full">
                                <TabContent tabId={currentTabId} />
                            </div>
                        )}
                    </Show>
                    <ModalsRenderer />
                </ErrorBoundary>
            </div>
            <StatusBar />
        </div>
    );
}

export { WorkspaceElem as Workspace };
