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
        // Pin the gate while CreateTab + preset import + applyTabPreset
        // run. Calling scheduleRevealLift here would let the 80ms SETTLE
        // window elapse during the (longtask-free) RPCs and layout-model
        // polling inside applyTabPreset, so the gate would lift before the
        // agent/sysinfo/swarm blocks have mounted and the user would still
        // see the piecemeal cascade. The detector is started in `finally`
        // once the preset apply has returned (or failed) — at that point
        // SETTLE / MAX_GATE measure the actual mount window. See issue
        // #774 / SPEC_TAB_CONTENT_REVEAL_GATE.md.
        holdRevealGate();
        try {
            const tabId = await WorkspaceService.CreateTab(ws.oid, "", true, false);
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
        } catch (e) {
            console.error("[createTab] failed:", e);
        } finally {
            // Pair with holdRevealGate above — without this the gate
            // would stay pinned forever on the error path.
            scheduleRevealLift();
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
