// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Window-identity signals — split out of global.ts's "Global signals"
// section. Lives in its own module (rather than inline in global.ts) so
// tab-actions.ts can read `workspace`/`activeTabId` without creating an
// import cycle with global.ts (global.ts re-exports tab-actions.ts's
// createTab/setActiveTab, and tab-actions.ts needs workspace/activeTabId —
// routing both global.ts and tab-actions.ts through this shared base module
// avoids the cycle). Re-exported from global.ts for backward-compat (97
// files import from that module).

import { createMemo, createSignal } from "solid-js";
import * as WOS from "./wos";

// Window identity — set once at init, never change.
export const [windowId, setWindowId] = createSignal("");
export const [clientId, setClientId] = createSignal("");
export const [staticTabId, setStaticTabId] = createSignal("");

// Derived objects from WOS
export const client = createMemo<Client>(() => {
    const cid = clientId();
    if (!cid) return null;
    return WOS.getObjectValue(WOS.makeORef("client", cid));
});

export const waveWindow = createMemo<WaveWindow>(() => {
    const wid = windowId();
    if (!wid) return null;
    return WOS.getObjectValue<WaveWindow>(WOS.makeORef("window", wid));
});

export const workspace = createMemo<Workspace>(() => {
    const win = waveWindow();
    if (!win) return null;
    return WOS.getObjectValue(WOS.makeORef("workspace", win.workspaceid));
});

export const tabAtom = createMemo<Tab>(() => {
    return WOS.getObjectValue(WOS.makeORef("tab", staticTabId()));
});

export const activeTabId = createMemo<string>(() => {
    const ws = workspace();
    const tabId = staticTabId();
    if (!ws) return tabId;
    return ws.activetabid || ws.pinnedtabids?.[0] || ws.tabids?.[0] || tabId;
});

// NOTE: uiContext must use activeTabId (derived from workspace), NOT staticTabId.
// staticTabId is set once at init and never changes. activeTabId tracks the
// workspace's current active tab so backend service calls get the correct tab.
export const uiContext = createMemo<UIContext>(() => ({
    windowid: windowId(),
    activetabid: activeTabId(),
}));
