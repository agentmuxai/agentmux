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

import { invokeCommand } from "@/app/platform/ipc";
import { ErrorBoundary } from "@/app/element/errorboundary";
import { CenteredDiv } from "@/app/element/quickelems";
import { ModalsRenderer } from "@/app/modals/modalsrenderer";
import { TabContent } from "@/app/tab/tabcontent";
import { atoms, getApi } from "@/store/global";
import { Show, createEffect, createMemo, onCleanup, onMount, type JSX } from "solid-js";

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

    // Pane-header drag — JS-driven window move, scoped to the standard
    // docked-pane header (`[data-role="block-header"]`). Mirrors the
    // main-window pattern in `frontend/app/hook/useWindowDrag.win32.ts`
    // (one-in-flight + coalesce, DPR scaling, catch-up move on IPC
    // resolution, per-mousedown sequence token) — minus the
    // `data-drag-region` traversal because we scope explicitly to the
    // pane header instead of opting in via attributes.
    //
    // `preventDefault()` on the qualifying mousedown is load-bearing:
    // it suppresses the HTML5 dragstart pragmatic-dnd would otherwise
    // use to initiate a pane tear-off (TileLayout.win32.tsx:443-471).
    // Without it, dragging a floater's header would tear the block off
    // into ANOTHER floating window — the "double tear-off" bug.
    //
    // See docs/analyses/ANALYSIS_FLOATING_PANE_HEADER_DRAG_2026-05-27.md.
    onMount(() => {
        const INTERACTIVE_SELECTOR =
            "button, a, input, select, textarea, [role='button']";
        const HEADER_SELECTOR = '[data-role="block-header"]';

        let currentMouseDownId = 0;
        let dragging = false;
        let clickScreenX = 0;
        let clickScreenY = 0;
        let initWinX = 0;
        let initWinY = 0;
        let latestScreenX = 0;
        let latestScreenY = 0;
        let setPosInFlight = false;
        let pendingPos: { x: number; y: number } | null = null;

        const sendPos = (x: number, y: number): void => {
            if (setPosInFlight) {
                pendingPos = { x, y };
                return;
            }
            setPosInFlight = true;
            invokeCommand("set_window_position", { x, y })
                .catch(() => {})
                .finally(() => {
                    setPosInFlight = false;
                    if (pendingPos) {
                        const { x: nx, y: ny } = pendingPos;
                        pendingPos = null;
                        sendPos(nx, ny);
                    }
                });
        };

        const onMouseDown = async (e: MouseEvent) => {
            if (e.button !== 0) return;
            const target = e.target as HTMLElement | null;
            if (!target) return;
            // Must be inside the standard pane header
            if (!target.closest(HEADER_SELECTOR)) return;
            // Skip interactive controls within the header so close /
            // magnify / mic / endIconButton clicks reach their handlers
            if (target.closest(INTERACTIVE_SELECTOR)) return;

            // Block the HTML5 dragstart pragmatic-dnd would have used —
            // without this, the click tears the pane off into ANOTHER
            // floating window (the double-tear-off regression).
            e.preventDefault();

            currentMouseDownId += 1;
            const myId = currentMouseDownId;
            clickScreenX = e.screenX;
            clickScreenY = e.screenY;
            latestScreenX = e.screenX;
            latestScreenY = e.screenY;

            try {
                const pos = await invokeCommand<{ x: number; y: number }>(
                    "get_window_position",
                );
                // Race guard: bail if mouseup or a newer mousedown
                // happened during the IPC round-trip
                if (myId !== currentMouseDownId) return;
                initWinX = pos.x;
                initWinY = pos.y;
                dragging = true;
                // Catch-up: if the cursor moved during the IPC, fire
                // one set_window_position immediately so we don't lose
                // the first few pixels of motion
                if (
                    latestScreenX !== clickScreenX ||
                    latestScreenY !== clickScreenY
                ) {
                    const dpr = window.devicePixelRatio || 1;
                    const tx =
                        initWinX +
                        Math.round((latestScreenX - clickScreenX) * dpr);
                    const ty =
                        initWinY +
                        Math.round((latestScreenY - clickScreenY) * dpr);
                    sendPos(tx, ty);
                }
            } catch {
                // host unavailable — abort drag silently
            }
        };

        const onMouseMove = (e: MouseEvent) => {
            // Track the latest cursor position even before `dragging`
            // is armed so the catch-up at IPC resolution can use the
            // most recent value
            latestScreenX = e.screenX;
            latestScreenY = e.screenY;
            if (!dragging) return;
            // CSS-px delta * devicePixelRatio = physical-px delta added
            // to the physical-px baseline from get_window_position. Re-
            // read DPR every move so a mid-drag monitor crossing picks
            // up the new value automatically.
            const dpr = window.devicePixelRatio || 1;
            const tx =
                initWinX + Math.round((e.screenX - clickScreenX) * dpr);
            const ty =
                initWinY + Math.round((e.screenY - clickScreenY) * dpr);
            sendPos(tx, ty);
        };

        const onMouseUp = () => {
            // Invalidate any in-flight mousedown handler — incrementing
            // the id ensures their `myId !== currentMouseDownId` check
            // fires when they resolve.
            currentMouseDownId += 1;
            dragging = false;
            // Do NOT clear pendingPos — that would discard the FINAL
            // queued position (the cursor's location at release time).
            // Let the in-flight set_window_position complete; its
            // `.finally` drains pendingPos to the correct end state.
        };

        // Capture-phase listeners so we run BEFORE pragmatic-dnd's
        // bubble-phase mousedown handler — preventDefault here blocks
        // the HTML5 dragstart it would have triggered.
        document.addEventListener("mousedown", onMouseDown, true);
        document.addEventListener("mousemove", onMouseMove);
        document.addEventListener("mouseup", onMouseUp);

        onCleanup(() => {
            document.removeEventListener("mousedown", onMouseDown, true);
            document.removeEventListener("mousemove", onMouseMove);
            document.removeEventListener("mouseup", onMouseUp);
        });
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
