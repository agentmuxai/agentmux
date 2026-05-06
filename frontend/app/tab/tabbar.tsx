// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { atoms, createBlock, createTab, getApi, setActiveTab } from "@/store/global";
import { FlyoutMenu } from "@/app/element/flyoutmenu";
import { invokeCommand } from "@/app/platform/ipc";
import { fireAndForget } from "@/util/util";
import { useWindowDrag } from "@/app/hook/useWindowDrag.platform";
import { monitorForElements } from "@atlaskit/pragmatic-drag-and-drop/element/adapter";
import { createMemo, For, onCleanup, onMount, Show } from "solid-js";
import type { JSX } from "solid-js";
import { ObjectService, WorkspaceService } from "../store/services";
import { makeORef, getObjectValue } from "../store/wos";
import { deleteLayoutModelForTab } from "@/layout/index";
import { DroppableTab } from "./droppable-tab";
import {
    tabItemType,
    insertionPoint,
    setInsertionPoint,
    bouncingTabId,
    setBouncingTabId,
    computeInsertionPoint,
    InsertionPoint,
} from "./tabbar-dnd";
import { setCurrentDragPayload } from "@/app/drag/CrossWindowDragMonitor";
import { Logger } from "@/util/logger";
import "./tabbar.scss";

export { tabItemType } from "./tabbar-dnd";

interface TabBarProps {
    workspace: Workspace;
}


// Pixels past the tab strip's bottom edge before a drag becomes a
// tear-off (Chrome uses a similar small threshold). 24 px is enough
// to filter out brief excursions while the user is still hunting for
// the drop position; small enough that the tear feels intentional.
// See docs/specs/SPEC_TAB_TEAR_OFF_SIZE_PRESERVATION_2026_04_26 §4.1.
const TEAR_PAST_PX = 24;

function TabBar(props: TabBarProps): JSX.Element {
    const activeTabId = atoms.activeTabId;
    let tabBarScrollRef!: HTMLDivElement;
    // Latches once per drag — the tear-off handshake should only run a
    // single time even if the user keeps moving past the threshold.
    let tearOffFired = false;

    // Pin feature removed — merge any legacy pinnedtabids into the regular list
    // so existing workspaces don't lose tabs. A one-time UpdateTabIds (below)
    // drains pinnedtabids server-side so this concat becomes a no-op.
    const tabIds = () => {
        const ws = props.workspace;
        if (!ws) return [];
        return [...(ws.pinnedtabids ?? []), ...(ws.tabids ?? [])];
    };

    const handleSelect = (tabId: string) => {
        if (tabId !== activeTabId()) setActiveTab(tabId);
    };

    const handleClose = (tabId: string) => {
        const allTabs = tabIds();
        if (allTabs.length <= 1) return;
        fireAndForget(async () => {
            if (tabId === activeTabId()) {
                const idx = allTabs.indexOf(tabId);
                const nextTab = allTabs[idx + 1] ?? allTabs[idx - 1];
                if (nextTab) await setActiveTab(nextTab);
            }
            await WorkspaceService.CloseTab(props.workspace.oid, tabId);
            deleteLayoutModelForTab(tabId);
        });
    };

    const { dragProps } = useWindowDrag();

    // One-time migration: if this workspace still has pinned tabs from an older
    // build, fold them into tabids and clear pinnedtabids server-side.
    onMount(() => {
        const ws = props.workspace;
        if (ws && (ws.pinnedtabids?.length ?? 0) > 0) {
            const merged = [...(ws.pinnedtabids ?? []), ...(ws.tabids ?? [])];
            fireAndForget(async () => {
                try {
                    await WorkspaceService.UpdateTabIds(ws.oid, merged, []);
                } catch (e) {
                    console.error("[tabbar] pin migration failed:", e);
                }
            });
        }
    });

    // Startup-tab color: if the workspace has exactly one tab and it has no
    // tab:color set, apply the theme Blue from TAB_COLORS. The backend-created
    // startup tab doesn't get a color meta, so without this the first tab
    // stays neutral while every user-created tab is vibrant.
    onMount(() => {
        const ws = props.workspace;
        if (!ws) return;
        const ids = [...(ws.pinnedtabids ?? []), ...(ws.tabids ?? [])];
        if (ids.length !== 1) return;
        const firstId = ids[0];
        const tab = getObjectValue<Tab>(makeORef("tab", firstId));
        if (!tab) return;
        if (tab.meta?.["tab:color"]) return;
        // Skip if the default-color backfill has already run once for this
        // tab. Without this guard, a user who clears the color on a single-
        // tab workspace would have it silently restored on the next mount.
        if (tab.meta?.["tab:color-initialized"]) return;
        // #3b82f6 is the "Blue" entry in TAB_COLORS — same as the color
        // picker's blue swatch, so stays consistent with user-chosen blue.
        fireAndForget(async () => {
            try {
                await ObjectService.UpdateObjectMeta(
                    makeORef("tab", firstId),
                    { "tab:color": "#3b82f6", "tab:color-initialized": true } as MetaType,
                );
            } catch (e) {
                console.error("[tabbar] startup-tab color apply failed:", e);
            }
        });
    });

    onMount(() => {
        const cleanup = monitorForElements({
            canMonitor: ({ source }) => source.data.type === tabItemType,

            // Always compute insertion point from cursor position — drives the gap animation on all tabs
            onDrag: ({ source, location }) => {
                const input = location.current.input;
                const rect = tabBarScrollRef?.getBoundingClientRect();
                if (rect && !tearOffFired && input.clientY > rect.bottom + TEAR_PAST_PX) {
                    tearOffFired = true;
                    const draggedTabId = source.data.tabId as string;
                    const wsId = props.workspace?.oid;
                    if (wsId) {
                        // Phase 5: capture the original position so cancel-back
                        // can restore in place. We need TWO things:
                        //   * `wasPinned` — which list the tab lived in
                        //   * `originalTabIndex` — its index inside THAT list
                        // The backend restores into `pinnedtabids` if
                        // wasPinned, else `tabids`. Using one combined
                        // displayed-index won't do: the lists are persisted
                        // separately. (gemini PR #567 round-6 MEDIUM)
                        const ws = props.workspace;
                        const pinnedIds = ws?.pinnedtabids ?? [];
                        const tabIdsRaw = ws?.tabids ?? [];
                        const pinnedIdx = pinnedIds.indexOf(draggedTabId);
                        const wasPinned = pinnedIdx >= 0;
                        const originalTabIndex = wasPinned
                            ? pinnedIdx
                            : Math.max(0, tabIdsRaw.indexOf(draggedTabId));
                        // openWindowAtPosition + SC_MOVE expect SCREEN
                        // coordinates, but `input.clientX/Y` are
                        // viewport-relative. Convert via window.screenX/Y
                        // (works across multi-monitor setups).
                        const screenX = window.screenX + input.clientX;
                        const screenY = window.screenY + input.clientY;
                        // Phase 2: real tear-off (sidecar TearOffTab +
                        // host openWindowAtPosition + Win32 SC_MOVE).
                        // Fire-and-forget — the user is mid-drag and the
                        // host's SC_MOVE handshake will take over the
                        // cursor synchronously from Windows' point of view.
                        fireAndForget(() =>
                            requestTearOff(draggedTabId, wsId, screenX, screenY, originalTabIndex, wasPinned),
                        );
                    }
                    // Continue updating insertionPoint below — until the
                    // host's SC_MOVE handshake completes (~150-300ms cold
                    // path), the user's mouse is still over our window
                    // and pragmatic-dnd is still firing onDrag. The
                    // tearOffFired latch ensures requestTearOff doesn't
                    // re-fire; the gap-tracking is harmless during the
                    // brief overlap.
                }
                setInsertionPoint(computeInsertionPoint(input.clientX));
            },

            onDrop: ({ source, location }) => {
                // Reset the tear-off latch for the next drag.
                // NOTE: even though Phase 2's requestTearOff now does a
                // real handshake and the SC_MOVE takes over the cursor,
                // pragmatic-dnd's onDrop still fires for the original
                // gesture. We don't short-circuit here because a drag
                // that briefly dipped past the strip's bottom and then
                // came back to drop on a tab should still reorder
                // (Chrome treats the threshold as commit-on-cross, but
                // requestTearOff already snapshotted via a fire-and-
                // forget, so the data path is settled by the time onDrop
                // runs). Once Phase 5 (cancel-back) ships, this gate
                // tightens further.
                tearOffFired = false;

                const ip = insertionPoint();
                const draggedTabId = source.data.tabId as string;

                // `insertionPoint` reflects the last cursor X, so it can be
                // non-null even when the user has dragged BELOW the tab bar
                // for a tear-off. Gate both the payload-clear and the
                // reorder on the cursor actually being over the tab strip
                // at drop time — otherwise we'd suppress tear-offs and
                // also reorder on out-of-bar drops. Pragmatic-dnd does not
                // register a drop target for the bar (insertion is purely
                // X-driven), so we hit-test the cursor against the strip's
                // bounding rect ourselves.
                const input = location.current.input;
                const rect = tabBarScrollRef?.getBoundingClientRect();
                const dropInsideBar =
                    rect != null &&
                    input.clientY >= rect.top && input.clientY <= rect.bottom &&
                    input.clientX >= rect.left && input.clientX <= rect.right;
                const willReorder = dropInsideBar && ip != null && draggedTabId != null;

                if (willReorder || location.current.dropTargets.length > 0) {
                    setCurrentDragPayload(null);
                }

                if (willReorder) {
                    const tabs = tabIds();
                    const wsId = props.workspace?.oid;

                    executeReorder(ip, draggedTabId, tabs, wsId);

                    // Trigger bounce on the dragged tab at its new position
                    setBouncingTabId(draggedTabId);
                    setTimeout(() => setBouncingTabId(null), 400);

                    Logger.info("dnd", "tab drop", {
                        draggedTabId,
                        beforeTabId: ip.beforeTabId,
                        afterTabId: ip.afterTabId,
                        workspaceId: wsId,
                    });
                }

                setInsertionPoint(null);
            },
        });
        onCleanup(cleanup);
    });

    // Phase 4 — listen for the host's tear-off events. Each AgentMux
    // window subscribes; the host targets the right window via
    // emit_event_to_window so other windows see nothing.
    //
    //  tearoff:hover-changed — cursor entered this window's strip area
    //    while another window's tab is mid-tear-off. Show the standard
    //    insertion-point indicator so the user can see where the merge
    //    will land.
    //  tearoff:hover-cleared — cursor left this window's strip. Drop
    //    the indicator.
    //  tearoff:merge — mouseup over this window. Pull the dragged
    //    tab into our workspace at the cursor's X position, then close
    //    the (now empty) dragged window.
    //  tearoff:standalone — emitted to the source window when the user
    //    releases over no AgentMux window. Currently informational only;
    //    Phase 5 will use this to update cancel-back UI state.
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
        // Coordinate-space helper. `payload.cursorX/Y` come from
        // Win32's WH_MOUSE_LL hook in PHYSICAL pixels (Windows
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
        const dpr = () => Math.max(1, window.devicePixelRatio || 1);
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
                        const stripRect = tabBarScrollRef?.getBoundingClientRect();
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
                            const ownWsId = props.workspace?.oid;
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
                            const stripRect = tabBarScrollRef?.getBoundingClientRect();
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
                            const ip = computeInsertionPoint(clientX);
                            const tabs = tabIds();
                            // Convert insertion point to numeric index. The
                            // dragged tab isn't in `tabs` (different workspace),
                            // so no removal-shift adjustment needed (unlike
                            // executeReorder above).
                            let insertIdx: number;
                            if (!ip) {
                                insertIdx = tabs.length;
                            } else if (ip.beforeTabId === null) {
                                insertIdx = 0;
                            } else if (ip.afterTabId === null) {
                                insertIdx = tabs.length;
                            } else {
                                insertIdx = tabs.indexOf(ip.afterTabId);
                                if (insertIdx < 0) insertIdx = tabs.length;
                            }
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
                                const stripRect = tabBarScrollRef?.getBoundingClientRect();
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

    // Mouse-wheel hover over the strip → horizontal scroll. Ctrl+wheel
    // is reserved for the app-wide zoom (see AppZoomHandler in app.tsx),
    // so this only kicks in for unmodified scrolls. We forward whichever
    // delta dominates: `deltaX` if a horizontal-aware mouse / touchpad is
    // already producing it, otherwise `deltaY` so a normal vertical wheel
    // still scrolls the strip horizontally — matches Chrome / Edge tab
    // strip behaviour. preventDefault so the page doesn't try to scroll.
    const handleStripWheel = (e: WheelEvent) => {
        if (e.ctrlKey || e.metaKey) return;
        const delta = e.deltaX !== 0 ? e.deltaX : e.deltaY;
        if (delta === 0) return;
        if (!tabBarScrollRef) return;
        e.preventDefault();
        tabBarScrollRef.scrollLeft += delta;
    };

    onMount(() => {
        if (!tabBarScrollRef) return;
        // `passive: false` is required so preventDefault works.
        tabBarScrollRef.addEventListener("wheel", handleStripWheel, { passive: false });
        onCleanup(() => tabBarScrollRef?.removeEventListener("wheel", handleStripWheel));
    });

    if (!props.workspace) return null;

    const activeIndex = () => tabIds().indexOf(activeTabId());

    // Widgets that can be opened in a new window, sorted by display:order.
    // Excludes settings (non-pane, opens external editor) and devtools (toggles inspector).
    const openInNewWindowItems = createMemo((): MenuItem[] => {
        const wmap = atoms.fullConfigAtom()?.widgets ?? {};
        return Object.entries(wmap)
            .filter(([, w]) => {
                const view = w.blockdef?.meta?.view;
                return view !== "settings" && view !== "devtools";
            })
            .sort(([, a], [, b]) => (a["display:order"] ?? 0) - (b["display:order"] ?? 0))
            .map(([, widget]) => ({
                label: widget.label ?? "",
                icon: widget.icon,
                onClick: () => getApi().openNewWindow().catch(console.error),
            }));
    });

    const tabBarMenuItems = createMemo((): MenuItem[] => [
        {
            label: "New Tab",
            icon: "plus",
            onClick: () => createTab(),
        },
        { label: "", divider: true },
        {
            label: "New Window",
            icon: "window-restore",
            onClick: () => getApi().openNewWindow().catch(console.error),
        },
        {
            label: "Open in New Window",
            icon: "arrow-up-right-from-square",
            subItems: openInNewWindowItems(),
        },
        { label: "", divider: true },
        {
            label: "Settings",
            icon: "cog",
            onClick: () =>
                fireAndForget(async () => {
                    const path = await invokeCommand<string>("ensure_settings_file");
                    await invokeCommand("open_in_editor", { path });
                }),
        },
        {
            label: "Help",
            icon: "circle-question",
            onClick: () => fireAndForget(() => createBlock({ meta: { view: "help" } })),
        },
    ]);

    return (
        <div class="tab-bar" {...dragProps}>
            <FlyoutMenu
                items={tabBarMenuItems()}
                placement="bottom-start"
            >
                <button class="hamburger-btn" title="Menu" data-drag-region="false">
                    <i class="fa fa-bars" />
                </button>
            </FlyoutMenu>
            <div ref={tabBarScrollRef!} class="tab-bar-scroll" data-drag-region="false">
                <For each={tabIds()}>
                    {(tabId, i) => (
                        <>
                            {/* Real DOM separator between adjacent tabs (skipped
                                before index 0). Constant width + identical CSS
                                in every position guarantees uniform inter-tab
                                spacing, regardless of which tab is active /
                                hovered / dragged. Per
                                SPEC_TAB_BAR_FIRST_PRINCIPLES_2026_04_25 §3.4. */}
                            <Show when={i() > 0}>
                                <div class="tab-separator" aria-hidden="true" />
                            </Show>
                            <DroppableTab
                                tabId={tabId}
                                workspaceId={props.workspace.oid}
                                activeTabId={activeTabId()}
                                isActive={tabId === activeTabId()}
                                isFirst={i() === 0}
                                isBeforeActive={i() === activeIndex() - 1}
                                allTabCount={tabIds().length}
                                tabIndex={i()}
                                tabIds={tabIds()}
                                onSelect={() => handleSelect(tabId)}
                                onClose={() => handleClose(tabId)}
                            />
                        </>
                    )}
                </For>
            </div>
            {/* Empty right-side space — draggable so the user can grab the window from here */}
            <div class="tab-bar-fill" data-drag-region="true" />
        </div>
    );
}

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
async function requestTearOff(
    tabId: string,
    workspaceId: string,
    cursorX: number,
    cursorY: number,
    originalTabIndex: number,
    wasPinned: boolean,
): Promise<void> {
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
            destWindowLabel = await getApi().tearOffPoolPromote(newWsId, cursorX, cursorY);
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

/**
 * Execute the reorder described by the insertion point.
 * All drop logic lives here — droppable-tab.tsx is visual-only.
 */
function executeReorder(
    ip: InsertionPoint,
    draggedTabId: string,
    tabs: string[],
    wsId: string
): void {
    let insertIdx: number;
    if (ip.beforeTabId === null) {
        insertIdx = 0;
    } else if (ip.afterTabId === null) {
        insertIdx = tabs.length;
    } else {
        insertIdx = tabs.indexOf(ip.afterTabId);
    }

    const sourceIdx = tabs.indexOf(draggedTabId);
    if (sourceIdx < 0 || insertIdx < 0) return;
    // Adjust for element removal shifting indices
    const finalIdx = sourceIdx < insertIdx ? insertIdx - 1 : insertIdx;
    fireAndForget(async () => {
        try {
            await WorkspaceService.ReorderTab(wsId, draggedTabId, finalIdx);
        } catch (e) {
            Logger.error("dnd", "tab-reorder failed", { tabId: draggedTabId, finalIdx, error: String(e) });
        }
    });
}

export { TabBar };
