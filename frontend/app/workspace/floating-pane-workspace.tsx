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
 *  - no extra title bar — the floater renders the block's standard
 *    pane header (from `BlockFrame_Header` in `block/blockframe.tsx`)
 *    as its sole chrome. Dragging the pane header's title area moves
 *    the floating window via the host's `floating_pane_wndproc` →
 *    `WM_NCHITTEST → HTCAPTION` shim (excluding the rightmost ~130 CSS
 *    px where the per-pane action buttons live, so close / magnify /
 *    mic / endIconButtons remain clickable).
 *
 * The frontend code path is otherwise identical to the docked case:
 * Block / view-model / RPC subscriptions all behave the same.
 */

import { ErrorBoundary } from "@/app/element/errorboundary";
import { CenteredDiv } from "@/app/element/quickelems";
import { ModalsRenderer } from "@/app/modals/modalsrenderer";
import { TabContent } from "@/app/tab/tabcontent";
import { atoms, getApi } from "@/store/global";
import { Show, createEffect, createMemo, type JSX } from "solid-js";

function FloatingPaneWorkspaceElem(): JSX.Element {
    const tabId = atoms.activeTabId;
    const ws = atoms.workspace;

    const windowLabel = createMemo(() => {
        const params = new URLSearchParams(window.location.search);
        return params.get("windowLabel") ?? "";
    });

    // Auto-close the floating window when its workspace becomes empty —
    // a floater wraps exactly one pane today (single-block workspace),
    // so closing that pane via the standard BlockFrame_Header × button
    // should also dismiss the now-purposeless outer window. We watch
    // `workspace.blockids` and trigger close as soon as it transitions
    // from non-empty → empty (the `hadBlocks` latch avoids closing on
    // the brief empty state during initial workspace load).
    let hadBlocks = false;
    createEffect(() => {
        const w = ws();
        if (!w) return;
        const blockids = (w as { blockids?: string[] }).blockids ?? [];
        if (blockids.length > 0) {
            hadBlocks = true;
        } else if (hadBlocks) {
            const label = windowLabel();
            if (label) {
                getApi()
                    .closeWindowByLabel(label)
                    .catch((e) =>
                        console.error(
                            "[floating-pane] auto-close on empty workspace failed",
                            e,
                        ),
                    );
            }
        }
    });

    return (
        <div class="flex flex-col w-full flex-grow overflow-hidden">
            {/* The torn-off block lives in the new workspace's active
                tab. Render that tab's TabContent only — no per-tab loop
                (there's exactly one tab) and no tab bar / widgets bar /
                status bar to surround it. The block renders its standard
                `BlockFrame_Header` which serves as both the title bar
                and the action surface — exactly as it appears when
                docked. */}
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
