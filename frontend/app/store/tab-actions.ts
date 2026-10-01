// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tab management — split out of global.ts (see global.ts's "Tab management"
// section for the original context). Re-exported from global.ts for
// backward-compat (97 files import from that module).
//
// Reads `workspace`/`activeTabId` from window-identity.ts (NOT from
// global.ts) to avoid an import cycle: global.ts re-exports createTab/
// setActiveTab from this module, so this module cannot import back from
// global.ts. Both files instead import the shared window-identity base
// module.

import { keepInactiveTabsLaidOut } from "@/app/workspace/window-tab-visibility";
import { markEnd, markStart } from "@/perf";
import { fireAndForget } from "@/util/util";
import { focusManager } from "./focusManager";
import { WorkspaceService } from "./services";
import { holdRevealGate, logUngatedReveal, markTabShown, scheduleRevealLift, tabWasShown } from "./tab-reveal";
import { activeTabId, workspace } from "./window-identity";
import { createEffect, createRoot, createSignal } from "solid-js";

// The window tab `setActiveTab` is switching to, from before its RPC until
// the committed `activeTabId` settles. `workspace.tsx` shows a warm
// destination (already shown, kept laid out) from this in the same frame as
// the optimistic pill, instead of after the round trip
// (docs/analysis/ANALYSIS_WINDOW_TAB_SWITCH_PAINT_2026_09_30.md §5.2).
//
// Cleared when the committed tab reaches the intent, or moves to a tab
// outside this switch chain (the tab the chain started from plus every tab
// asked for since): e.g. the source tab was closed mid-switch and the
// backend promoted a neighbor instead (ReAgent P1 on #4107). A tab earlier
// in the chain keeps it, so a quick B-then-C doesn't flash B on its way to
// C. Also cleared when the RPC fails. Never merely because the RPC
// resolved: the Workspace push can land after the reply, and dropping the
// intent first would show the source tab again for a frame.
const [switchIntentTabId, setSwitchIntentTabId] = createSignal<string | null>(null);
let switchChain = new Set<string>();
export { switchIntentTabId };

/** Publish `tabId` as the switch intent, starting or extending the switch
 *  chain from the committed tab. Returns a cancel for when the backend
 *  didn't move after all. */
function showSwitchIntent(tabId: string): () => void {
    if (switchIntentTabId() == null) switchChain = new Set([activeTabId()]);
    switchChain.add(tabId);
    setSwitchIntentTabId(tabId);
    return () => {
        if (switchIntentTabId() === tabId) setSwitchIntentTabId(null);
    };
}

/**
 * Closing the active window tab: CloseTab's own reducer promotes a neighbor
 * (`promotedTabId`, the one the strip already shows) in the same transition
 * as the removal. A warm neighbor (already shown, kept laid out) is shown
 * from the switch intent at once, in the click's own frame, as a warm switch
 * is; it used to be held behind the reveal gate, which blanked the whole
 * content area for ~110 ms with nothing left to settle — the flash on close.
 * A neighbor never shown is still gated. Call the returned function when
 * CloseTab returns, with whether it succeeded.
 */
export function beginClosePromotion(promotedTabId: string): (closed: boolean) => void {
    if (!(keepInactiveTabsLaidOut() && tabWasShown(promotedTabId))) {
        holdRevealGate(promotedTabId);
        return () => scheduleRevealLift();
    }
    logUngatedReveal(promotedTabId);
    const cancel = showSwitchIntent(promotedTabId);
    return (closed) => {
        if (!closed) cancel();
    };
}

createRoot(() =>
    createEffect(() => {
        const intent = switchIntentTabId();
        if (intent == null) return;
        const committed = activeTabId();
        if (committed === intent || !switchChain.has(committed)) setSwitchIntentTabId(null);
    })
);

// The tab `createTab` is building, until the committed tab reaches it. The
// strip highlights it in the frame its pill first appears, instead of ~150 ms
// later when the built tab is activated: the pill used to appear unselected
// and then jump. The content still swaps in once the tab has settled.
//
// Before CreateTab replies, the new tab is the one that wasn't in the
// workspace when the creation started: the Workspace push that adds its pill
// can land before the reply, and waiting for the id cost a frame of an
// unselected pill. Dropped as soon as the committed tab moves anywhere but
// the tab this creation started from (the user went elsewhere meanwhile), or
// the creation fails or is abandoned.
//
// Only the latest creation holds the selection, and only it activates its
// tab: a newer New Tab, or any click in the strip while one is building
// (cancelTabCreation), cancels it — that tab is still created, left
// inactive. A click on the source tab would otherwise be a no-op switch to
// the committed tab, after which this selection came straight back and the
// built tab activated anyway (Codex on #4140).
type Creation = {
    tabId: string | null;
    /** Committed tabs this creation may activate from: the tab it started on,
     *  plus a tab an in-flight switch was already heading to — that switch
     *  can't be retracted, and this newer creation must still win after it
     *  lands (ReAgent on #4140). */
    from: ReadonlySet<string>;
    existing: ReadonlySet<string>;
    cancelled: boolean;
};
const [creatingTab, setCreatingTab] = createSignal<Creation | null>(null);
/** Creations not yet resolved (CreateTab hasn't replied or failed), oldest first. */
const unresolvedCreations = new Set<Creation>();
/** Tabs of creations that have resolved while others were still unresolved. */
const resolvedCreationTabIds = new Set<string>();

/** Mark `creation` resolved; once none are left, forget what was claimed. */
function resolveCreation(creation: Creation): void {
    unresolvedCreations.delete(creation);
    if (creation.tabId != null) resolvedCreationTabIds.add(creation.tabId);
    if (unresolvedCreations.size === 0) resolvedCreationTabIds.clear();
}

/**
 * How many earlier, still-unresolved creations will add a pill after
 * `creation`'s `existing` snapshot: their pills arrive before its own, so
 * they come first among the unknown ids. Computed when read, not fixed at
 * the start, so an earlier creation that fails stops counting. Their pills
 * already in the snapshot don't count either: a tab that appeared since the
 * oldest of them started and isn't claimed by a resolved creation is one of
 * theirs (ReAgent on #4140).
 */
function creationsAhead(creation: Creation): number {
    const earlier: Creation[] = [];
    for (const c of unresolvedCreations) {
        if (c === creation) break;
        earlier.push(c);
    }
    if (earlier.length === 0) return 0;
    const oldest = earlier[0];
    const arrived = [...creation.existing].filter(
        (id) => !oldest.existing.has(id) && !resolvedCreationTabIds.has(id)
    ).length;
    return Math.max(0, earlier.length - arrived);
}
/** The tab being created, for the strip to show as selected. */
export function creatingTabId(): string | null {
    const creating = creatingTab();
    if (creating == null) return null;
    if (creating.tabId != null) return creating.tabId;
    const ws = workspace();
    const ids = [...(ws?.pinnedtabids ?? []), ...(ws?.tabids ?? [])];
    const unknown = ids.filter((id) => !creating.existing.has(id) && !resolvedCreationTabIds.has(id));
    return unknown[creationsAhead(creating)] ?? null;
}
/** Stop the current creation from holding the selection or activating its
 *  tab: the user chose a tab in the strip while it was building. */
export function cancelTabCreation(): void {
    const creating = creatingTab();
    if (creating == null) return;
    creating.cancelled = true;
    setCreatingTab(null);
}
createRoot(() =>
    createEffect(() => {
        const creating = creatingTab();
        if (creating == null) return;
        const committed = activeTabId();
        if (!creating.from.has(committed)) setCreatingTab(null);
    })
);

/** How long a new tab may wait, hidden, for its panes' first data. */
const NEW_TAB_SETTLE_CAP_MS = 800;

export function createTab() {
    const ws = workspace();
    if (ws == null) return;
    // Captured BEFORE the RPC even fires — the baseline to compare against
    // once applyTabPreset finishes, so a user who switched tabs while it
    // was running (its layout-model poll alone can take up to 2s) doesn't
    // get yanked back to the new tab out from under whatever they
    // navigated to meanwhile (codex P2, PR #3300).
    const startingActiveTabId = activeTabId();
    const existing = new Set([...(ws.pinnedtabids ?? []), ...(ws.tabids ?? [])]);
    const from = new Set([startingActiveTabId]);
    const inFlight = switchIntentTabId();
    if (inFlight != null) from.add(inFlight);
    cancelTabCreation();
    const creation: Creation = { tabId: null, from, existing, cancelled: false };
    unresolvedCreations.add(creation);
    setCreatingTab(creation);
    fireAndForget(async () => {
        try {
            // Created INACTIVE (`activate: false`) — the current tab
            // stays fully visible/interactive the whole time this runs.
            // No reveal gate needed for this phase: nothing the user is
            // looking at changes yet. See
            // SPEC_TAB_CREATION_REVEAL_ARCHITECTURE_2026_09_16.md — the
            // old design activated eagerly (as part of this same RPC)
            // and raced the reveal gate's frame-heuristic settle-
            // detector against applyTabPreset's own in-flight RPCs
            // below, which are pure `await`s with no long tasks and
            // reliably outlast the gate's 80ms settle window: the tab
            // revealed near-empty, panes popped in one at a time, and
            // activating explicitly (once real content exists) could
            // re-trigger the gate on an already-revealed tab — the
            // flash a user reported.
            const tabId = await WorkspaceService.CreateTab(ws.oid, "", false, false);
            creation.tabId = tabId;
            resolveCreation(creation);
            if (creatingTab() === creation) {
                // Same object, new content: notify readers explicitly.
                setCreatingTab(null);
                setCreatingTab(creation);
            }
            // New tabs intentionally start with no `tab:color` — see
            // docs/reports/REPORT_REMOVE_AUTO_TAB_COLOR_2026_08_18.md. Users
            // still pick one manually via the right-click swatch picker
            // (tab.tsx); this used to auto-assign a random hex here.
            // Default-layout preset (agent + sysinfo + swarm). Lives in
            // a single central module so any future tab-creation path
            // (duplicate, tear-off destination, startup-tab backfill)
            // can reuse the same panes layout. See
            // frontend/app/tab/tab-presets.ts.
            const { applyTabPreset, DEFAULT_TAB_PRESET } = await import("@/app/tab/tab-presets");
            await applyTabPreset(tabId, DEFAULT_TAB_PRESET);
            // Activate now that the tab's content actually exists — via
            // the ordinary, unmodified setActiveTab() path below, whose
            // own gate/settle-detector now measures a genuine cached-
            // content switch instead of racing pane creation. A no-op
            // if the backend already auto-activated this tab (the
            // "first tab in an empty workspace" case activates
            // unconditionally, regardless of the `activate: false`
            // passed above).
            //
            // Only if the user hasn't navigated elsewhere in the meantime
            // (codex P2, PR #3300): applyTabPreset's own polling/RPCs can
            // take long enough for a rapid New Tab, or a manual switch to
            // some other tab, to land first. Forcing activation here would
            // then override that newer, more deliberate choice — instead
            // the new tab is left created but inactive; the user reaches
            // it via the tab bar whenever they actually want it, same as
            // any other background tab.
            // Its panes exist now, but their first data (the picker's agents,
            // the sysinfo history, the swarm list) is still arriving. Showing
            // the tab now put that loading on screen: covers, an empty chart,
            // "Loading…", then content popping in. Wait until it has settled,
            // hidden, then show it in one frame — bounded, so a slow pane
            // can't hold the new tab back for long.
            const { whenTabContentSettled } = await import("@/app/tab/tab-content-settled");
            await whenTabContentSettled(tabId, NEW_TAB_SETTLE_CAP_MS);
            if (!creation.cancelled && creation.from.has(activeTabId())) {
                // Built while hidden, and kept laid out: the same state as a
                // tab already shown, so it takes the same one-frame switch —
                // no reveal gate, no cross-fade, no wait for the round trip
                // (docs/analysis/ANALYSIS_NEW_WINDOW_TAB_LATENCY_2026_09_30.md
                // §3.2). With the setting off it's content-visibility: hidden
                // and not laid out, so its first reveal stays gated.
                if (keepInactiveTabsLaidOut()) markTabShown(tabId);
                await setActiveTab(tabId);
            }
        } catch (e) {
            console.error("[createTab] failed:", e);
        } finally {
            // Activated, the committed tab catches up (switchIntent is set
            // until it does) and the effect above drops this. Left inactive
            // or failed, nothing will: drop it here.
            resolveCreation(creation);
            const activating = creation.tabId != null && switchIntentTabId() === creation.tabId;
            if (creatingTab() === creation && !activating) setCreatingTab(null);
        }
    });
}

// Tracks an in-flight tab-switch measurement so rapid back-to-back
// switches (held Ctrl+Tab, programmatic bursts) don't collide on the
// shared `tab-switch:start` mark name. performance.mark throws on
// duplicates and the second call would silently drop its measurement.
// Sequence guard ensures the prior switch's pending double-rAF
// markEnd doesn't close the new switch's measurement instead.
let tabSwitchInFlight = false;
let tabSwitchSeq = 0;

export async function setActiveTab(tabId: string): Promise<void> {
    const ws = workspace();
    if (ws == null) return;
    const fromTabId = activeTabId();
    if (fromTabId === tabId) return;
    // Canonical chokepoint for tab-switch perf marks. Wraps every entry
    // path: click (tabbar), keyboard (Ctrl+Tab/1..9 in keymodel),
    // palette (command-registry), test app API (cef-api). markEnd lands
    // two rAFs after the IPC so the duration captures user-perceived
    // switch cost — IPC + Solid fan-out + layout + paint — not just IPC.
    // Backend-driven switches (tearoff merge, cross-drag) bypass this
    // function and are not measured here; they're rare and observable
    // via the long-task timeline.
    if (tabSwitchInFlight) {
        // Close prior measurement (truncated) so the new markStart
        // doesn't collide. The prior call's pending rAF markEnd will
        // see its sequence is stale and skip.
        markEnd("tab-switch", "interrupted");
    }
    const mySeq = ++tabSwitchSeq;
    tabSwitchInFlight = true;
    markStart("tab-switch", { from: fromTabId, to: tabId });
    // Pin the gate during the SetActiveTab RPC so the destination
    // tab can't paint piecemeal once the workspace update lands.
    // The auto-lift detector is started in `finally` (i.e. AFTER
    // the active-tab update lands) so SETTLE / MAX_GATE measure the
    // destination mount window, not the longtask-free RPC duration.
    // Honours rapid Ctrl-Tab spam — each call resets the detector.
    // See issue #774 / SPEC_TAB_CONTENT_REVEAL_GATE.md.
    //
    // Targeted at the DESTINATION tab (SPEC_TAB_CLOSE_BUTTON_SELECT_FLASH
    // §9): the source tab keeps painting during the RPC instead of
    // blanking the content region the moment the switch starts; only the
    // destination is FOUC-gated, from the activetabid flip until settle.
    //
    // Not for a tab already shown while inactive tabs are kept laid out: it
    // has no catch-up for the gate to hide (ANALYSIS_WINDOW_TAB_SWITCH_
    // SMOOTHNESS_2026_09_24.md §6.4, measured on #3686).
    const gated = !(keepInactiveTabsLaidOut() && tabWasShown(tabId));
    if (gated) holdRevealGate(tabId);
    const cancelIntent = showSwitchIntent(tabId);
    try {
        await WorkspaceService.SetActiveTab(ws.oid, tabId);
    } catch (e) {
        // The backend didn't move: show the committed tab again.
        if (mySeq === tabSwitchSeq) cancelIntent();
        throw e;
    } finally {
        // Pair with holdRevealGate above. Also lifts the gate on
        // the RPC-throws path so the user isn't stuck on a hidden
        // source tab.
        if (gated) scheduleRevealLift();
        else logUngatedReveal(tabId);
        requestAnimationFrame(() =>
            requestAnimationFrame(() => {
                if (mySeq === tabSwitchSeq) {
                    markEnd("tab-switch");
                    tabSwitchInFlight = false;
                    // SPEC_PANE_SELECT_AUTOFOCUS_2026_09_22.md §2a/§3 —
                    // switching tabs never moved the caret at all before
                    // this: setActiveTab() had no focus call anywhere in it.
                    // Fires after the same settle window the perf mark
                    // itself waits for, so the destination tab's panes have
                    // had a chance to mount. Guarded against interruption:
                    // if a newer switch started before this rAF pair landed,
                    // `mySeq` is stale and this whole block is skipped, so a
                    // rapid Ctrl+Tab burst only ever focuses the FINAL
                    // destination, not each intermediate one.
                    focusManager.refocusNode();
                }
            })
        );
    }
}
