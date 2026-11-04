// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The state behind the window tab strip (tabbar.tsx): the tab lists with the optimistically
 * hidden ones, the optimistic select, the close flow with its confirm modal, the one-time
 * migration of legacy pinned tabs, and the scroll that keeps the active tab in view. The hooks
 * are called in this order from TabBar, which is also the order their effects are created.
 */

import { revealInStrip } from "@/app/element/strip-reveal";
import { deleteLayoutModelForTab } from "@/layout/index";
import { settingsAtom } from "@/store/config-signals";
import { atoms } from "@/store/global";
import { RpcApi } from "@/store/rpc-api";
import { TabRpcClient } from "@/store/rpc-util";
import { beginClosePromotion, cancelTabCreation, creatingTabId, setActiveTab } from "@/store/tab-actions";
import { fireAndForget } from "@/util/util";
import { createEffect, createSignal, on, onCleanup, onMount, type Accessor } from "solid-js";
import { WorkspaceService } from "../store/services";
import { driveTabSelection, resolveDisplayActiveTabId } from "./active-tab-display";
import { asksBeforeClosingTab, DONT_ASK_AGAIN } from "./tab-close-confirm";
import { registerTabCloseRequestHandler } from "./tab-close-request";
import { tabWrapperRefs } from "./tabbar-dnd";

interface WorkspaceProps {
    workspace: Workspace;
}

export interface TabLists {
    /** The workspace's tabs, the raw list: what the backend indices are computed against. */
    allTabIds: () => string[];
    pendingHiddenTabIds: Accessor<ReadonlySet<string>>;
    hideTab: (tabId: string) => void;
    unhideTab: (tabId: string) => void;
    /** What the strip renders: the workspace's tabs minus any mid-close ones. */
    tabIds: () => string[];
}

export function useTabLists(props: WorkspaceProps): TabLists {
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

    const tabIds = () => {
        const hidden = pendingHiddenTabIds();
        if (hidden.size === 0) return allTabIds();
        return allTabIds().filter((id) => !hidden.has(id));
    };

    return { allTabIds, pendingHiddenTabIds, hideTab, unhideTab, tabIds };
}

export interface TabSelection {
    /** What the strip highlights. */
    displayActiveTabId: () => string;
    handleSelect: (tabId: string) => void;
}

export function useTabSelection(lists: TabLists): TabSelection {
    const activeTabId = atoms.activeTabId;

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
    // Not used by workspace.tsx's content reveal: that follows the committed
    // atom, or, for a warm destination, `setActiveTab`'s own switch intent
    // (tab-actions.ts `switchIntentTabId`), which lands in the same frame as
    // this pill.
    const displayActiveTabId = () =>
        resolveDisplayActiveTabId({
            realActiveTabId: activeTabId(),
            allTabIds: lists.allTabIds(),
            hiddenTabIds: lists.pendingHiddenTabIds(),
            // A click in flight outranks a tab still being created.
            pendingSelectedTabId: pendingSelectedTabId() ?? creatingTabId(),
        });

    // Whether a switch loop is already draining `pendingSelectedTabId`. Not a
    // signal — nothing renders off it; it only stops a second click from
    // starting a competing loop, since the running one re-reads the intent
    // and will pick the newer target up itself.
    let switchLoopRunning = false;

    const handleSelect = (tabId: string) => {
        // Guard on the DISPLAYED tab, not the committed one. With an
        // optimistic selection in flight the two differ, and comparing
        // against `activeTabId()` made a click back to the committed tab a
        // silent no-op — the in-flight switch then won over the user's newer
        // click (#2993).
        if (tabId === displayActiveTabId()) return;
        // Any choice in the strip while a new tab builds wins over it: the
        // new tab stays created but isn't selected or activated (#4140).
        // Without this a click on the source tab — already the
        // committed one — was a no-op, and the new tab took over anyway.
        cancelTabCreation();
        // Commit the highlight BEFORE issuing the RPC so the pill paints on
        // its own cheap schedule rather than behind the destination's reveal.
        setPendingSelectedTabId(tabId);
        if (switchLoopRunning) return;
        switchLoopRunning = true;
        fireAndForget(async () => {
            try {
                await driveTabSelection({
                    latestIntent: pendingSelectedTabId,
                    committed: activeTabId,
                    setActive: setActiveTab,
                });
            } finally {
                // The loop only returns once the committed id has caught up
                // with the latest intent, so dropping the override here hands
                // the strip back to backend state with nothing visibly
                // changing. On the throw path it un-sticks a phantom
                // highlight for a tab the backend never activated (mirrors
                // handleClose's own unhideTab in finally).
                switchLoopRunning = false;
                setPendingSelectedTabId(null);
            }
        });
    };

    return { displayActiveTabId, handleSelect };
}

/** Close `tabId` on the server, with the optimistic hide and the promotion of its neighbor. */
function createHandleClose(props: WorkspaceProps, lists: TabLists, selection: TabSelection): (tabId: string) => void {
    const activeTabId = atoms.activeTabId;
    const { allTabIds, hideTab, unhideTab } = lists;
    return (tabId: string) => {
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
        // atomically (crates/srv/src/reducer/tab.rs::handle_delete_tab)
        // in the SAME state transition as the removal (§5).
        const closingActiveTab = tabId === activeTabId();
        // The optimistic hide (§8/§10) — for BOTH paths now: the
        // skip-confirm path hides right here as it always did; the modal
        // path hides here too, since as of §10 requestClose no longer
        // hides early when it opens the modal. onConfirm calls this
        // function synchronously in the same click that closes the modal,
        // so the hide and the modal's disappearance land in the same frame.
        hideTab(tabId);
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
        //
        // A warm neighbor isn't gated at all: it shows at once, from the
        // switch intent (`beginClosePromotion`, tab-actions.ts).
        const settlePromotion = closingActiveTab ? beginClosePromotion(selection.displayActiveTabId()) : null;
        fireAndForget(async () => {
            let closed = false;
            try {
                await WorkspaceService.CloseTab(props.workspace.oid, tabId);
                closed = true;
                deleteLayoutModelForTab(tabId);
            } finally {
                settlePromotion?.(closed);
                // Success: the RPC response applied the workspace update
                // synchronously before the await resolved, so the id is no
                // longer in allTabIds() and unhiding cannot resurrect it.
                // Failure: restores the tab — a close that didn't happen
                // must not leave the tab invisibly alive.
                unhideTab(tabId);
            }
        });
    };
}

export interface TabClose {
    requestClose: (tabId: string) => void;
    /** The tab the confirm modal is asking about, if it is open. */
    pendingCloseTabId: Accessor<string | null>;
    /** The modal's Close: shut it and close the tab in the same click. */
    confirmClose: (skipFuture: boolean) => void;
    cancelClose: () => void;
}

export function useTabClose(props: WorkspaceProps, lists: TabLists, selection: TabSelection): TabClose {
    const activeTabId = atoms.activeTabId;
    const handleClose = createHandleClose(props, lists, selection);
    const [pendingCloseTabId, setPendingCloseTabId] = createSignal<string | null>(null);

    const requestClose = (tabId: string) => {
        // Guard on the VISIBLE count here (unlike handleClose's raw-list
        // guard): with N closes already pending, the raw list still counts
        // the hidden tabs until their RPCs land, so rapid skip-confirm
        // clicks could pass a raw guard and hide every last tab, leaving an
        // empty strip until an RPC failed (#2818). A new
        // close may only START while more than one tab is actually visible.
        if (lists.tabIds().length <= 1) return;
        if (pendingCloseTabId() === tabId) return; // modal already open for this tab
        if (lists.pendingHiddenTabIds().has(tabId)) return; // already confirmed, RPC pending
        // Asking first is opt-in (tab-close-confirm.ts).
        if (!asksBeforeClosingTab(settingsAtom() as Record<string, unknown>)) {
            handleClose(tabId);
        } else {
            // Repo-owner-directed UX, amended (SPEC_TAB_CLOSE_BUTTON_SELECT_
            // FLASH_2026_08_25.md §8 originally hid the tab the instant the
            // modal opened, "if the user cancels, put the tab back" — see
            // that section for why: an optimistic hide the strip's render
            // never has to depend on backend ordering for, so the flash it
            // was chasing became structurally impossible. In practice that
            // made the tab visibly vanish before the user had even decided,
            // then visibly reappear on Cancel — its own kind of flash. §10
            // (below) keeps the SAME optimistic-hide trick — still
            // ordering-immune, still no dependency on backend response
            // timing — just moves WHEN it fires: the tab now stays exactly
            // as it was while the modal is open (nothing to restore on
            // Cancel), and hides in the same synchronous click as the modal
            // closing on Confirm (see confirmClose) — both happen in one
            // frame, together, never one before the other.
            setPendingCloseTabId(tabId);
        }
    };

    onMount(() => {
        const unregister = registerTabCloseRequestHandler(() => requestClose(activeTabId()));
        onCleanup(unregister);
    });

    // §10: the tab hasn't been hidden yet (unlike the old §8 behavior) —
    // setPendingCloseTabId(null) (closes the modal) and handleClose's own
    // hideTab call (closes the tab) both run synchronously in this one
    // click handler, so they land in the same frame. No ordering dependency
    // between them either way — both are optimistic, backend-ordering-immune
    // sets, same property §8 originally established, just triggered together
    // here instead of at modal-open time.
    const confirmClose = (skipFuture: boolean) => {
        const tabId = pendingCloseTabId()!;
        setPendingCloseTabId(null);
        if (skipFuture) {
            fireAndForget(() =>
                RpcApi.SetConfigCommand(TabRpcClient, DONT_ASK_AGAIN as any)
            );
        }
        handleClose(tabId);
    };
    // §10: nothing to restore — the tab was never hidden while the modal was
    // open, so Cancel is just closing the modal.
    const cancelClose = () => setPendingCloseTabId(null);

    return { requestClose, pendingCloseTabId, confirmClose, cancelClose };
}

/**
 * One-time migration: if this workspace still has pinned tabs from an older
 * build, fold them into tabids and clear pinnedtabids server-side.
 */
export function usePinnedTabsMigration(props: WorkspaceProps): void {
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
}

/**
 * Keep the active tab visible when the strip scrolls; the last tab scrolls
 * the strip to its end (SPEC_TAB_BAR_DRAG_GUTTER_2026_10_04 §3). After a
 * frame, so a just-created tab has mounted and been laid out.
 */
export function useRevealActiveTab(
    displayActiveTabId: () => string,
    tabIds: () => string[],
    scrollRef: () => HTMLDivElement | undefined,
): void {
    createEffect(
        on(displayActiveTabId, (tabId) => {
            if (!tabId) return;
            const frame = requestAnimationFrame(() => {
                const el = tabWrapperRefs.get(tabId);
                const strip = scrollRef();
                if (!el || !strip) return;
                const ids = tabIds();
                revealInStrip(strip, el, ids[ids.length - 1] === tabId);
            });
            onCleanup(() => cancelAnimationFrame(frame));
        })
    );
}
