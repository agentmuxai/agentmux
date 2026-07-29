// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Tear-off RPC orchestration, split out of tabbar.tsx. This is mostly pure
// async logic (host RPC calls + workspace-service calls) that barely
// touches Solid signals — see requestTearOff and tearOffTabAtRelease below.

import { getApi } from "@/store/global";
import { fireAndForget } from "@/util/util";
import { getTabGrabOffset } from "./tab-grab-offset";
import { WorkspaceService } from "../store/services";
import { setCurrentDragPayload } from "@/app/drag/CrossWindowDragMonitor";
import { Logger } from "@/util/logger";

/**
 * Phase 2 — orchestrates the Chrome-faithful tear-off when the cursor
 * crosses the strip's bottom edge. Three steps, all from the source
 * window's renderer:
 *
 *   1. Move the tab data to a brand-new workspace (sidecar).
 *   2. Spawn a new agentmux-cef window pointed at that workspace
 *      (host).
 *   3. Hand cursor capture over to the new window via Win32 SC_MOVE
 *      (host) so it follows the mouse like a Chrome torn-off tab.
 *
 * Phase 2 acceptance is structural: the SC_MOVE plumbing fires and
 * the window follows the cursor. The cold-path first-paint flash
 * (~150-300ms while the new window registers + paints) is expected
 * and is not an acceptance failure — Phase 6's pre-warmed pool
 * brings it to 0 ms. The ≤ 8 ms handshake budget from the spec §0
 * is measured by the host and emitted as `handshakeMs` on this
 * call's result.
 *
 * Subsequent phases:
 *   - Phase 3: capture a TabSnapshot for width preservation
 *   - Phase 4: WH_MOUSE_LL hook for cross-window merge detection
 *   - Phase 5: cancel-back-to-source on drop over origin strip
 *   - Phase 6: pre-warmed window pool (eliminates first-paint flash)
 *
 * See docs/specs/SPEC_TAB_TEAR_OFF_SIZE_PRESERVATION_2026_04_26.
 */
export async function requestTearOff(
    tabId: string,
    workspaceId: string,
    cursorX: number,
    cursorY: number,
    originalTabIndex: number,
    wasPinned: boolean,
    tabAnchorX?: number,
    tabAnchorY?: number,
    // Commit-on-release path (down-drag tear-off fired from onDrop): the
    // mouse is already up, so there is nothing to follow — skip Step 3's
    // Win32 SC_MOVE modal move-loop. The window is created + shown at the
    // anchor by pool-promote / openWindowAtPosition alone (Step 2), which
    // is exactly what CrossWindowDragMonitor's release-time performTearOff
    // relies on. When false (legacy mid-drag), SC_MOVE runs as before.
    skipScMove: boolean = false,
    // Returns the destination window's label and its (newly-created) workspace
    // id on success, or undefined on failure — SPEC_NATIVE_POINTER_DRAG_
    // TEAROFF_2026_07_28's live-follow tracker needs both: the label to call
    // engageNativeWindowDrag with, the workspace id to cancel-back into if the
    // gesture is later aborted mid-drag. The original release-time caller
    // (createTearOffTabAtRelease below) ignores the return value.
): Promise<{ label: string; newWsId: string } | undefined> {
    const t0 = performance.now();
    // F1.B — orphan-cleanup state. We only restore the tab when we
    // can PROVE the destination window doesn't exist: the
    // create-window APIs (pool-promote and openWindowAtPosition) post
    // window creation asynchronously, so neither a successful return
    // nor a handshake error proves the window won't materialize.
    //
    // The single safe signal: `openWindowAtPosition` itself threw.
    // That means the host couldn't even post the create command —
    // no window will ever materialize for this tear-off. (Pool-
    // promote being exhausted is also a host-side rejection, but
    // we then attempt cold-path; only when cold-path ALSO throws do
    // we know no create was posted.)
    //
    // Anything else (pool-promote succeeded; cold-path returned a
    // label and the create posted but never registered; handshake
    // failed for any reason post-create) leaves the window in an
    // unknown state. Conservatively skip restore in those cases —
    // the orphan workspace is a smaller harm than the risk of
    // cascade-deleting a workspace that a delayed window is about
    // to register against. (codex P1 round-3 #624.)
    let newWsId: string | undefined;
    let coldPathFailed = false;
    // Capture source window's outer dimensions so the tear-off result
    // matches the user's current frame instead of the hardcoded pool
    // default (1200×800). UX expectation: dragging a tab out gives you
    // a window the same size as the one you dragged from.
    const sourceWidth = window.outerWidth;
    const sourceHeight = window.outerHeight;
    try {
        const sourceWindowLabel = await getApi().getWindowLabel();
        // Step 1 — sidecar transfers the tab into a new workspace.
        // Returns the new workspace's ID.
        newWsId = await WorkspaceService.TearOffTab(tabId, workspaceId);
        // Step 2 — get the destination window. Phase 6 prefers the
        // pre-warmed pool (0 ms first-paint flash). On pool exhaustion
        // we fall back to the cold-path openWindowAtPosition (~150-300 ms
        // flash). Per spec §0 this fallback should never fire in
        // practice; if it does we'll see WARN logs and tear_off.pool_
        // exhausted increments and can investigate the underlying race.
        let destWindowLabel: string;
        try {
            destWindowLabel = await getApi().tearOffPoolPromote(
                newWsId,
                cursorX,
                cursorY,
                sourceWidth,
                sourceHeight,
                tabAnchorX,
                tabAnchorY,
            );
            Logger.info("dnd", "tear-off used warm pool", { destWindowLabel });
        } catch (poolErr) {
            Logger.warn("dnd", "tear-off pool exhausted, falling back to cold path", {
                error: String(poolErr),
            });
            try {
                destWindowLabel = await getApi().openWindowAtPosition(
                    cursorX,
                    cursorY,
                    newWsId,
                    sourceWidth,
                    sourceHeight,
                    tabAnchorX,
                    tabAnchorY,
                );
            } catch (coldErr) {
                // F1.B safe-restore signal: cold-path API itself
                // threw. The host couldn't post the create command,
                // so no window will materialize. Re-throw to outer
                // catch which will dispatch RestoreTornOffTab.
                coldPathFailed = true;
                throw coldErr;
            }
        }
        if (skipScMove) {
            // Commit-on-release: the window was already created + shown at
            // the anchor by Step 2 (pool-promote / cold path). The mouse is
            // up, so there is no drag to follow — skip the SC_MOVE
            // move-loop. Clear the cross-window drag payload so the legacy
            // dragend pipeline (CrossWindowDragMonitor) doesn't
            // double-process this gesture when its dragend fires.
            setCurrentDragPayload(null);
            Logger.info("dnd", "tab tear-off complete (commit-on-release)", {
                tabId,
                destWindowLabel,
                totalMs: performance.now() - t0,
            });
            return { label: destWindowLabel, newWsId };
        } else {
            // Step 3 — Win32 SC_MOVE handshake. Host waits for the new
            // window's HWND to register, then transfers cursor capture
            // and posts WM_SYSCOMMAND/SC_MOVE so Windows enters its
            // built-in modal move-loop. Until mouseup, the new window
            // follows the cursor at full opacity, no ghost.
            const result = await getApi().tearOffSCMoveHandshake({
                sourceWindowLabel,
                destWindowLabel,
                cursorX,
                cursorY,
                // Phase 4 — fields the host hook needs to drive the merge
                // event on mouseup. Without these the hook is skipped and
                // the dragged window simply ends as a standalone.
                tabId,
                sourceWsId: workspaceId,
                destWsId: newWsId,
                // Phase 5 — original tab index so ESC / drop-on-source
                // can restore at the right position rather than the end.
                // wasPinned controls which list the index points into
                // (pinnedtabids vs tabids).
                originalTabIndex,
                wasPinned,
            });
            // Handshake confirmed the destination window's HWND
            // registered + Windows now owns the move loop.
            // Clear the cross-window drag payload so the legacy dragend
            // pipeline (CrossWindowDragMonitor) doesn't double-process
            // this gesture when its dragend fires. Cleared HERE rather
            // than in onDrag so a failure mid-pipeline (TearOffTab,
            // openWindowAtPosition, or the handshake itself) leaves the
            // legacy fallback intact.
            setCurrentDragPayload(null);
            Logger.info("dnd", "tab tear-off complete", {
                tabId,
                destWindowLabel,
                handshakeMs: result.handshakeMs,
                totalMs: performance.now() - t0,
            });
            return { label: destWindowLabel, newWsId };
        }
    } catch (e) {
        Logger.error("dnd", "tab tear-off failed", { tabId, error: String(e) });
        // F1.B — orphan workspace cleanup. Only restore the tab when
        // we're PROVABLY safe: cold-path window-create threw (no
        // window will materialize). Any other failure path leaves
        // the destination window in an unknown state and we
        // conservatively keep the orphan workspace rather than risk
        // cascade-deleting a workspace a delayed window is about
        // to register against. (codex P1 round-3 #624.)
        if (newWsId && coldPathFailed) {
            try {
                await WorkspaceService.RestoreTornOffTab(
                    tabId,
                    newWsId,
                    workspaceId,
                    originalTabIndex,
                    wasPinned,
                );
                Logger.info("dnd", "tab tear-off restored after window-create failure", {
                    tabId,
                    newWsId,
                });
            } catch (restoreErr) {
                Logger.error("dnd", "tab tear-off restore also failed — orphan workspace persists", {
                    tabId,
                    newWsId,
                    error: String(restoreErr),
                });
            }
        } else if (newWsId) {
            // TearOffTab succeeded but we hit a failure mode where
            // we can't safely restore (handshake error, post-window-
            // create timing, etc.). Leave the orphan workspace; the
            // user will see it in the workspace list and can close
            // it via the UI.
            Logger.warn("dnd", "tab tear-off failed post-create — orphan workspace left for user cleanup", {
                tabId,
                newWsId,
            });
        }
    }
}

export type TearOffTabAtReleaseFn = (
    draggedTabId: string,
    input: { clientX: number; clientY: number },
) => void;

/**
 * Builds the commit-on-release tear-off handler. Fired from the drag
 * monitor's onDrop (see useTabDragAndDrop in tab-reorder.ts) when the tab
 * is released below the strip. Computes the same tear-off anchor the old
 * mid-drag path used (so the new window's first tab lands under the
 * cursor), then calls requestTearOff with skipScMove=true — the mouse is
 * already up, so the window is simply placed at the release point instead
 * of entering Windows' SC_MOVE modal move-loop.
 *
 * `workspace` and `tabBarScrollRef` are accessors rather than plain values
 * so this can be built once (e.g. in TabBar's setup) and still observe the
 * DOM ref / prop as of whenever the returned function actually runs.
 */
export function createTearOffTabAtRelease(
    workspace: () => Workspace,
    tabBarScrollRef: () => HTMLDivElement,
): TearOffTabAtReleaseFn {
    return (draggedTabId, input) => {
        const ws = workspace();
        const wsId = ws?.oid;
        if (!wsId) return;
        // Which list the tab lived in + its index there, so a cancel-back
        // restores it to the right place (backend restores into pinnedtabids
        // when wasPinned, else tabids — the lists persist separately).
        const pinnedIds = ws?.pinnedtabids ?? [];
        const tabIdsRaw = ws?.tabids ?? [];
        const pinnedIdx = pinnedIds.indexOf(draggedTabId);
        const wasPinned = pinnedIdx >= 0;
        const originalTabIndex = wasPinned
            ? pinnedIdx
            : Math.max(0, tabIdsRaw.indexOf(draggedTabId));

        // Window-create APIs expect SCREEN coordinates; input.clientX/Y are
        // viewport-relative. Convert via window.screenX/Y (both DIP — matches
        // the DIP grab offset below, so no devicePixelRatio scaling needed).
        const screenX = window.screenX + input.clientX;
        const screenY = window.screenY + input.clientY;
        // Tab anchor: place the new window's outer top-left so its first tab
        // lands at the same screen pixel as the grabbed source tab (identical
        // chrome + CSS ⇒ identical first-tab rect). See the removed mid-drag
        // path for the full derivation.
        const grabOffset = getTabGrabOffset();
        const chromeBorderX = Math.max(0, window.outerWidth - window.innerWidth) / 2;
        const chromeBorderY = Math.max(
            0,
            window.outerHeight - window.innerHeight - chromeBorderX,
        );
        // First TAB, not firstElementChild — the leading .tab-separator is a
        // centered sliver whose rect would skew the anchor.
        const firstTabEl = tabBarScrollRef()?.querySelector(
            ".tab-drop-wrapper",
        ) as HTMLElement | null;
        const firstTabRect = firstTabEl?.getBoundingClientRect();
        const tabAnchorX =
            grabOffset && firstTabRect
                ? Math.round(screenX - grabOffset.x - firstTabRect.left - chromeBorderX)
                : undefined;
        const tabAnchorY =
            grabOffset && firstTabRect
                ? Math.round(screenY - grabOffset.y - firstTabRect.top - chromeBorderY)
                : undefined;
        Logger.info("dnd", "tab tear-off on release", {
            draggedTabId, screenX, screenY, tabAnchorX, tabAnchorY,
        });
        fireAndForget(() =>
            requestTearOff(
                draggedTabId,
                wsId,
                screenX,
                screenY,
                originalTabIndex,
                wasPinned,
                tabAnchorX,
                tabAnchorY,
                true, // skipScMove — commit-on-release, no move-loop
            ),
        );
    };
}

export type NativeTearOffResult = {
    label: string;
    newWsId: string;
    /** Screen-px outer top-left the destination window was created at —
     *  the point that lands under the cursor's grabbed pixel. Used by the
     *  caller (droppable-tab.tsx) to derive engageNativeWindowDrag's
     *  grab offset without a redundant HWND/rect query. */
    anchorX: number;
    anchorY: number;
    originalTabIndex: number;
    wasPinned: boolean;
    sourceWorkspaceId: string;
};

export type NativeTearOffFn = (
    draggedTabId: string,
    screenX: number,
    screenY: number,
) => Promise<NativeTearOffResult | undefined>;

/**
 * SPEC_NATIVE_POINTER_DRAG_TEAROFF_2026_07_28 — builds the tear-off handler
 * for the Windows native-pointer-drag tracker's `onTearOffStart`. Same
 * anchor derivation and `requestTearOff(..., skipScMove=true)` call as
 * `createTearOffTabAtRelease` above, with two differences: (1) `screenX/Y`
 * come straight from the triggering PointerEvent — already real screen
 * coordinates thanks to setPointerCapture, so no `window.screenX + clientX`
 * conversion is needed (and wouldn't even be meaningful once the drag has
 * left the window); (2) this fires at threshold-cross, not release, and
 * returns the created window's label + anchor so the caller can hand off to
 * `engageNativeWindowDrag` for live cursor-follow.
 */
export function createNativeTearOffHandler(
    workspace: () => Workspace,
    tabBarScrollRef: () => HTMLDivElement,
): NativeTearOffFn {
    return async (draggedTabId, screenX, screenY) => {
        const ws = workspace();
        const wsId = ws?.oid;
        if (!wsId) return undefined;
        const pinnedIds = ws?.pinnedtabids ?? [];
        const tabIdsRaw = ws?.tabids ?? [];
        const pinnedIdx = pinnedIds.indexOf(draggedTabId);
        const wasPinned = pinnedIdx >= 0;
        const originalTabIndex = wasPinned
            ? pinnedIdx
            : Math.max(0, tabIdsRaw.indexOf(draggedTabId));

        const grabOffset = getTabGrabOffset();
        const chromeBorderX = Math.max(0, window.outerWidth - window.innerWidth) / 2;
        const chromeBorderY = Math.max(
            0,
            window.outerHeight - window.innerHeight - chromeBorderX,
        );
        const firstTabEl = tabBarScrollRef()?.querySelector(
            ".tab-drop-wrapper",
        ) as HTMLElement | null;
        const firstTabRect = firstTabEl?.getBoundingClientRect();
        // Falls back to the same half-width/16px-above formula the host's
        // own open_window_at_position uses when no anchor is supplied
        // (agentmux-cef/src/commands/drag.rs) — grabOffset/firstTabRect
        // should always be present in practice (grabOffset is set at every
        // pointerdown by the tracker's caller before a drag can begin), so
        // this branch is defensive, not a normal path.
        const anchorX =
            grabOffset && firstTabRect
                ? Math.round(screenX - grabOffset.x - firstTabRect.left - chromeBorderX)
                : Math.round(Math.max(0, screenX - 600));
        const anchorY =
            grabOffset && firstTabRect
                ? Math.round(screenY - grabOffset.y - firstTabRect.top - chromeBorderY)
                : Math.round(Math.max(0, screenY - 16));

        Logger.info("dnd", "native tab tear-off engage", {
            draggedTabId, screenX, screenY, anchorX, anchorY,
        });

        const result = await requestTearOff(
            draggedTabId,
            wsId,
            screenX,
            screenY,
            originalTabIndex,
            wasPinned,
            anchorX,
            anchorY,
            true, // skipScMove — window created immediately, followed via native drag instead
        );
        if (!result) return undefined;
        return {
            label: result.label,
            newWsId: result.newWsId,
            anchorX,
            anchorY,
            originalTabIndex,
            wasPinned,
            sourceWorkspaceId: wsId,
        };
    };
}
