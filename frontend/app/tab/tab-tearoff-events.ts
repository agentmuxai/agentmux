// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Cross-window tear-off event-listener subsystem, split out of tabbar.tsx.
// Phase 4/5 host IPC listeners (tearoff:hover-changed/merge/standalone/
// cancel-back, tabdrag:merge-direct) that let a window react to a tab being
// torn off / merged / cancelled in ANOTHER window.

import { onCleanup, onMount } from "solid-js";
import { getApi } from "@/store/global";
import { fireAndForget } from "@/util/util";
import { isWindows } from "@/util/platformutil";
import { WorkspaceService } from "../store/services";
import {
    computeInsertionPoint,
    insertionPointToIndex,
    markTabMerged,
    setBouncingTabId,
    setInsertionPoint,
} from "./tabbar-dnd";
import { Logger } from "@/util/logger";

/**
 * Phase 4 — listen for the host's tear-off events. Each AgentMux
 * window subscribes; the host targets the right window via
 * emit_event_to_window so other windows see nothing.
 *
 *  tearoff:hover-changed — cursor entered this window's strip area
 *    while another window's tab is mid-tear-off. Show the standard
 *    insertion-point indicator so the user can see where the merge
 *    will land.
 *  tearoff:hover-cleared — cursor left this window's strip. Drop
 *    the indicator.
 *  tearoff:merge — mouseup over this window. Pull the dragged
 *    tab into our workspace at the cursor's X position, then close
 *    the (now empty) dragged window.
 *  tearoff:standalone — emitted to the source window when the user
 *    releases over no AgentMux window. Currently informational only;
 *    Phase 5 will use this to update cancel-back UI state.
 *
 * Must be called during SolidJS component setup (uses onMount/onCleanup
 * internally).
 */
export function useTabTearOffEvents(
    workspace: () => Workspace,
    tabBarScrollRef: () => HTMLDivElement,
    tabIds: () => string[],
): void {
    onMount(() => {
        // `mounted` flag protects against the race where the component
        // unmounts (e.g. tab change, HMR) while the dynamic import or
        // listenEvent calls are still in flight. Without this, listeners
        // registered after onCleanup ran would leak forever — they're
        // not in `unsubs` yet when onCleanup fires, so the cleanup pass
        // misses them. (gemini PR #565 HIGH)
        let mounted = true;
        let unsubs: Array<() => void> = [];
        const trackOrDispose = (unsub: () => void) => {
            if (mounted) {
                unsubs.push(unsub);
            } else {
                unsub();
            }
        };
        // Coordinate-space helper. On Windows, `payload.cursorX/Y` come
        // from Win32's WH_MOUSE_LL hook in PHYSICAL pixels (Windows
        // reports per-monitor coords for DPI-aware processes, which
        // CEF is). `window.screenX/Y` and `getBoundingClientRect()`
        // return CSS / LOGICAL pixels. Subtract directly and you're
        // off by a factor of devicePixelRatio at DPR ≠ 1.0 — the
        // strip hit-test would never trigger on HiDPI. Convert
        // physical → CSS by dividing by DPR before subtracting.
        // (gemini PR #567 HIGH; same fix applies to Phase 4 merge
        // handler below — both shared the bug.)
        // Math.max(1, ...) defends against the (rare but possible) browser
        // edge case where devicePixelRatio is 0 or negative; the falsy-||
        // already covered undefined/NaN. (gemini PR #567 round-8 MEDIUM)
        //
        // On macOS, `payload.cursorX/Y` come from CGEventTap's
        // CGEvent.location(), which reports coordinates in POINTS —
        // the same unit `window.screenX/Y` already uses (macOS has no
        // physical/logical pixel distinction at this level; Retina scale
        // is baked into rendering, never exposed here). Dividing those
        // by DPR would silently shrink them on any Retina display, so
        // the conversion only applies on Windows.
        // See SPEC_MACOS_TAB_REDOCK_PARITY_2026_07_24.md §3.
        const dpr = () => (isWindows() ? Math.max(1, window.devicePixelRatio || 1) : 1);
        const physicalToClientX = (px: number) => px / dpr() - window.screenX;
        const physicalToClientY = (py: number) => py / dpr() - window.screenY;
        fireAndForget(async () => {
            const { listenEvent } = await import("@/app/platform/ipc");
            if (!mounted) return;

            trackOrDispose(
                await listenEvent<{ cursorX: number; cursorY: number }>(
                    "tearoff:hover-changed",
                    (payload) => {
                        // Host emits hover-changed when cursor is over
                        // ANY part of this window's HWND; check Y against
                        // the strip rect so dropping on the content area
                        // doesn't trigger a merge. (codex PR #565 P1)
                        const stripRect = tabBarScrollRef()?.getBoundingClientRect();
                        if (!stripRect) {
                            setInsertionPoint(null);
                            return;
                        }
                        const clientX = physicalToClientX(payload.cursorX);
                        const clientY = physicalToClientY(payload.cursorY);
                        if (clientY < stripRect.top || clientY > stripRect.bottom) {
                            setInsertionPoint(null);
                            return;
                        }
                        setInsertionPoint(computeInsertionPoint(clientX));
                    },
                ),
            );

            trackOrDispose(
                await listenEvent("tearoff:hover-cleared", () => {
                    setInsertionPoint(null);
                }),
            );

            trackOrDispose(
                await listenEvent<{
                    tabId: string;
                    fromWsId: string;
                    draggedWindowLabel: string;
                    cursorX: number;
                    cursorY: number;
                }>("tearoff:merge", (payload) => {
                    setInsertionPoint(null);
                    fireAndForget(async () => {
                        try {
                            const ownWsId = workspace()?.oid;
                            if (!ownWsId) {
                                Logger.warn("dnd", "tearoff:merge — no own workspace, skipping", payload);
                                return;
                            }
                            // Strip-area hit test: only merge when the
                            // cursor is actually over the tab strip,
                            // not the content area below. Otherwise an
                            // accidental release while passing over a
                            // window's body would silently relocate
                            // the tab. (codex PR #565 P1)
                            const stripRect = tabBarScrollRef()?.getBoundingClientRect();
                            const clientX = physicalToClientX(payload.cursorX);
                            const clientY = physicalToClientY(payload.cursorY);
                            if (
                                !stripRect ||
                                clientY < stripRect.top ||
                                clientY > stripRect.bottom
                            ) {
                                Logger.info("dnd", "tearoff:merge — cursor not over strip, leaving as standalone", payload);
                                setInsertionPoint(null);
                                return;
                            }
                            const insertIdx = insertionPointToIndex(
                                computeInsertionPoint(clientX),
                                tabIds(),
                            );
                            // Tear-off workspaces always carry exactly one
                            // tab, so MoveTabToWorkspace's last-tab guard
                            // would reject this. RestoreTornOffTab bypasses
                            // that and deletes the now-empty source ws so
                            // closeWindowByLabel below doesn't cascade.
                            await WorkspaceService.RestoreTornOffTab(
                                payload.tabId,
                                payload.fromWsId,
                                ownWsId,
                                insertIdx,
                            );
                            await getApi().closeWindowByLabel(payload.draggedWindowLabel);
                            Logger.info("dnd", "tearoff:merge complete", {
                                tabId: payload.tabId,
                                fromWsId: payload.fromWsId,
                                ownWsId,
                                insertIdx,
                            });
                        } catch (e) {
                            Logger.error("dnd", "tearoff:merge failed", {
                                error: String(e),
                                payload,
                            });
                        }
                    });
                }),
            );

            trackOrDispose(
                await listenEvent("tearoff:standalone", (payload) => {
                    Logger.info("dnd", "tearoff:standalone", payload);
                }),
            );

            // Cross-window tab remount (SPEC_CROSS_WINDOW_TAB_REMOUNT §4.2):
            // a tab dragged directly from another window was released over
            // THIS window (host mouse hook, TabDrag mode). Unlike
            // tearoff:merge, the tab still lives in its original multi-tab
            // workspace — no temporary tear-off workspace exists — so the
            // multi-tab path uses MoveTabToWorkspace (its last-tab guard is
            // desirable) and only the last-tab path uses RestoreTornOffTab
            // (which bypasses the guard and deletes the emptied workspace).
            trackOrDispose(
                await listenEvent<{
                    tabId: string;
                    fromWsId: string;
                    sourceWindowLabel: string;
                    isLastTab: boolean;
                    cursorX: number;
                    cursorY: number;
                }>("tabdrag:merge-direct", (payload) => {
                    setInsertionPoint(null);
                    fireAndForget(async () => {
                        try {
                            const ownWsId = workspace()?.oid;
                            if (!ownWsId) {
                                Logger.warn("dnd", "tabdrag:merge-direct — no own workspace, skipping", payload);
                                return;
                            }
                            if (payload.fromWsId === ownWsId) {
                                // Shouldn't happen (hook excludes the source
                                // window), but a same-workspace "move" would
                                // reorder-to-end; bail defensively.
                                return;
                            }
                            // Strip-area hit test, same as tearoff:merge:
                            // releasing over this window's CONTENT area is
                            // not a header drop. The legacy cross-drag
                            // pipeline (DragOverlay) still handles that
                            // case as an append-merge, unchanged.
                            const stripRect = tabBarScrollRef()?.getBoundingClientRect();
                            const clientX = physicalToClientX(payload.cursorX);
                            const clientY = physicalToClientY(payload.cursorY);
                            if (
                                !stripRect ||
                                clientY < stripRect.top ||
                                clientY > stripRect.bottom
                            ) {
                                Logger.info("dnd", "tabdrag:merge-direct — cursor not over strip, ignoring", payload);
                                return;
                            }
                            const insertIdx = insertionPointToIndex(
                                computeInsertionPoint(clientX),
                                tabIds(),
                            );
                            // Never close the main window (SPEC §4.3): a
                            // 1-tab main's last tab stays put rather than
                            // feeding the last-window quit sequence.
                            if (payload.isLastTab && payload.sourceWindowLabel === "main") {
                                Logger.info(
                                    "dnd",
                                    "tabdrag:merge-direct — declining last tab of main window",
                                    payload,
                                );
                                return;
                            }
                            // Mark BEFORE the awaits — DragOverlay's
                            // cross-drag-end for the same gesture arrives
                            // while these RPCs are in flight, and the mark
                            // is what makes it a no-op.
                            markTabMerged(payload.tabId);
                            if (payload.isLastTab) {
                                await WorkspaceService.RestoreTornOffTab(
                                    payload.tabId,
                                    payload.fromWsId,
                                    ownWsId,
                                    insertIdx,
                                );
                                await getApi().closeWindowByLabel(payload.sourceWindowLabel);
                            } else {
                                await WorkspaceService.MoveTabToWorkspace(
                                    payload.tabId,
                                    payload.fromWsId,
                                    ownWsId,
                                    insertIdx,
                                );
                            }
                            setBouncingTabId(payload.tabId);
                            setTimeout(() => setBouncingTabId(null), 400);
                            Logger.info("dnd", "tabdrag:merge-direct complete", {
                                tabId: payload.tabId,
                                fromWsId: payload.fromWsId,
                                ownWsId,
                                insertIdx,
                                isLastTab: payload.isLastTab,
                            });
                        } catch (e) {
                            Logger.error("dnd", "tabdrag:merge-direct failed", {
                                error: String(e),
                                payload,
                            });
                        }
                    });
                }),
            );

            // Phase 5 — cancel-back. Source window receives this on
            // either ESC during the SC_MOVE loop or drop-on-source-
            // strip. Move the tab back from the dragged window's
            // workspace into ours at its original index, then close
            // the dragged window.
            trackOrDispose(
                await listenEvent<{
                    tabId: string;
                    fromWsId: string;
                    originalSourceWsId: string;
                    draggedWindowLabel: string;
                    originalIndex: number;
                    wasPinned: boolean;
                    cursorX?: number;
                    cursorY?: number;
                    reason: string;
                }>("tearoff:cancel-back", (payload) => {
                    fireAndForget(async () => {
                        try {
                            // Drop any stale insertion gap left over from
                            // tearoff:hover-changed updates while the cursor
                            // was still on the strip. (codex PR #567 P3)
                            setInsertionPoint(null);
                            // Restore into the workspace the tab was torn
                            // from, NOT this window's currently-active
                            // workspace. If the user switched workspaces
                            // mid-drag, ownWsId would put the tab in the
                            // wrong place. (codex PR #567 round-5 P2)
                            const restoreWsId = payload.originalSourceWsId;
                            if (!restoreWsId) {
                                Logger.warn("dnd", "tearoff:cancel-back — no original source workspace, skipping", payload);
                                return;
                            }
                            // Strip-area hit test (drop-on-source path
                            // only — ESC has no cursor coords). Mirrors
                            // the merge handler's check: the host emits
                            // cancel-back whenever the cursor's over
                            // any part of the source window's HWND, but
                            // we only restore if the cursor was
                            // actually on the tab strip. Otherwise fall
                            // through to standalone (do nothing — the
                            // dragged window stays where it landed).
                            if (payload.reason === "drop-on-source"
                                && payload.cursorX != null
                                && payload.cursorY != null
                            ) {
                                const stripRect = tabBarScrollRef()?.getBoundingClientRect();
                                const clientY = physicalToClientY(payload.cursorY);
                                if (
                                    !stripRect
                                    || clientY < stripRect.top
                                    || clientY > stripRect.bottom
                                ) {
                                    Logger.info("dnd", "tearoff:cancel-back — cursor over source body, leaving as standalone", payload);
                                    return;
                                }
                            }
                            // Tear-off workspace has exactly one tab —
                            // MoveTabToWorkspace would reject moving it
                            // out. RestoreTornOffTab bypasses the last-tab
                            // guard and deletes the empty source ws, so
                            // the dragged window's close cascade has
                            // nothing left to do. (codex PR #567 P1)
                            await WorkspaceService.RestoreTornOffTab(
                                payload.tabId,
                                payload.fromWsId,
                                restoreWsId,
                                payload.originalIndex,
                                payload.wasPinned,
                            );
                            await getApi().closeWindowByLabel(payload.draggedWindowLabel);
                            Logger.info("dnd", "tearoff:cancel-back complete", {
                                tabId: payload.tabId,
                                originalIndex: payload.originalIndex,
                                reason: payload.reason,
                            });
                        } catch (e) {
                            Logger.error("dnd", "tearoff:cancel-back failed", {
                                error: String(e),
                                payload,
                            });
                        }
                    });
                }),
            );
        });
        onCleanup(() => {
            mounted = false;
            for (const u of unsubs) u();
            unsubs = [];
        });
    });
}
