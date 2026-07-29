// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Logger } from "@/util/logger";
import { draggable, dropTargetForElements } from "@atlaskit/pragmatic-drag-and-drop/element/adapter";
import { preventUnhandled } from "@atlaskit/pragmatic-drag-and-drop/prevent-unhandled";
import { isWindows } from "@/util/platformutil";
import { createMemo, createSignal, onCleanup, onMount } from "solid-js";
import type { JSX } from "solid-js";
import clsx from "clsx";
import { Tab } from "./tab";
import {
    tabItemType,
    GAP_PX,
    globalDragTabId,
    setGlobalDragTabId,
    insertionPoint,
    setInsertionPoint,
    bouncingTabId,
    hoveredDropTabId,
    setHoveredDropTabId,
    tabWrapperRefs,
    SPRING_SWITCH_MS,
    dragActivatedTabIds,
    setDragEscaped,
    setBouncingTabId,
    computeInsertionPoint,
    TEAR_PAST_PX,
} from "./tabbar-dnd";
import { getCurrentDragPayload, setCurrentDragPayload } from "@/app/drag/CrossWindowDragMonitor";
import { getLayoutModelForTabById, redockDraggedPane, tileItemType } from "@/layout/index";
import { getApi } from "@/store/global";
import { fireAndForget } from "@/util/util";
import { setTabGrabOffset } from "./tab-grab-offset";
import { attachNativePointerDragTracker } from "@/app/drag/native-pointer-drag-tracker";
import { executeReorder } from "./tab-reorder";
import type { NativeTearOffFn } from "./tab-tearoff-rpc";
import { WorkspaceService } from "../store/services";

export interface DroppableTabProps {
    tabId: string;
    workspaceId: string;
    activeTabId: string;
    isActive: boolean;
    isFirst: boolean;
    isBeforeActive: boolean;
    allTabCount: number;
    tabIndex: number;      // index into tabIds — used for activeIndex math and ReorderTab
    tabIds: string[];
    onSelect: () => void;
    onClose: () => void;
    // SPEC_NATIVE_POINTER_DRAG_TEAROFF_2026_07_28 — Windows-only native
    // pointer-drag wiring. Unused (but still required) on macOS/Linux,
    // which keep the pragmatic-dnd draggable() path below unchanged.
    tabBarScrollRef: () => HTMLDivElement;
    requestNativeTearOff: NativeTearOffFn;
}

export function DroppableTab(props: DroppableTabProps): JSX.Element {
    let tabWrapRef!: HTMLDivElement;
    const [isDragging, setIsDragging] = createSignal(false);
    const [naturalWidth, setNaturalWidth] = createSignal<number | null>(null);
    // Own window label — drives the lone-tab drag policy below (a
    // standalone torn-off window's single tab must be draggable to
    // remount it into another window; main's never is). Read
    // SYNCHRONOUSLY from the URL param (the same authoritative source
    // app-init.ts uses) rather than the async getWindowLabel() IPC: a
    // freshly torn-off window is exactly the primary use case, and an
    // async fetch would leave its lone tab undraggable until the
    // promise resolved. (reagent PR #2086 P2)
    const windowLabel = new URLSearchParams(window.location.search).get("windowLabel") || "main";
    // Lone-tab drags are a REMOUNT-ONLY gesture (cross-window tab
    // remount, SPEC_CROSS_WINDOW_TAB_REMOUNT §4.3): reordering a single
    // tab is meaningless, and tearing it off would just leave an empty
    // window behind — but dragging it onto ANOTHER window's strip is how
    // a torn-off standalone window gets remounted. Allowed for every
    // window except main (whose last tab must never leave, see spec).
    const isLoneTabDrag = () => props.allTabCount === 1;

    // Gap before (left padding) — this tab is the afterTabId of the insertion point
    const gapBefore = createMemo(() => {
        const ip = insertionPoint();
        return ip?.afterTabId === props.tabId ? GAP_PX : 0;
    });

    // Gap after (right padding) — this tab is the beforeTabId of the insertion point
    const gapAfter = createMemo(() => {
        const ip = insertionPoint();
        return ip?.beforeTabId === props.tabId ? GAP_PX : 0;
    });

    const isBouncing = () => bouncingTabId() === props.tabId;
    const isTileDropHover = () => hoveredDropTabId() === props.tabId;

    // SPEC_NATIVE_POINTER_DRAG_TEAROFF_2026_07_28 — Windows-only pointer-
    // capture drag tracker. Tracks the torn-off window across this gesture
    // (created by onTearOffStart, cleared at onTearOffEnd) so a mid-drag
    // abort (Escape / pointercancel) can cancel-back: close the window and
    // restore the tab to its original position. macOS/Linux never enter
    // this — they keep the pragmatic-dnd draggable() path below untouched.
    let activeTearOff: {
        label: string;
        newWsId: string;
        originalTabIndex: number;
        wasPinned: boolean;
        sourceWorkspaceId: string;
    } | null = null;

    const canDrag = () => props.allTabCount > 1 || windowLabel !== "main";

    const nativeDragHandlers = {
        onDragStart: (cursorX: number, cursorY: number) => {
            const tabRect = tabWrapRef.getBoundingClientRect();
            setTabGrabOffset({ x: cursorX - tabRect.left, y: cursorY - tabRect.top });
            setGlobalDragTabId(props.tabId);
            setDragEscaped(false);
            setInsertionPoint(null);
            setIsDragging(true);
            // Lone-tab drags (isTearOffZone always declines them — see
            // below) never reach onTearOffStart, so THIS window/workspace
            // is arming the hook is the only chance to enable their
            // cross-window remount — and it stays correct for the whole
            // gesture since a lone-tab drag never moves the tab anywhere
            // itself (dragging the whole window onto another one's strip
            // is the entire gesture). Regular (multi-tab) drags do NOT arm
            // here: they might cross into tear-off, which moves the tab to
            // a new workspace — see onTearOffStart, which arms fresh with
            // that new (correct) context instead of this soon-to-be-stale one.
            if (isLoneTabDrag()) {
                fireAndForget(async () => {
                    try {
                        await getApi().startTabDragTracking({
                            sourceWindowLabel: windowLabel,
                            tabId: props.tabId,
                            sourceWsId: props.workspaceId,
                            isLastTab: true,
                        });
                    } catch (e) {
                        Logger.warn("dnd", "startTabDragTracking failed", { error: String(e) });
                    }
                });
            }
            Logger.info("dnd", "tab-drag started (native)", {
                tabId: props.tabId,
                workspaceId: props.workspaceId,
                tabIndex: props.tabIndex,
            });
        },
        onClick: () => {
            // No-op: a plain click (no movement past threshold) never calls
            // preventDefault/setPointerCapture, so the native click event
            // still bubbles to Tab's own onClick={props.onSelect} normally.
        },
        onReorderUpdate: (cursorX: number) => {
            setInsertionPoint(computeInsertionPoint(cursorX));
        },
        onReorderCommit: (_cursorX: number, _cursorY: number) => {
            const ip = insertionPoint();
            if (ip) {
                executeReorder(ip, props.tabId, props.tabIds, props.workspaceId);
                setBouncingTabId(props.tabId);
                setTimeout(() => setBouncingTabId(null), 400);
                Logger.info("dnd", "tab drop (native)", {
                    draggedTabId: props.tabId,
                    beforeTabId: ip.beforeTabId,
                    afterTabId: ip.afterTabId,
                    workspaceId: props.workspaceId,
                });
            }
            setInsertionPoint(null);
            setGlobalDragTabId(null);
            setIsDragging(false);
            fireAndForget(() => getApi().stopTabDragTracking());
            getApi().setJsDragActive(false).catch(() => {});
        },
        onReorderCancel: () => {
            setInsertionPoint(null);
            setGlobalDragTabId(null);
            setIsDragging(false);
            fireAndForget(() => getApi().stopTabDragTracking());
            getApi().setJsDragActive(false).catch(() => {});
        },
        isTearOffZone: (cursorX: number, cursorY: number) => {
            // Lone-tab drags can only ever be a cross-window remount — see
            // the note in onTearOffStart below for how that path is armed
            // now. Tearing the only tab off within its own window would
            // just trade one standalone window for another, so never treat
            // it as a tear-off candidate regardless of cursor position.
            if (props.allTabCount <= 1) return false;
            const rect = props.tabBarScrollRef()?.getBoundingClientRect();
            if (!rect) return false;
            return cursorY > rect.bottom + TEAR_PAST_PX;
        },
        onTearOffStart: async (screenX: number, screenY: number) => {
            const result = await props.requestNativeTearOff(props.tabId, screenX, screenY);
            if (!result) return undefined;
            activeTearOff = {
                label: result.label,
                newWsId: result.newWsId,
                originalTabIndex: result.originalTabIndex,
                wasPinned: result.wasPinned,
                sourceWorkspaceId: result.sourceWorkspaceId,
            };
            fireAndForget(() =>
                getApi().engageNativeWindowDrag(
                    result.label,
                    screenX - result.anchorX,
                    screenY - result.anchorY,
                ),
            );
            // Win32 quirk, not an OLE one this time: WM_SETCURSOR is only
            // delivered to whichever window currently holds mouse capture
            // (CEF's setPointerCapture implementation on Windows is backed
            // by native SetCapture) — every OTHER window, including the
            // desktop, is locked out of changing the cursor for the
            // gesture's duration and just keeps showing whatever was last
            // set. If Chromium's own captured-pointer-outside-viewport
            // cursor state lands on "not-allowed", nothing will move it off
            // that until something explicitly calls SetCursor again — so we
            // do, once, right as capture engages. Unlike the old OLE-era
            // fix, there's no IDropSource::GiveFeedback loop racing this
            // anymore (no OLE session exists at all under this model), so
            // a single call should stick for the whole drag.
            fireAndForget(() => getApi().setDragCursor());
            // Cross-window tab remount (SPEC_CROSS_WINDOW_TAB_REMOUNT §4.1),
            // reused per SPEC_NATIVE_POINTER_DRAG_TEAROFF_2026_07_28 §3.4:
            // arm the host's global mouse hook NOW, with the torn-off
            // window/workspace as the "source" — requestTearOff already
            // moved the tab there, so this is the CURRENT truth, unlike
            // arming at drag-start with what's about to become stale
            // context. A tear-off workspace always holds exactly one tab
            // (see requestTearOff's callers), so isLastTab is always true
            // here. Release over another AgentMux window's strip emits
            // tabdrag:merge-direct to it; release over nothing recognized
            // just leaves the live-followed window where it landed.
            fireAndForget(async () => {
                try {
                    await getApi().startTabDragTracking({
                        sourceWindowLabel: result.label,
                        tabId: props.tabId,
                        sourceWsId: result.newWsId,
                        isLastTab: true,
                    });
                } catch (e) {
                    Logger.warn("dnd", "startTabDragTracking (post-tearoff) failed", { error: String(e) });
                }
            });
            return result.label;
        },
        onTearOffMove: (screenX: number, screenY: number) => {
            fireAndForget(() => getApi().updateNativeWindowDrag(screenX, screenY));
        },
        onTearOffEnd: (committed: boolean) => {
            fireAndForget(() => getApi().endNativeWindowDrag());
            fireAndForget(() => getApi().stopTabDragTracking());
            fireAndForget(() => getApi().restoreDragCursor());
            setGlobalDragTabId(null);
            setIsDragging(false);
            getApi().setJsDragActive(false).catch(() => {});
            const torn = activeTearOff;
            activeTearOff = null;
            if (!committed && torn) {
                // Escape / pointercancel while a torn-off window was already
                // live-following the cursor: cancel-back — close it and
                // restore the tab to its original position. (The old SC_MOVE-
                // era tearoff:cancel-back host event this mirrors is dead —
                // requestTearOff's skipScMove has been hardcoded true for
                // months — so this is done directly, client-side, with data
                // already on hand from onTearOffStart's result.)
                fireAndForget(async () => {
                    try {
                        await WorkspaceService.RestoreTornOffTab(
                            props.tabId,
                            torn.newWsId,
                            torn.sourceWorkspaceId,
                            torn.originalTabIndex,
                            torn.wasPinned,
                        );
                        await getApi().closeWindowByLabel(torn.label);
                        Logger.info("dnd", "native tear-off cancel-back complete", { tabId: props.tabId });
                    } catch (e) {
                        Logger.error("dnd", "native tear-off cancel-back failed", {
                            tabId: props.tabId,
                            error: String(e),
                        });
                    }
                });
            }
        },
    };

    onMount(() => {
        if (!tabWrapRef) return;

        tabWrapperRefs.set(props.tabId, tabWrapRef);

        // SPEC_NATIVE_POINTER_DRAG_TEAROFF_2026_07_28 — Windows drives this
        // gesture entirely through native pointer capture (no OLE drag
        // session, so no circle-slash cursor to fight). macOS/Linux keep
        // the original pragmatic-dnd draggable() below, unchanged.
        const cleanupDraggable = isWindows()
            ? attachNativePointerDragTracker(tabWrapRef, nativeDragHandlers, canDrag)
            : draggable({
                element: tabWrapRef,
                canDrag,
                getInitialData: () => ({
                    tabId: props.tabId,
                    workspaceId: props.workspaceId,
                    tabIndex: props.tabIndex,
                    type: tabItemType,
                }),
                onGenerateDragPreview: ({ location, source }) => {
                    // Capture grab offset here (rather than onDragStart)
                    // because pragmatic-dnd's DragLocation is on this event;
                    // onDragStart only carries `source`. Used by tear-off to
                    // anchor the new window so the cursor stays on the same
                    // pixel of the same tab across the handoff.
                    //
                    // Note: we DO NOT suppress the OS drag image — the ghost
                    // following the cursor is the only drag feedback (SC_MOVE
                    // doesn't engage during the HTML5 drag; the new window
                    // appears on mouseup). We keep the ghost AND stop the
                    // "drop rejected" snapback via `preventUnhandled` in
                    // onDragStart/onDrop (macOS/Linux). An earlier note here
                    // argued against suppressing the ghost because it left a
                    // no-drop cursor with zero feedback — that's moot now:
                    // PR #1175 fixed the cursor (→ "move"), and this path
                    // keeps the ghost and only removes the snapback.
                    const tabRect = tabWrapRef.getBoundingClientRect();
                    setTabGrabOffset({
                        x: location.current.input.clientX - tabRect.left,
                        y: location.current.input.clientY - tabRect.top,
                    });
                    Logger.info("dnd", "tab-drag preview generated", {
                        tabId: source.data.tabId,
                        grabX: location.current.input.clientX - tabRect.left,
                        grabY: location.current.input.clientY - tabRect.top,
                    });
                },
                onDragStart: () => {
                    // Suppress the WebKit/WebKitGTK "drop rejected" snapback on
                    // tab tear-off (macOS/Linux): the tab is released outside any
                    // pragmatic-dnd drop target (the new window is created on
                    // dragend), so the browser would otherwise animate the drag
                    // ghost back into the source window. preventUnhandled makes the
                    // drop "handled" so the ghost just vanishes on release. Windows
                    // never reaches this branch anymore (native tracker above).
                    preventUnhandled.start();
                    setGlobalDragTabId(props.tabId);
                    setDragEscaped(false);
                    setInsertionPoint(null);
                    setIsDragging(true);
                    // Lone-tab drags carry NO cross-window payload: the HTML5
                    // pipeline's outcomes for a tab (tear-off to a new window,
                    // append-merge via DragOverlay) are all wrong for a
                    // single-tab window — tear-off would strand an empty
                    // window. The host mouse hook below is the only consumer;
                    // release anywhere but another window's strip is a no-op.
                    if (!isLoneTabDrag()) {
                        setCurrentDragPayload({ kind: "tab", tabId: props.tabId, workspaceId: props.workspaceId });
                    }
                    getApi().setJsDragActive(true).catch(() => {});
                    // Cross-window tab remount (SPEC_CROSS_WINDOW_TAB_REMOUNT
                    // §4.1): arm the host's global mouse hook for this drag so
                    // a release over another AgentMux window's strip emits
                    // tabdrag:merge-direct to it. No-op off-Windows.
                    fireAndForget(async () => {
                        try {
                            await getApi().startTabDragTracking({
                                sourceWindowLabel: windowLabel,
                                tabId: props.tabId,
                                sourceWsId: props.workspaceId,
                                isLastTab: props.allTabCount === 1,
                            });
                        } catch (e) {
                            Logger.warn("dnd", "startTabDragTracking failed", { error: String(e) });
                        }
                    });
                    Logger.info("dnd", "tab-drag started", {
                        tabId: props.tabId,
                        workspaceId: props.workspaceId,
                        tabIndex: props.tabIndex,
                    });
                },
                onDrop: () => {
                    preventUnhandled.stop();
                    setGlobalDragTabId(null);
                    setIsDragging(false);
                    // Belt-and-suspenders hook teardown. Ordinarily the hook
                    // self-uninstalled on the WM_LBUTTONUP that produced this
                    // dragend (LL hooks run before the event reaches the app),
                    // so this is a no-op; it matters for swallowed-dragend
                    // paths. Harmless for a superseding tear-off session too —
                    // that session's own WM_LBUTTONUP already retired it by
                    // the time any dragend fires.
                    fireAndForget(() => getApi().stopTabDragTracking());
                    // Do NOT clear setTabGrabOffset here. pragmatic-dnd's
                    // onDrop fires during the dragend event dispatch, BEFORE
                    // CrossWindowDragMonitor.win32.tsx::handleDragEnd runs.
                    // Clearing here makes the cross-window tear-off path
                    // read null for the grab offset (which it needs to
                    // compute the tab anchor) — host then falls back to
                    // "cursor at caption top-center" placement. The next
                    // drag's onGenerateDragPreview overwrites the offset,
                    // so leaving it stale across the no-drag interval is
                    // safe.
                    getApi().setJsDragActive(false).catch(() => {});
                    // Do NOT clear currentDragPayload here — this fires for ALL drops including
                    // out-of-window. Payload is cleared in the monitorForElements onDrop in
                    // tabbar.tsx (only fires for valid in-window drops) so the CrossWindowDragMonitor
                    // can still read it when dragend fires for out-of-window drops.
                },
            });

        // Pane (tile) drags over this tab's button — the spring-loaded-tabs
        // flow (SPEC_PANE_DRAG_TO_TAB_2026_07_10.md): a real drop target
        // (so the browser shows a move cursor instead of not-allowed), an
        // immediate blink (.tile-drop-hover pulse via hoveredDropTabId),
        // and after SPRING_SWITCH_MS of dwell, switch the UI to this tab so
        // the user can place the pane in its REAL layout (TileLayout's
        // overlay handles the ghost + drop from there).
        let springTimer: ReturnType<typeof setTimeout> | null = null;
        const clearSpring = () => {
            if (springTimer != null) {
                clearTimeout(springTimer);
                springTimer = null;
            }
            if (hoveredDropTabId() === props.tabId) setHoveredDropTabId(null);
        };
        const cleanupTileDropTarget = dropTargetForElements({
            element: tabWrapRef,
            canDrop: ({ source }) => source.data.type === tileItemType,
            onDragEnter: () => {
                // Hovering the ACTIVE tab's own button needs no switch (its
                // layout is already visible below) — no blink, no timer.
                if (props.tabId === props.activeTabId) return;
                setHoveredDropTabId(props.tabId);
                springTimer = setTimeout(() => {
                    springTimer = null;
                    setHoveredDropTabId(null);
                    Logger.info("dnd", "spring-switch to tab mid-drag", { tabId: props.tabId });
                    // Activate the target overlay BEFORE the switch so the
                    // very first dragover after the tab becomes visible
                    // already hits TileLayout's drop targets.
                    const model = getLayoutModelForTabById(props.tabId);
                    model?.activeDrag._set(true);
                    dragActivatedTabIds.add(props.tabId);
                    props.onSelect();
                    // The tab was display:none until the switch — its
                    // layout rects (additionalProps/overlay transforms)
                    // were computed against a zero-size container, so the
                    // first hover/drop hit-tests would use garbage
                    // geometry. Force a tree re-measure once the tab has
                    // real bounds (double rAF: display flip applies on the
                    // next frame; measure the one after).
                    requestAnimationFrame(() => {
                        requestAnimationFrame(() => {
                            fireAndForget(async () => model?.onTreeStateAtomUpdated(true));
                        });
                    });
                }, SPRING_SWITCH_MS);
            },
            onDragLeave: clearSpring,
            onDrop: () => {
                clearSpring();
                // Drop directly on the tab button (without waiting for the
                // spring switch): append the pane to that tab. Payload is
                // read BEFORE clearing; the clear stops CrossWindowDragMonitor
                // misreading the release as a tear-off.
                const payload = getCurrentDragPayload();
                setCurrentDragPayload(null);
                if (
                    payload?.kind === "tile" &&
                    payload.sourceTabId &&
                    payload.sourceTabId !== props.tabId &&
                    payload.node.data?.blockId
                ) {
                    redockDraggedPane({
                        blockId: payload.node.data.blockId,
                        sourceTabId: payload.sourceTabId,
                        targetTabId: props.tabId,
                    });
                }
            },
        });

        onCleanup(() => {
            tabWrapperRefs.delete(props.tabId);
            cleanupDraggable();
            cleanupTileDropTarget();
            clearSpring();
        });
    });

    return (
        <div
            ref={tabWrapRef!}
            data-drag-region="false"
            class={clsx("tab-drop-wrapper", {
                "tab-dragging": isDragging(),
                "tab-bouncing": isBouncing(),
                "tile-drop-hover": isTileDropHover(),
            })}
            style={{
                "padding-left": `${gapBefore()}px`,
                "padding-right": `${gapAfter()}px`,
                ...(naturalWidth() != null
                    ? { "--tab-natural-width": `${naturalWidth()}px` }
                    : {}),
            } as JSX.CSSProperties}
        >
            <Tab
                id={props.tabId}
                active={props.isActive}
                isFirst={props.isFirst}
                isBeforeActive={props.isBeforeActive}
                isDragging={isDragging()}
                tabWidth={0}
                isNew={false}
                onSelect={props.onSelect}
                onClose={props.onClose}
                onDragStart={() => {}}
                onLoaded={() => {}}
                onNaturalWidth={setNaturalWidth}
            />
        </div>
    );
}
