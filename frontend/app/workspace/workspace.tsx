// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { ErrorBoundary } from "@/app/element/errorboundary";
import { CenteredDiv } from "@/app/element/quickelems";
import { ModalsRenderer } from "@/app/modals/modalsrenderer";
import { StatusBar } from "@/app/statusbar/StatusBar";
import { WindowHeader } from "@/app/window/window-header";
import { TabContent } from "@/app/tab/tabcontent";
import { atoms } from "@/store/global";
import { markEnd, markStart } from "@/perf";
import { createEffect, For, Show, createMemo } from "solid-js";
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

    // Phase 0.5 reactive perf mark for tab switches. The Phase-0 mark
    // lived in `tabbar.tsx::handleSelect` — only fired on a user click
    // on the tab strip. Programmatic switches (`workspace.SetActiveTab`
    // via the service API), keyboard-shortcut switches, and any other
    // path that writes `workspace.activetabid` bypassed the click
    // handler entirely. Same lesson surfaced by the Phase 1 baseline
    // retro: imperative marks at the click site miss every alternate
    // entry point.
    //
    // This `createEffect` subscribes to `atoms.activeTabId` (a
    // createMemo over `workspace.activetabid`) and emits the
    // `tab-switch` measure on every change regardless of source. The
    // first run establishes the baseline tabId; only subsequent
    // changes count as switches.
    let prevTabId: string | undefined;
    createEffect(() => {
        const next = tabId();
        if (prevTabId === undefined) {
            // First run on mount — record the initial tabId without
            // emitting a switch measure. The very first reactive read
            // is a subscription, not a transition.
            prevTabId = next;
            return;
        }
        if (next === prevTabId) return;
        markStart("tab-switch", { from: prevTabId, to: next });
        prevTabId = next;
        // markEnd in the next microtask captures the synchronous
        // dispatch path: the workspace atom write, the activeTabId
        // memo recomputation, this effect's run, and any other
        // effects that subscribe to activeTabId. Reactive fan-out
        // beyond microtask boundaries (IPC for pane HWND show/hide,
        // for example) is observed separately via the Long Tasks
        // observer + IPC roundtrip clock.
        queueMicrotask(() => markEnd("tab-switch"));
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
