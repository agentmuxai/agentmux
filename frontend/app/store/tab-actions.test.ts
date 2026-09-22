// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Regression test for codex's P2 finding on PR #3300
// (SPEC_TAB_CREATION_REVEAL_ARCHITECTURE_2026_09_16.md): createTab() now
// populates the new tab's panes in the background before activating it
// (applyTabPreset's own layout-model poll alone can take up to 2s). If the
// user switches to a different tab while that's still running, activating
// the new tab afterward would yank them back to it, overriding their newer,
// more deliberate choice. createTab() must skip activation in that case —
// the new tab is left created but inactive, reachable via the tab bar
// whenever the user actually wants it.

import { beforeEach, describe, expect, it, vi } from "vitest";

let mockActiveTabId = "tab-original";
const mockWorkspace = { oid: "ws-1" } as any;

// tab-actions.ts now imports focusManager.ts (for the tab-switch auto-focus
// wiring — SPEC_PANE_SELECT_AUTOFOCUS_2026_09_22.md), which transitively
// imports global.ts, which destructures ALL of these off window-identity at
// module load. vi.mock's returned object is exact — global.ts throws at
// import time on any name missing here, even one this suite never exercises.
vi.mock("./window-identity", () => ({
    workspace: () => mockWorkspace,
    activeTabId: () => mockActiveTabId,
    windowId: () => "",
    setWindowId: () => {},
    clientId: () => "",
    setClientId: () => {},
    staticTabId: () => "",
    setStaticTabId: () => {},
    client: () => ({}) as any,
    muxWindow: () => ({}) as any,
    tabAtom: () => ({}) as any,
    uiContext: () => ({}) as any,
}));

// Typed with an explicit `(...args: unknown[])` signature on the mock
// itself — vi.fn() otherwise infers a ZERO-argument type from its initial
// implementation, and spreading an `unknown[]` into that fails tsc's
// strict tuple check (TS2556) even though vitest's own transform doesn't
// catch it — always re-run `tsc --noEmit`, not just `vitest run`, after
// touching a mock like this.
const createTabRpc = vi.fn(async (..._args: unknown[]) => "tab-new");
const setActiveTabRpc = vi.fn(async (..._args: unknown[]) => undefined);
vi.mock("./services", () => ({
    WorkspaceService: {
        CreateTab: (...args: unknown[]) => createTabRpc(...args),
        SetActiveTab: (...args: unknown[]) => setActiveTabRpc(...args),
    },
}));

// applyTabPreset is dynamically imported INSIDE createTab — mock the module
// it comes from. A deferred promise lets each test control exactly when
// it resolves relative to activeTabId changing.
let resolveApplyTabPreset: (() => void) | null = null;
const applyTabPreset = vi.fn(
    (..._args: unknown[]) =>
        new Promise<void>((resolve) => {
            resolveApplyTabPreset = resolve;
        }),
);
vi.mock("@/app/tab/tab-presets", () => ({
    applyTabPreset: (...args: unknown[]) => applyTabPreset(...args),
    DEFAULT_TAB_PRESET: {},
}));

import { createTab } from "./tab-actions";

const flushMicrotasks = () => new Promise((r) => setTimeout(r, 0));

describe("createTab", () => {
    beforeEach(() => {
        mockActiveTabId = "tab-original";
        createTabRpc.mockClear();
        setActiveTabRpc.mockClear();
        applyTabPreset.mockClear();
        resolveApplyTabPreset = null;
    });

    it("creates the tab inactive (activate: false)", async () => {
        createTab();
        await flushMicrotasks();
        expect(createTabRpc).toHaveBeenCalledWith(mockWorkspace.oid, "", false, false);
    });

    it("activates the new tab once populated, if the user hasn't navigated elsewhere", async () => {
        createTab();
        await vi.waitFor(() => expect(applyTabPreset).toHaveBeenCalled());
        resolveApplyTabPreset!();
        await vi.waitFor(() => expect(setActiveTabRpc).toHaveBeenCalled());
        expect(setActiveTabRpc).toHaveBeenCalledWith(mockWorkspace.oid, "tab-new");
    });

    it("does NOT activate the new tab if the user switched to a different tab while it was being populated (codex P2, PR #3300)", async () => {
        createTab();
        await vi.waitFor(() => expect(applyTabPreset).toHaveBeenCalled());
        // Simulate the user manually switching to some other tab while
        // applyTabPreset is still in flight — the same signal createTab()
        // itself reads, just changed out from under it.
        mockActiveTabId = "tab-user-switched-to";
        resolveApplyTabPreset!();
        // Give createTab()'s async body every remaining microtask/timer
        // tick to run to completion.
        await flushMicrotasks();
        await flushMicrotasks();
        expect(setActiveTabRpc).not.toHaveBeenCalled();
    });

    it("still activates if the user navigated away and back to the ORIGINAL tab before preset apply finished", async () => {
        createTab();
        await vi.waitFor(() => expect(applyTabPreset).toHaveBeenCalled());
        mockActiveTabId = "tab-user-switched-to";
        mockActiveTabId = "tab-original"; // ...and back, before preset apply resolves
        resolveApplyTabPreset!();
        await vi.waitFor(() => expect(setActiveTabRpc).toHaveBeenCalled());
        expect(setActiveTabRpc).toHaveBeenCalledWith(mockWorkspace.oid, "tab-new");
    });
});
