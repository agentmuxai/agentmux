// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Tests the lifecycle-gating half of BrowserViewModel — the `closed` flag
// that gets flipped in `dispose()` and gates navigate/goBack/goForward/
// reload/giveFocus. The state-transition logic (favicon derivation,
// title fallback, history-gate masking on url_only events, etc.) is
// covered by the pure-reducer tests in
// frontend/app/store/browser-pane-state/reducer.test.ts.

import { beforeEach, describe, expect, it, vi } from "vitest";

// Mock every platform-layer module the model touches BEFORE import.
// vi.mock is hoisted, so the mocks are in place when browser-model imports them.
vi.mock("@/app/platform/ipc", () => ({
    invokeCommand: vi.fn(() => Promise.resolve()),
    listenEvent: vi.fn(() => Promise.resolve(() => {})),
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
}));

import { invokeCommand } from "@/app/platform/ipc";
import { RpcApi } from "@/app/store/rpc-api";
import { __resetAllSlots } from "@/app/store/browser-pane-state-store";
import { BrowserViewModel } from "./browser-model";

let __vmSeq = 0;
function makeVM(): BrowserViewModel {
    // BlockNodeModel is used only by blockAtom construction — a minimal
    // placeholder is enough for the gating tests. Each test gets a
    // unique blockId so the slice store doesn't collide on register
    // across tests in the same describe block.
    return new BrowserViewModel(`test-block-${++__vmSeq}`, {} as never);
}

describe("BrowserViewModel lifecycle gating", () => {
    beforeEach(() => {
        vi.clearAllMocks();
        __resetAllSlots();
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
        vm.dispose();
        vm.navigate("https://example.com");
        expect(RpcApi.SetMetaCommand).not.toHaveBeenCalled();
        expect(vm.urlAtom()).toBe("");
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
