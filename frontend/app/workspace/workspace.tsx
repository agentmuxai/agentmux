// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { ErrorBoundary } from "@/app/element/errorboundary";
import { CenteredDiv } from "@/app/element/quickelems";
import { ModalsRenderer } from "@/app/modals/modalsrenderer";
import { StatusBar } from "@/app/statusbar/StatusBar";
import { WindowHeader } from "@/app/window/window-header";
import { TabContent } from "@/app/tab/tabcontent";
import { atoms } from "@/store/global";
import { For, Show, createMemo } from "solid-js";
import type { JSX } from "solid-js";

function WorkspaceElem(): JSX.Element {
    const tabId = atoms.activeTabId;
    const ws = atoms.workspace;

    // All tab IDs (pinned + regular). Keep every tab mounted so terminals
    // preserve their xterm.js instance and scrollback across tab switches.
    // Inactive tabs are hidden via display:none — no unmount/remount.
    const allTabIds = createMemo<string[]>(() => {
        const w = ws();
        if (!w) return [];
        return [...(w.pinnedtabids ?? []), ...(w.tabids ?? [])];
    });

    return (
        <div class="flex flex-col w-full flex-grow overflow-hidden">
            <WindowHeader workspace={ws()} />
            <div class="flex flex-row flex-grow overflow-hidden" style={{ "min-height": 0 }}>
                <ErrorBoundary>
                    <Show when={allTabIds().length > 0} fallback={<CenteredDiv>No Active Tab</CenteredDiv>}>
                        <For each={allTabIds()}>
                            {(tid) => (
                                <div
                                    class="flex flex-row h-full w-full"
                                    style={{ display: tid === tabId() ? "flex" : "none" }}
                                >
                                    <ErrorBoundary>
                                        <TabContent tabId={tid} />
                                    </ErrorBoundary>
                                </div>
                            )}
                        </For>
                    </Show>
                    <ModalsRenderer />
                </ErrorBoundary>
            </div>
            <StatusBar />
        </div>
    );
}

export { WorkspaceElem as Workspace };
