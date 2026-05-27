// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * FloatingPaneWorkspace — minimal chromeless workspace for the floating
 * window opened by `open_floating_pane_window` (SPEC_FLOATING_PANE_TEAROFF
 * Phase 2 / issue #1077).
 *
 * The floating window's backend state is a normal workspace+tab+block
 * (created by `TearOffBlock` in the source window). The standard
 * `initApp` → `initHostNewWindow` path picks it up via `?workspaceId=`
 * and populates atoms the same as any new window. What changes here vs.
 * the docked `<Workspace />` component is *only* the rendered chrome:
 *
 *  - no `<WindowHeader>` (which carries the tab bar + action widgets)
 *  - no `<StatusBar>`
 *  - just a slim draggable title bar (with a close button) plus the
 *    active tab's `<TabContent>` (which contains the tile layout — for
 *    a torn-off pane that layout has exactly one leaf, the block).
 *
 * The frontend code path is otherwise identical to the docked case:
 * Block / view-model / RPC subscriptions all behave the same.
 */

import { ErrorBoundary } from "@/app/element/errorboundary";
import { CenteredDiv } from "@/app/element/quickelems";
import { ModalsRenderer } from "@/app/modals/modalsrenderer";
import { TabContent } from "@/app/tab/tabcontent";
import { atoms, getApi } from "@/store/global";
import { Show, createMemo, type JSX } from "solid-js";

function FloatingPaneWorkspaceElem(): JSX.Element {
    const tabId = atoms.activeTabId;
    const ws = atoms.workspace;

    const windowLabel = createMemo(() => {
        const params = new URLSearchParams(window.location.search);
        return params.get("windowLabel") ?? "";
    });

    const handleClose = () => {
        const label = windowLabel();
        if (!label) {
            console.warn("[floating-pane] no windowLabel — cannot close via host IPC");
            return;
        }
        getApi()
            .closeWindowByLabel(label)
            .catch((e) => console.error("[floating-pane] closeWindowByLabel failed", e));
    };

    return (
        <div class="floating-pane-workspace flex flex-col w-full flex-grow overflow-hidden">
            {/* Slim title bar.
                Drag is handled by the host's `floating_pane_wndproc`
                (agentmux-cef/src/floating_pane.rs): it returns HTCAPTION
                for the top 30 CSS px (excluding the right 36 CSS px so the
                close button click passes through). The OS handles the drag
                loop natively — cross-monitor, cross-DPI safe. The CSS
                `-webkit-app-region` properties below are kept for
                documentation/electron-compat, but are no-ops in CEF. */}
            <div
                class="floating-pane-titlebar"
                style={{
                    display: "flex",
                    "align-items": "center",
                    height: "30px",
                    "padding-left": "12px",
                    "padding-right": "4px",
                    "background-color": "var(--main-bg-color, #161620)",
                    "border-bottom": "1px solid var(--border-color, #262633)",
                    "user-select": "none",
                    "-webkit-app-region": "drag",
                } as any}
            >
                <div
                    style={{
                        flex: "1 1 auto",
                        "font-size": "12px",
                        color: "var(--secondary-text-color, #94a3b8)",
                    }}
                >
                    Floating pane
                </div>
                <button
                    type="button"
                    onClick={handleClose}
                    title="Close floating pane"
                    aria-label="Close"
                    class="floating-pane-close-btn"
                    style={{
                        width: "28px",
                        height: "22px",
                        border: "none",
                        background: "transparent",
                        color: "var(--secondary-text-color, #94a3b8)",
                        cursor: "pointer",
                        "font-size": "16px",
                        "line-height": "1",
                        "border-radius": "4px",
                        "-webkit-app-region": "no-drag",
                    } as any}
                    onMouseEnter={(e) => {
                        (e.currentTarget as HTMLElement).style.background =
                            "var(--hover-bg-color, rgba(255,255,255,0.1))";
                        (e.currentTarget as HTMLElement).style.color =
                            "var(--main-text-color, #e5e7eb)";
                    }}
                    onMouseLeave={(e) => {
                        (e.currentTarget as HTMLElement).style.background = "transparent";
                        (e.currentTarget as HTMLElement).style.color =
                            "var(--secondary-text-color, #94a3b8)";
                    }}
                >
                    ×
                </button>
            </div>

            {/* The torn-off block lives in the new workspace's active
                tab. Render that tab's TabContent only — no per-tab loop
                (there's exactly one tab) and no tab bar / widgets bar /
                status bar to surround it. */}
            <div
                class="flex flex-row flex-grow overflow-hidden"
                style={{ "min-height": 0 }}
            >
                <ErrorBoundary>
                    <Show
                        when={ws() && tabId()}
                        fallback={<CenteredDiv>Loading pane…</CenteredDiv>}
                    >
                        <ErrorBoundary>
                            <TabContent tabId={tabId()} />
                        </ErrorBoundary>
                    </Show>
                    <ModalsRenderer />
                </ErrorBoundary>
            </div>
        </div>
    );
}

export { FloatingPaneWorkspaceElem as FloatingPaneWorkspace };
