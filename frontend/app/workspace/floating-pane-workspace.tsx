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
 *    `BlockFrame_Header` (33 CSS px, `--header-height` in
 *    `theme.scss:97`) as its sole chrome.
 *  - window drag is **JS-driven**, installed by the `onMount` below:
 *    a targeted document mousedown listener scoped to
 *    `[data-role="block-header"]` (and skipping interactive elements
 *    via `target.closest('button, a, input, ...')`) drives
 *    `get/set_window_position` IPC. `preventDefault` on mousedown
 *    blocks the HTML5 dragstart pragmatic-dnd would otherwise have
 *    used, suppressing a "double tear-off" regression. See
 *    `docs/analyses/ANALYSIS_FLOATING_PANE_HEADER_DRAG_2026-05-27.md`
 *    for why we use JS-driven drag rather than OS HTCAPTION.
 *
 * The frontend code path is otherwise identical to the docked case:
 * Block / view-model / RPC subscriptions all behave the same.
 */

import { invokeCommand } from "@/app/platform/ipc";
import { ErrorBoundary } from "@/app/element/errorboundary";
import { CenteredDiv } from "@/app/element/quickelems";
import { ModalsRenderer } from "@/app/modals/modalsrenderer";
import { TabContent } from "@/app/tab/tabcontent";
import { WorkspaceService } from "@/store/services";
import { atoms, getApi } from "@/store/global";
import * as WOS from "@/store/wos";
import {
    setFloatingPaneInfo,
    useFloatingPaneInfo,
} from "@/app/store/floating-pane-context";
import { Show, createEffect, createMemo, onCleanup, onMount, type JSX } from "solid-js";

import "./floating-pane-workspace.scss";

function FloatingPaneWorkspaceElem(): JSX.Element {
    const tabId = atoms.activeTabId;
    const ws = atoms.workspace;

    const windowLabel = createMemo(() => {
        const params = new URLSearchParams(window.location.search);
        return params.get("windowLabel") ?? "";
    });

    // Auto-close the floating window when its only pane is closed.
    // The Workspace WaveObj has `tabids` but NO `blockids` field — the
    // block-membership signal lives on the Tab (`tab.blockids`, see
    // `frontend/types/gotypes.d.ts:1491`). We subscribe to the active
    // tab and trigger close as soon as its blockids array transitions
    // from non-empty → empty. The `hadBlocks` latch avoids closing on
    // the brief empty state during initial workspace load.
    //
    // useWaveObjectValue installs an onCleanup tied to the surrounding
    // reactive owner — calling it inside createEffect refreshes the
    // subscription whenever tabId changes (the prior effect run's
    // cleanup decrements the previous refcount).
    let hadBlocks = false;
    createEffect(() => {
        const tid = tabId();
        if (!tid) return;
        const [tab] = WOS.useWaveObjectValue<Tab>(WOS.makeORef("tab", tid));
        const t = tab();
        if (!t) return;
        const blockids = t.blockids ?? [];
        if (blockids.length > 0) {
            hadBlocks = true;
        } else if (hadBlocks) {
            const label = windowLabel();
            if (label) {
                getApi()
                    .closeWindowByLabel(label)
                    .catch((e) =>
                        console.error(
                            "[floating-pane] auto-close on empty tab failed",
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

        // Capture once per mount — windowLabel is fixed for the
        // floater's lifetime. Used to route `get/set_window_position`
        // IPC to OUR HWND (not whichever top-level the OS happens to
        // enumerate first in Z-order).
        const label = windowLabel();

        // Publish floating-pane context so BlockFrame's EndIcons renders
        // the maximize button + read-back can answer "are we maximized
        // right now" for the drag-from-maximized branch below. State is
        // seeded from a one-shot `get_window_state` query so a fresh
        // floater (or one that was already maximized via a programmatic
        // ShowWindow elsewhere) starts with the right icon glyph.
        if (label) {
            setFloatingPaneInfo({ windowLabel: label, state: "normal" });
            invokeCommand<{ state: "normal" | "maximized" | "minimized" }>(
                "get_window_state",
                { label },
            )
                .then((res) => {
                    if (res?.state === "maximized") {
                        setFloatingPaneInfo({
                            windowLabel: label,
                            state: "maximized",
                        });
                    }
                })
                .catch(() => {});
        }
        onCleanup(() => setFloatingPaneInfo(null));

        const sendPos = (x: number, y: number): void => {
            if (setPosInFlight) {
                pendingPos = { x, y };
                return;
            }
            setPosInFlight = true;
            invokeCommand("set_window_position", { x, y, label })
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

            // Drag-from-maximized restore. Windows Explorer-style: dragging
            // a maximized window's "title" restores it under the cursor.
            // We replicate manually because the JS-driven drag suppresses
            // the OS WM_NCLBUTTONDOWN/HTCAPTION path that would otherwise
            // do this for free. The host's `restore_window_and_move` is
            // atomic — no flash of the maximized rect at its old origin
            // between SW_RESTORE and SetWindowPos. See spec §4.2 / §5.4.
            const currentState = useFloatingPaneInfo()?.state;
            if (currentState === "maximized") {
                let stash: {
                    left: number;
                    top: number;
                    w: number;
                    h: number;
                } | null = null;
                try {
                    stash = await invokeCommand<{
                        left: number;
                        top: number;
                        w: number;
                        h: number;
                    } | null>("consume_restored_rect", { label });
                } catch {
                    stash = null;
                }
                if (myId !== currentMouseDownId) return;
                const dpr = window.devicePixelRatio || 1;
                // Anchor the restored rect under the cursor. Centre the
                // header width on the cursor (matches the Explorer feel)
                // and place the cursor ~16 px below the top edge so it
                // lands in the middle of the pane header. Fall back to a
                // reasonable default size if no stash existed (shouldn't
                // happen — maximize_window stashes — but defensive).
                const restoredW = stash?.w ?? Math.round(800 * dpr);
                const restoredH = stash?.h ?? Math.round(600 * dpr);
                const cursorPhysX = Math.round(e.screenX * dpr);
                const cursorPhysY = Math.round(e.screenY * dpr);
                const restoredX = cursorPhysX - Math.round(restoredW / 2);
                const restoredY = cursorPhysY - 16;
                try {
                    await invokeCommand("restore_window_and_move", {
                        label,
                        x: restoredX,
                        y: restoredY,
                        w: restoredW,
                        h: restoredH,
                    });
                } catch (err) {
                    console.error(
                        "[floating-pane] restore_window_and_move failed",
                        err,
                    );
                    return;
                }
                if (myId !== currentMouseDownId) return;
                setFloatingPaneInfo({
                    windowLabel: label,
                    state: "normal",
                });
                // Baseline the drag loop on the post-restore origin so
                // mousemove deltas land relative to where the floater now
                // sits, not where the maximized rect used to be.
                initWinX = restoredX;
                initWinY = restoredY;
                dragging = true;
                return;
            }

            try {
                const pos = await invokeCommand<{ x: number; y: number }>(
                    "get_window_position",
                    { label },
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

        // Throttled redock-hover push — drives the highlight overlay
        // on whichever agentmux window the cursor is currently over.
        // ~50 ms cadence is plenty for highlight tracking and keeps
        // IPC load minimal during fast drags.
        const HOVER_PUSH_THROTTLE_MS = 50;
        let lastHoverPushAt = 0;
        const pushRedockHover = (screenX: number, screenY: number) => {
            const now = performance.now();
            if (now - lastHoverPushAt < HOVER_PUSH_THROTTLE_MS) return;
            lastHoverPushAt = now;
            const dpr = window.devicePixelRatio || 1;
            const px = Math.round(screenX * dpr);
            const py = Math.round(screenY * dpr);
            const sourceLabel = windowLabel();
            if (!sourceLabel) return;
            invokeCommand("update_floating_redock_hover", {
                source_label: sourceLabel,
                x: px,
                y: py,
            }).catch(() => {});
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
            pushRedockHover(e.screenX, e.screenY);
        };

        const onMouseUp = (e: MouseEvent) => {
            // Invalidate any in-flight mousedown handler — incrementing
            // the id ensures their `myId !== currentMouseDownId` check
            // fires when they resolve.
            currentMouseDownId += 1;
            const wasDragging = dragging;
            dragging = false;
            // Do NOT clear pendingPos — that would discard the FINAL
            // queued position (the cursor's location at release time).
            // Let the in-flight set_window_position complete; its
            // `.finally` drains pendingPos to the correct end state.

            // Floating-pane re-dock (Phase 4a MVP): if we just finished
            // a drag and the cursor landed over another agentmux window
            // in this process, fire RedockFloatingPane to move our
            // single block into that window's active tab. The source
            // workspace's tab.blockids becomes empty → the createEffect
            // above auto-closes this floater.
            //
            // Cross-instance / cross-version safety is free: the host's
            // `resolve_window_at_cursor` looks up HWND in this process's
            // `window_hwnds` map — another agentmux version's HWNDs
            // aren't in it, so cross-process drops silently no-op.
            if (wasDragging) {
                // Always clear the hover state on release — the highlight
                // overlay needs to disappear whether or not we ended up
                // over a target. Fire-and-forget; the redock attempt
                // below doesn't depend on this completing.
                invokeCommand("clear_floating_redock_hover", {}).catch(() => {});
                void tryRedockAtCursor(e.screenX, e.screenY);
            }
        };

        const tryRedockAtCursor = async (screenX: number, screenY: number) => {
            const ourLabel = windowLabel();
            if (!ourLabel) return;
            // `mouseup.screenX/Y` are CSS pixels; host expects physical.
            const dpr = window.devicePixelRatio || 1;
            const px = Math.round(screenX * dpr);
            const py = Math.round(screenY * dpr);

            let target: { label: string | null; window_id: string | null };
            try {
                // exclude_label = our own floater's label. Without this,
                // the host's Z-order walk would return the floater itself
                // — it's at the cursor (the JS-driven drag follows the
                // cursor), so it's always topmost where the cursor is.
                target = await invokeCommand<{
                    label: string | null;
                    window_id: string | null;
                }>("resolve_window_at_cursor", {
                    x: px,
                    y: py,
                    exclude_label: ourLabel,
                });
            } catch (e) {
                console.error("[floating-pane] resolve_window_at_cursor failed", e);
                return;
            }
            if (!target.label || !target.window_id) {
                // Cursor over desktop, external app, or our own floater
                // — leave floater at the dropped position.
                return;
            }

            // Resolve target's active tab via the WaveObj graph:
            // window → workspace.activetabid. Use the non-pinning
            // `reloadWaveObject` — `loadAndPinWaveObject` would bump
            // `refCount` with no matching unpin in this async flow,
            // leaking one ref on each of the target Window + Workspace
            // per successful redock so the cache cleanup never evicts
            // them.
            let targetWs: Workspace;
            try {
                const targetWindow = await WOS.reloadWaveObject<WaveWindow>(
                    WOS.makeORef("window", target.window_id),
                );
                targetWs = await WOS.reloadWaveObject<Workspace>(
                    WOS.makeORef("workspace", targetWindow.workspaceid),
                );
            } catch (e) {
                console.error(
                    "[floating-pane] failed to resolve target window's workspace",
                    e,
                );
                return;
            }
            const targetTabId = targetWs.activetabid;
            const targetWsId = targetWs.oid;
            if (!targetTabId || !targetWsId) {
                console.warn(
                    "[floating-pane] target window has no active tab — skipping redock",
                );
                return;
            }

            // Source identifiers — the floater's only-tab + only-block.
            const sourceTabId = tabId();
            const sourceWs = ws();
            if (!sourceTabId || !sourceWs) return;
            const sourceWsId = sourceWs.oid;
            // Non-reactive read: `useWaveObjectValue` would register an
            // `onCleanup` against the current reactive owner, but we're inside
            // an async mouseup callback with no owner — the refCount would
            // never get decremented and we'd leak a Tab subscription per drop.
            const sourceTabObj = WOS.getObjectValue<Tab>(
                WOS.makeORef("tab", sourceTabId),
            );
            const sourceBlockId = sourceTabObj?.blockids?.[0];
            if (!sourceBlockId) {
                console.warn(
                    "[floating-pane] floater has no block to redock — skipping",
                );
                return;
            }

            try {
                await WorkspaceService.RedockFloatingPane(
                    sourceBlockId,
                    sourceTabId,
                    sourceWsId,
                    targetTabId,
                    targetWsId,
                );
                // After successful redock, source tab.blockids empties
                // → the auto-close watcher dismisses the floater.
            } catch (e) {
                console.error("[floating-pane] RedockFloatingPane failed", e);
            }
        };

        // Double-click on the pane header toggles maximize, same
        // affordance as the explicit header button. Mirrors the main-
        // window pattern in `frontend/app/hook/useWindowDrag.win32.ts`
        // (PR #315): dblclick capture-phase listener gated on header
        // scope + non-interactive target. The toggle's return value
        // drives the icon state so no extra IPC is needed. See spec
        // §4.2.
        const onDblClick = async (e: MouseEvent) => {
            if (e.button !== 0) return;
            const target = e.target as HTMLElement | null;
            if (!target) return;
            if (!target.closest(HEADER_SELECTOR)) return;
            if (target.closest(INTERACTIVE_SELECTOR)) return;
            e.preventDefault();
            // Suppress the header's bubble-phase onDblClick that toggles
            // pane magnify (`blockframe.tsx:405`). Inside the floating
            // shell, dblclick on the header should ONLY maximize the
            // window — magnifying a pane that already fills the entire
            // floater is meaningless and visually broken (the magnify
            // overlay tries to ride alongside the tile, but the floater
            // has no other tile).
            e.stopPropagation();
            // Cancel any in-flight mousedown drag so the dblclick's
            // second mousedown doesn't leave `dragging=true` behind.
            currentMouseDownId += 1;
            dragging = false;
            try {
                const res = await invokeCommand<{
                    state: "maximized" | "normal";
                }>("maximize_window", { label });
                setFloatingPaneInfo({
                    windowLabel: label,
                    state: res?.state === "maximized" ? "maximized" : "normal",
                });
            } catch (err) {
                console.error("[floating-pane] dblclick maximize_window failed", err);
            }
        };

        // Capture-phase listeners so we run BEFORE pragmatic-dnd's
        // bubble-phase mousedown handler — preventDefault here blocks
        // the HTML5 dragstart it would have triggered.
        document.addEventListener("mousedown", onMouseDown, true);
        document.addEventListener("mousemove", onMouseMove);
        document.addEventListener("mouseup", onMouseUp);
        document.addEventListener("dblclick", onDblClick, true);

        onCleanup(() => {
            document.removeEventListener("mousedown", onMouseDown, true);
            document.removeEventListener("mousemove", onMouseMove);
            document.removeEventListener("mouseup", onMouseUp);
            document.removeEventListener("dblclick", onDblClick, true);
        });
    });

    return (
        <div class="floating-pane-workspace flex flex-col w-full flex-grow overflow-hidden">
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
