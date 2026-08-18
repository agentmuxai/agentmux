// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Tests the lifecycle-gating half of BrowserViewModel — the `closed` flag
// that gets flipped in `dispose()` and gates navigate/goBack/goForward/
// reload/giveFocus. This is the frontend half of the orphan-prevention
// fix in SPEC_BROWSER_PANE_LIFECYCLE_TESTS.md §4.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

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

import { invokeCommand, listenEvent } from "@/app/platform/ipc";
import { RpcApi } from "@/app/store/rpc-api";
import { BrowserViewModel } from "./browser-model";

/** Retrieves the handler the model registered for a given event name via
 *  `listenEvent(name, handler)`, so tests can invoke it directly with a
 *  synthetic payload instead of needing a real IPC round-trip. */
function capturedHandler(eventName: string): (payload: any) => void {
    const call = vi
        .mocked(listenEvent)
        .mock.calls.find(([name]) => name === eventName);
    if (!call) throw new Error(`no listenEvent registration found for "${eventName}"`);
    return call[1] as (payload: any) => void;
}

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

/** Same as `makeVM()`, but captures the nav-state handler BEFORE clearing
 *  mock history (`makeVM`'s clear would wipe `listenEvent`'s recorded
 *  calls, taking the handler reference with it). */
function makeVMWithNavStateHandler() {
    const vm = new BrowserViewModel("test-block-id", {} as never);
    const navStateHandler = capturedHandler("browser-pane-nav-state");
    vi.clearAllMocks();
    return { vm, navStateHandler };
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

// SPEC_BROWSER_PANE_LOADING_BRAIN_INDICATOR_2026_07_11.md §4.2: the
// browser-pane-nav-state listener used to unconditionally dispatch
// LoadFinished on every event (including ones fired at navigation START,
// not just completion), so `loadingAtom` cleared within the same tick
// Navigate() set it. These tests pin the fixed behavior — is_loading now
// drives an accurate TabLoadingChanged dispatch.
describe("BrowserViewModel nav-state loading wiring", () => {
    beforeEach(() => {
        vi.clearAllMocks();
        // Layer 3 (SPEC_BROWSER_PANE_LOADING_INDICATOR_FLICKER_2026_08_17.md)
        // holds a loading:false dispatch briefly so a rapid true→false→true
        // flip collapses into one visible transition — tests that assert on
        // a `false` outcome need to advance past that hold.
        vi.useFakeTimers();
    });

    afterEach(() => {
        vi.useRealTimers();
    });

    it("loadingAtom starts true after construction (the ctor's initial navigate())", () => {
        const { vm } = makeVMWithNavStateHandler();
        expect(vm.loadingAtom()).toBe(true);
    });

    it("an on_loading_state_change event with is_loading:true does NOT clear loadingAtom (the exact prior bug)", () => {
        const { vm, navStateHandler } = makeVMWithNavStateHandler();
        navStateHandler({
            block_id: "test-block-id",
            url: "https://example.com",
            is_loading: true,
            can_go_back: false,
            can_go_forward: false,
        });
        expect(vm.loadingAtom()).toBe(true);
    });

    it("an on_loading_state_change event with is_loading:false clears loadingAtom after the debounce hold", () => {
        const { vm, navStateHandler } = makeVMWithNavStateHandler();
        navStateHandler({
            block_id: "test-block-id",
            url: "https://example.com",
            is_loading: false,
            can_go_back: true,
            can_go_forward: false,
        });
        // canGoBackAtom flows through the separate, non-debounced
        // HistoryUpdated dispatch — updates immediately regardless of the
        // loading hold.
        expect(vm.canGoBackAtom()).toBe(true);
        // loadingAtom is held briefly (layer 3) before it clears.
        expect(vm.loadingAtom()).toBe(true);
        vi.advanceTimersByTime(250);
        expect(vm.loadingAtom()).toBe(false);
    });

    it("a true arriving within the debounce window cancels the pending false — no flicker", () => {
        const { vm, navStateHandler } = makeVMWithNavStateHandler();
        navStateHandler({
            block_id: "test-block-id",
            url: "https://example.com",
            is_loading: false,
            can_go_back: false,
            can_go_forward: false,
        });
        expect(vm.loadingAtom()).toBe(true); // still held
        vi.advanceTimersByTime(50); // well within the hold window
        navStateHandler({
            block_id: "test-block-id",
            url: "https://example.com",
            is_loading: true,
            can_go_back: false,
            can_go_forward: false,
        });
        // The held false never fires — advance well past when it would have.
        vi.advanceTimersByTime(500);
        expect(vm.loadingAtom()).toBe(true);
    });

    // reagent P1 on PR #2642: this used to send ONLY the url_only event and
    // assert it alone cleared loadingAtom via an immediate (non-debounced)
    // LoadFinished dispatch — that dispatch is now removed, since it
    // bypassed the layer-3 debounce hold for exactly the same-tick
    // redirect-hop case the hold exists to coalesce. Real
    // on_load_end_browser_pane calls ALWAYS emit a paired is_loading:false
    // event first (layer 1) — simulate that real pairing here, not the
    // url_only event in isolation.
    it("the paired is_loading:false + url_only (on_load_end) events clear loadingAtom after the debounce hold", () => {
        const { vm, navStateHandler } = makeVMWithNavStateHandler();
        navStateHandler({
            block_id: "test-block-id",
            url: "https://example.com",
            is_loading: false,
            can_go_back: false,
            can_go_forward: false,
        });
        navStateHandler({
            block_id: "test-block-id",
            url: "https://example.com",
            url_only: true,
        });
        expect(vm.loadingAtom()).toBe(true); // still held
        vi.advanceTimersByTime(250);
        expect(vm.loadingAtom()).toBe(false);
    });

    it("a same-tick redirect hop's url_only event no longer bypasses the debounce hold", () => {
        const { vm, navStateHandler } = makeVMWithNavStateHandler();
        // The finishing hop's is_loading:false, immediately followed by the
        // url_only address-bar-sync event Rust always pairs with it.
        navStateHandler({
            block_id: "test-block-id",
            url: "https://a.com",
            is_loading: false,
            can_go_back: false,
            can_go_forward: false,
        });
        navStateHandler({ block_id: "test-block-id", url: "https://a.com", url_only: true });
        // A fresh redirect hop lands within the debounce window.
        vi.advanceTimersByTime(50);
        navStateHandler({
            block_id: "test-block-id",
            url: "https://b.com",
            is_loading: true,
            can_go_back: false,
            can_go_forward: false,
        });
        // The held false never fires — advance well past when it would have.
        vi.advanceTimersByTime(500);
        expect(vm.loadingAtom()).toBe(true);
    });

    it("re-navigating after load-finished sets loadingAtom true again, and a subsequent is_loading:false clears it", () => {
        const { vm, navStateHandler } = makeVMWithNavStateHandler();
        navStateHandler({ block_id: "test-block-id", url: "https://a.com", is_loading: false, can_go_back: false, can_go_forward: false });
        vi.advanceTimersByTime(250);
        expect(vm.loadingAtom()).toBe(false);

        // navigate() cancels any pending held false so a stale hold can't
        // clobber this fresh true a moment later.
        vm.navigate("https://b.com");
        expect(vm.loadingAtom()).toBe(true);

        navStateHandler({ block_id: "test-block-id", url: "https://b.com", is_loading: false, can_go_back: true, can_go_forward: false });
        vi.advanceTimersByTime(250);
        expect(vm.loadingAtom()).toBe(false);
    });

    it("a nav-state event for a different block_id is ignored", () => {
        const { vm, navStateHandler } = makeVMWithNavStateHandler();
        expect(vm.loadingAtom()).toBe(true);
        navStateHandler({
            block_id: "some-other-block-id",
            url: "https://example.com",
            is_loading: false,
            can_go_back: false,
            can_go_forward: false,
        });
        expect(vm.loadingAtom()).toBe(true);
    });
});
