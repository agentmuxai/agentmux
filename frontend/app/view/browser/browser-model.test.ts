// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Tests the lifecycle-gating half of BrowserViewModel — the `closed` flag
// that gets flipped in `dispose()` and gates navigate/goBack/goForward/
// reload/giveFocus. This is the frontend half of the orphan-prevention
// fix in SPEC_BROWSER_PANE_LIFECYCLE_TESTS.md §4.

import { beforeEach, describe, expect, it, vi } from "vitest";

// Mock every platform-layer module the model touches BEFORE import.
// vi.mock is hoisted, so the mocks are in place when browser-model imports them.
vi.mock("@/app/platform/ipc", () => ({
    invokeCommand: vi.fn(() => Promise.resolve()),
    // Pane subscribes to `browser-pane-nav-state` and `browser-pane-clicked`
    // in the constructor; both are stubbed here so test VMs don't spawn real
    // listeners. The returned promise resolves with a no-op unsubscribe.
    listenEvent: vi.fn(() => Promise.resolve(() => {})),
}));

vi.mock("@/app/store/global", () => ({
    refocusNode: vi.fn(),
}));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        SetMetaCommand: vi.fn(() => Promise.resolve()),
    },
}));

vi.mock("@/app/store/rpc-util", () => ({
    TabRpcClient: {},
}));

vi.mock("@/app/store/wos", () => ({
    makeORef: (type: string, id: string) => `${type}:${id}`,
    getWaveObjectAtom: () => () => ({ meta: {} }),
    // global.ts evaluates a `tabAtom` createMemo at module-init that calls
    // WOS.getObjectValue; without this stub the import chain crashes during
    // test setup before any BrowserViewModel test can run.
    getObjectValue: () => ({}),
}));

import { invokeCommand } from "@/app/platform/ipc";
import { RpcApi } from "@/app/store/rpc-api";
import { BrowserViewModel } from "./browser-model";

function makeVM() {
    // BlockNodeModel is used only by blockAtom construction — a minimal
    // placeholder is enough for the gating tests.
    const vm = new BrowserViewModel("test-block-id", {} as never);
    // The constructor calls `navigate(DEFAULT_BROWSER_URL)` and registers
    // two `listenEvent` subscriptions. Tests below assert call counts on
    // operations performed AFTER construction, so clear the mock history
    // here to keep the assertions readable.
    vi.clearAllMocks();
    return vm;
}

describe("BrowserViewModel lifecycle gating", () => {
    beforeEach(() => {
        vi.clearAllMocks();
    });

    it("is not closed after construction", () => {
        const vm = makeVM();
        expect(vm.closed).toBe(false);
    });

    it("dispose() flips closed to true", () => {
        const vm = makeVM();
        vm.dispose();
        expect(vm.closed).toBe(true);
    });

    it("navigate() is a no-op after dispose", () => {
        const vm = makeVM();
        // Capture the pre-dispose URL (the ctor auto-navigates to
        // DEFAULT_BROWSER_URL); a post-dispose navigate must not change it.
        const urlBefore = vm.urlAtom();
        vm.dispose();
        vm.navigate("https://example.com");
        expect(RpcApi.SetMetaCommand).not.toHaveBeenCalled();
        expect(vm.urlAtom()).toBe(urlBefore);
    });

    it("navigate() works before dispose", () => {
        const vm = makeVM();
        vm.navigate("https://example.com");
        expect(RpcApi.SetMetaCommand).toHaveBeenCalledOnce();
        expect(vm.urlAtom()).toBe("https://example.com");
    });

    it("giveFocus() returns false after dispose, issues no IPC", () => {
        const vm = makeVM();
        vm.dispose();
        const result = vm.giveFocus();
        expect(result).toBe(false);
        expect(invokeCommand).not.toHaveBeenCalled();
    });

    it("goBack() is a no-op after dispose", () => {
        const vm = makeVM();
        // Populate history so goBack would have something to do pre-dispose
        vm.navigate("https://a.com");
        vm.navigate("https://b.com");
        vi.clearAllMocks();

        vm.dispose();
        vm.goBack();
        expect(RpcApi.SetMetaCommand).not.toHaveBeenCalled();
    });

    it("goForward() is a no-op after dispose", () => {
        const vm = makeVM();
        vm.navigate("https://a.com");
        vm.navigate("https://b.com");
        vm.goBack();
        vi.clearAllMocks();

        vm.dispose();
        vm.goForward();
        expect(RpcApi.SetMetaCommand).not.toHaveBeenCalled();
    });

    it("reload() is a no-op after dispose", () => {
        const vm = makeVM();
        vm.navigate("https://a.com");

        vm.dispose();
        vm.reload();
        // reload() would normally clear urlAtom() to "" then restore via rAF.
        // Gated out — urlAtom stays at "https://a.com" synchronously.
        expect(vm.urlAtom()).toBe("https://a.com");
        // Confirm no rAF-scheduled work sneaks through either.
        return new Promise<void>((resolve) => {
            requestAnimationFrame(() => {
                expect(vm.urlAtom()).toBe("https://a.com");
                resolve();
            });
        });
    });

    it("history is not mutated by navigate-after-dispose", () => {
        const vm = makeVM();
        vm.navigate("https://a.com");
        vm.dispose();
        vm.navigate("https://b.com");

        // urlAtom must still reflect the last pre-dispose navigate.
        expect(vm.urlAtom()).toBe("https://a.com");
        expect(vm.canGoBackAtom()).toBe(false);
    });

    it("dispose is idempotent", () => {
        const vm = makeVM();
        vm.dispose();
        vm.dispose();
        expect(vm.closed).toBe(true);
    });
});
