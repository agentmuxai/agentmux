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

import { markEnd, markStart } from "@/perf";
import { fireAndForget } from "@/util/util";
import { WorkspaceService } from "./services";
import { holdRevealGate, scheduleRevealLift } from "./tab-reveal";
import { activeTabId, workspace } from "./window-identity";

export function createTab() {
    const ws = workspace();
    if (ws == null) return;
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
            await setActiveTab(tabId);
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
    holdRevealGate(tabId);
    try {
        await WorkspaceService.SetActiveTab(ws.oid, tabId);
    } finally {
        // Pair with holdRevealGate above. Also lifts the gate on
        // the RPC-throws path so the user isn't stuck on a hidden
        // source tab.
        scheduleRevealLift();
        requestAnimationFrame(() =>
            requestAnimationFrame(() => {
                if (mySeq === tabSwitchSeq) {
                    markEnd("tab-switch");
                    tabSwitchInFlight = false;
                }
            })
        );
    }
}
