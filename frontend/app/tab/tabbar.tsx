// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { useWindowDrag } from "@/app/hook/useWindowDrag.platform";
import { HamburgerMenu } from "@/app/window/hamburger-menu";
import { deleteLayoutModelForTab } from "@/layout/index";
import { settingsAtom } from "@/store/config-signals";
import { atoms, setActiveTab } from "@/store/global";
import { RpcApi } from "@/store/rpc-api";
import { TabRpcClient } from "@/store/rpc-util";
import { holdRevealGate, scheduleRevealLift } from "@/store/tab-reveal";
import { isMacOS } from "@/util/platformutil";
import { fireAndForget } from "@/util/util";
import type { JSX } from "solid-js";
import { createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { WorkspaceService } from "../store/services";
import { resolveDisplayActiveTabId } from "./active-tab-display";
import { DroppableTab } from "./droppable-tab";
import { TabCloseConfirmModal } from "./tab-close-confirm-modal";
import { registerTabCloseRequestHandler } from "./tab-close-request";
import { useTabDragAndDrop } from "./tab-reorder";
import { useTabTearOffEvents } from "./tab-tearoff-events";
import { createTearOffTabAtRelease } from "./tab-tearoff-rpc";
import "./tabbar.scss";

interface TabBarProps {
    workspace: Workspace;
}

function TabBar(props: TabBarProps): JSX.Element {
    const activeTabId = atoms.activeTabId;
    let tabBarRef!: HTMLDivElement;
    let tabBarScrollRef!: HTMLDivElement;
    let tabBarFillRef!: HTMLDivElement;

    // Pin feature removed — merge any legacy pinnedtabids into the regular list
    // so existing workspaces don't lose tabs. A one-time UpdateTabIds (below)
    // drains pinnedtabids server-side so this concat becomes a no-op.
    const allTabIds = () => {
        const ws = props.workspace;
        if (!ws) return [];
        return [...(ws.pinnedtabids ?? []), ...(ws.tabids ?? [])];
    };

    // Optimistic close (SPEC_TAB_CLOSE_BUTTON_SELECT_FLASH §8): ids hidden
    // from the strip while a close is pending — from the moment the confirm
    // modal opens (or the close fires, on the skip-confirm path) until the
    // backend workspace update lands or the close is cancelled / fails.
    // A hidden tab is UNMOUNTED, so no ordering of the backend's HTTP
    // response vs WS push frames can ever paint it again — this is what
    // makes the close flash structurally impossible rather than a race we
    // keep winning (§§2-7 each closed one ordering hole; this closes the
    // class). A Set (not a single id) so overlapping skip-confirm closes
    // of different tabs each stay hidden.
    const [pendingHiddenTabIds, setPendingHiddenTabIds] = createSignal<ReadonlySet<string>>(new Set());
    const hideTab = (tabId: string) =>
        setPendingHiddenTabIds((prev) => {
            if (prev.has(tabId)) return prev;
            const next = new Set(prev);
            next.add(tabId);
            return next;
        });
    const unhideTab = (tabId: string) =>
        setPendingHiddenTabIds((prev) => {
            if (!prev.has(tabId)) return prev;
            const next = new Set(prev);
            next.delete(tabId);
            return next;
        });

    // What the strip renders: the workspace's tabs minus any mid-close ones.
    const tabIds = () => {
        const hidden = pendingHiddenTabIds();
        if (hidden.size === 0) return allTabIds();
        return allTabIds().filter((id) => !hidden.has(id));
    };

    // Optimistic select (SPEC_TAB_SWITCH_DECOUPLE_SELECT_FROM_PAINT_2026_09_04):
    // the tab the user just clicked, held until its SetActiveTab RPC settles.
    // Without this the pill waits on `activeTabId()` — which is
    // backend-authoritative, so it cannot move until the RPC round-trips, and
    // its arrival is also what flips the destination's `display:none → flex`
    // in the same Solid flush. That reveal owes the browser layout for a whole
    // newly-displayed subtree before anything can paint, including the pill;
    // SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md measured 500-600ms of it
    // for a large tab. Hence "the bigger the destination, the longer the tab
    // takes to even look selected."
    const [pendingSelectedTabId, setPendingSelectedTabId] = createSignal<string | null>(null);

    // What the strip highlights — see active-tab-display.ts for the full
    // precedence (optimistic select, then the close-flow neighbor promotion).
    // Deliberately NOT used by workspace.tsx's content reveal, which keeps
    // reading the raw backend-authoritative atom.
    const displayActiveTabId = () =>
        resolveDisplayActiveTabId({
            realActiveTabId: activeTabId(),
            allTabIds: allTabIds(),
            hiddenTabIds: pendingHiddenTabIds(),
            pendingSelectedTabId: pendingSelectedTabId(),
        });

    const handleSelect = (tabId: string) => {
        if (tabId === activeTabId()) return;
        // Commit the highlight BEFORE issuing the RPC so the pill paints on
        // its own cheap schedule rather than behind the destination's reveal.
        setPendingSelectedTabId(tabId);
        fireAndForget(async () => {
            try {
                await setActiveTab(tabId);
            } finally {
                // Clear only if this call's own target is still the pending
                // one — a newer click during the RPC owns the value now, and
                // clearing it here would drop the strip back to the stale
                // committed id until that newer RPC lands. Same
                // stale-generation discipline as tab-reveal.ts's gates.
                //
                // Runs on the throw path too: a select that failed must not
                // leave a phantom highlight on a tab the backend never
                // activated (mirrors handleClose's own unhideTab in finally).
                setPendingSelectedTabId((cur) => (cur === tabId ? null : cur));
            }
        });
    };

    const handleClose = (tabId: string) => {
        // Guard on the RAW list — the filtered tabIds() already excludes a
        // tab the modal path hid, so filtering here would make a 2-tab
        // workspace read as 1 and refuse every confirmed close.
        if (allTabIds().length <= 1) {
            unhideTab(tabId);
            return;
        }
        // Don't pre-select the neighbor via a separate SetActiveTab RPC
        // before closing: CloseTab's own DeleteTab reducer command already
        // reassigns the workspace's active tab to the correct neighbor
        // atomically (agentmux-srv/src/reducer/tab.rs::handle_delete_tab)
        // in the SAME state transition as the removal (§5).
        const closingActiveTab = tabId === activeTabId();
        hideTab(tabId); // no-op when the modal path already hid it
        // Destination-targeted gate (§9): hideTab already ran, so
        // displayActiveTabId() resolves to the neighbor the backend is
        // about to promote. Passing it keeps the CLOSING tab's content on
        // screen through the RPC round trip (only the neighbor hides,
        // once it becomes active, until it settles) — an untargeted hold
        // here blanked the whole content region at confirm-click, which
        // read as the neighbor pane "flashing".
        //
        // Reads the same displayActiveTabId() as the pill, so an in-flight
        // optimistic select (added by
        // SPEC_TAB_SWITCH_DECOUPLE_SELECT_FROM_PAINT_2026_09_04) can win here
        // too. Identical to the old behaviour whenever no select is pending,
        // which is the overwhelmingly common case. When one IS pending the two
        // candidates (clicked tab vs. close-promoted neighbor) are already
        // racing two concurrent RPCs on the backend, so neither is reliably
        // "the" next active tab — gating the tab the user actually clicked is
        // at least as good a guess as the inferred neighbor, and the gate's
        // own 800ms cap bounds the cost of guessing wrong either way.
        const promotedTabId = closingActiveTab ? displayActiveTabId() : null;
        fireAndForget(async () => {
            if (closingActiveTab) holdRevealGate(promotedTabId);
            try {
                await WorkspaceService.CloseTab(props.workspace.oid, tabId);
                deleteLayoutModelForTab(tabId);
            } finally {
                if (closingActiveTab) scheduleRevealLift();
                // Success: the RPC response applied the workspace update
                // synchronously before the await resolved, so the id is no
                // longer in allTabIds() and unhiding cannot resurrect it.
                // Failure: restores the tab — a close that didn't happen
                // must not leave the tab invisibly alive.
                unhideTab(tabId);
            }
        });
    };

    const [pendingCloseTabId, setPendingCloseTabId] = createSignal<string | null>(null);

    const requestClose = (tabId: string) => {
        // Guard on the VISIBLE count here (unlike handleClose's raw-list
        // guard): with N closes already pending, the raw list still counts
        // the hidden tabs until their RPCs land, so rapid skip-confirm
        // clicks could pass a raw guard and hide every last tab, leaving an
        // empty strip until an RPC failed (reagent P2 on PR #2818). A new
        // close may only START while more than one tab is actually visible.
        if (tabIds().length <= 1) return;
        if (pendingHiddenTabIds().has(tabId)) return; // close already pending
        if ((settingsAtom() as any)["tab:skipcloseconfirm"]) {
            handleClose(tabId);
        } else {
            // Repo-owner-directed UX (§8): the tab leaves the strip the
            // moment the modal opens; cancel puts it back.
            hideTab(tabId);
            setPendingCloseTabId(tabId);
        }
    };

    onMount(() => {
        const unregister = registerTabCloseRequestHandler(() => requestClose(activeTabId()));
        onCleanup(unregister);
    });

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

    // The startup tab intentionally has no `tab:color` — see
    // docs/reports/REPORT_REMOVE_AUTO_TAB_COLOR_2026_08_18.md. This used to
    // backfill a fixed "Blue" here so the first tab wouldn't look different
    // from every (then-randomly-colored) subsequent tab; now that new tabs
    // no longer auto-assign a color either (tab-actions.ts's createTab()),
    // there's no inconsistency left to paper over.

    // Commit-on-release tab tear-off. Fired from the drag monitor's onDrop
    // (useTabDragAndDrop, tab-reorder.ts) when the tab is released below
    // the strip. See tab-tearoff-rpc.ts for the full derivation.
    const tearOffTabAtRelease = createTearOffTabAtRelease(
        () => props.workspace,
        () => tabBarScrollRef
    );

    // In-strip reorder DnD + pane-drag-over-strip cleanup + Windows
    // tear-off-cursor workaround + wheel-scroll.
    //
    // RAW list, not the filtered one: these hooks compute BACKEND indices
    // (executeReorder's insertion math, tear-off merge/restore positions)
    // against the workspace's real tab_ids, which still contains any
    // optimistically-hidden mid-close tab until its RPC lands. Feeding
    // them the filtered list shifts every computed index by the number of
    // hidden tabs and silently misplaces a concurrently dragged tab
    // (reagent P1 on PR #2818). The filtered tabIds() is for RENDERING
    // only.
    useTabDragAndDrop({ tabBarScrollRef: () => tabBarScrollRef }, () => props.workspace, allTabIds, tearOffTabAtRelease);

    // Phase 4/5 — cross-window tear-off event listeners (hover/merge/
    // standalone/cancel-back). Raw list for the same reason as above.
    useTabTearOffEvents(
        () => props.workspace,
        () => tabBarScrollRef,
        allTabIds
    );

    if (!props.workspace) return null;

    const activeIndex = () => tabIds().indexOf(displayActiveTabId());

    return (
        <div ref={tabBarRef!} class="tab-bar" {...dragProps}>
            {/* Windows/Linux: hamburger sits at the LEFT of the tab strip.
                On macOS it's rendered at the far right of the window header
                instead (see window-header.tsx) so it clears the native
                traffic-light controls. */}
            <Show when={!isMacOS()}>
                <HamburgerMenu />
            </Show>
            <div ref={tabBarScrollRef!} class="tab-bar-scroll" data-drag-region="false">
                {/* When the hamburger sits to the left of the tabs (Windows/
                    Linux), give the hamburger→first-tab boundary the SAME 1px
                    separator every tab-to-tab boundary has — otherwise the
                    first tab is flush against the hamburger and reads tighter
                    than the rest. macOS renders the hamburger at the far right,
                    so no leading separator there. */}
                <Show when={!isMacOS()}>
                    <div class="tab-separator" aria-hidden="true" />
                </Show>
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
                                activeTabId={displayActiveTabId()}
                                isActive={tabId === displayActiveTabId()}
                                isFirst={i() === 0}
                                isBeforeActive={i() === activeIndex() - 1}
                                // RAW count, not the filtered one: while a
                                // close is pending the workspace still HAS
                                // the hidden tab, and undercounting here
                                // would flip droppable-tab's isLoneTabDrag/
                                // canDrag on the survivor of a 2-tab
                                // workspace, spuriously disabling its drag
                                // until the RPC resolves (reagent P2 on
                                // PR #2818).
                                allTabCount={allTabIds().length}
                                // Backend-facing index/list (drag payload's
                                // tabIndex feeds ReorderTab math): use the
                                // RAW list so indices line up with the
                                // workspace's real tab_ids even while a
                                // mid-close tab is hidden from rendering
                                // (reagent P1 on PR #2818). i() stays for
                                // the visual props above (isFirst /
                                // isBeforeActive / separators).
                                tabIndex={allTabIds().indexOf(tabId)}
                                tabIds={allTabIds()}
                                onSelect={() => handleSelect(tabId)}
                                onClose={() => requestClose(tabId)}
                            />
                        </>
                    )}
                </For>
                {/* Fill lives INSIDE the scroll container so the genuine empty
                    space to the right of the last tab is draggable. isInDragRegion
                    walks UP the DOM from the clicked element: the fill's own
                    data-drag-region="true" is found before the scroll container's
                    "false", so a click here starts a window drag. Moving it outside
                    the scroll (as a sibling) left a dead zone — the empty interior
                    of the scroll container looked draggable but wasn't. */}
                <div ref={tabBarFillRef!} class="tab-bar-fill" data-drag-region="true" />
            </div>
            <Show when={pendingCloseTabId() !== null}>
                <TabCloseConfirmModal
                    tabId={pendingCloseTabId()!}
                    onConfirm={(skipFuture) => {
                        const tabId = pendingCloseTabId()!;
                        setPendingCloseTabId(null);
                        if (skipFuture) {
                            fireAndForget(() =>
                                RpcApi.SetConfigCommand(TabRpcClient, { "tab:skipcloseconfirm": true } as any)
                            );
                        }
                        handleClose(tabId);
                    }}
                    onCancel={() => {
                        // Cancel restores the optimistically-hidden tab (§8).
                        unhideTab(pendingCloseTabId()!);
                        setPendingCloseTabId(null);
                    }}
                />
            </Show>
        </div>
    );
}

export { TabBar };
