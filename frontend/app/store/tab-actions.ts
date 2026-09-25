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
import { holdRevealGate, logUngatedReveal, scheduleRevealLift, tabWasShown } from "./tab-reveal";
import { activeTabId, workspace } from "./window-identity";

export function createTab() {
    const ws = workspace();
    if (ws == null) return;
    // Captured BEFORE the RPC even fires — the baseline to compare against
    // once applyTabPreset finishes, so a user who switched tabs while it
    // was running (its layout-model poll alone can take up to 2s) doesn't
    // get yanked back to the new tab out from under whatever they
    // navigated to meanwhile (codex P2, PR #3300).
    const startingActiveTabId = activeTabId();
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
            if (activeTabId() === startingActiveTabId) {
                await setActiveTab(tabId);
            }
        } catch (e) {
            console.error("[createTab] failed:", e);
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
    try {
        await WorkspaceService.SetActiveTab(ws.oid, tabId);
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
