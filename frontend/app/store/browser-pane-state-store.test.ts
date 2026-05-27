// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";
import {
    __resetAllSlots,
    type BrowserPaneProjections,
    dispatch,
    registerPane,
    setEventSink,
    snapshot,
    unregisterPane,
} from "./browser-pane-state-store";
import type {
    BrowserPaneEvent,
    BrowserTab,
} from "./browser-pane-state/types";

interface MockProj extends BrowserPaneProjections {
    calls: Record<keyof BrowserPaneProjections, unknown[]>;
}

function mkProj(): MockProj {
    const calls = {
        closed: [] as unknown[],
        loading: [] as unknown[],
        error: [] as unknown[],
        canGoBack: [] as unknown[],
        canGoForward: [] as unknown[],
        title: [] as unknown[],
        url: [] as unknown[],
        faviconUrl: [] as unknown[],
        tabs: [] as unknown[],
        activeTabId: [] as unknown[],
    };
    return {
        closed: (v) => calls.closed.push(v),
        loading: (v) => calls.loading.push(v),
        error: (v) => calls.error.push(v),
        canGoBack: (v) => calls.canGoBack.push(v),
        canGoForward: (v) => calls.canGoForward.push(v),
        title: (v) => calls.title.push(v),
        url: (v) => calls.url.push(v),
        faviconUrl: (v) => calls.faviconUrl.push(v),
        tabs: (v) => calls.tabs.push(v),
        activeTabId: (v) => calls.activeTabId.push(v),
        calls,
    };
}

describe("browser-pane-state-store (slice #9 — Phase 1A multi-tab)", () => {
    afterEach(() => {
        __resetAllSlots();
        setEventSink(() => {});
    });

    it("dispatch on unregistered blockId throws (no silent drops)", () => {
        expect(() =>
            dispatch("nope", { type: "Navigate", url: "https://x" }),
        ).toThrowError(/unregistered pane/);
    });

    it("snapshot returns null for unknown blockId", () => {
        expect(snapshot("nope")).toBeNull();
    });

    it("unregisterPane removes the slot", () => {
        const proj = mkProj();
        registerPane("blk-1", proj);
        unregisterPane("blk-1");
        expect(snapshot("blk-1")).toBeNull();
    });

    it("re-registering resets state to initial", () => {
        const proj = mkProj();
        registerPane("blk-1", proj);
        dispatch("blk-1", { type: "OpenTab", url: "https://x" });
        expect(snapshot("blk-1")?.tabs.length).toBe(1);
        registerPane("blk-1", proj);
        expect(snapshot("blk-1")?.tabs.length).toBe(0);
    });

    // ─────────────────────────────────────────────────────────────
    // Projection — active-tab fields
    // ─────────────────────────────────────────────────────────────
    describe("active-tab projections", () => {
        it("OpenTab projects url + title + loading + faviconUrl from the new active tab", () => {
            const proj = mkProj();
            registerPane("blk-1", proj);
            dispatch("blk-1", { type: "OpenTab", url: "https://example.com" });
            expect(proj.calls.url).toEqual(["https://example.com"]);
            // Title is hostname placeholder (optimistic header).
            expect(proj.calls.title).toEqual(["example.com"]);
            expect(proj.calls.faviconUrl).toEqual([
                "https://example.com/favicon.ico",
            ]);
            // Loading flipped false → true via makeTab's url-non-empty default.
            expect(proj.calls.loading).toEqual([true]);
            // closed didn't change (still false) — not projected.
            expect(proj.calls.closed).toEqual([]);
            // tabs + activeTabId projected.
            expect(proj.calls.tabs.length).toBe(1);
            expect(proj.calls.activeTabId.length).toBe(1);
        });

        it("Navigate on active tab projects url/loading/faviconUrl/title", () => {
            const proj = mkProj();
            registerPane("blk-1", proj);
            dispatch("blk-1", { type: "OpenTab", url: "" });
            // Reset call records to focus the Navigate dispatch.
            for (const k of Object.keys(proj.calls) as Array<
                keyof typeof proj.calls
            >) {
                proj.calls[k].length = 0;
            }
            dispatch("blk-1", { type: "Navigate", url: "https://example.com" });
            expect(proj.calls.url).toEqual(["https://example.com"]);
            expect(proj.calls.loading).toEqual([true]);
            expect(proj.calls.faviconUrl).toEqual([
                "https://example.com/favicon.ico",
            ]);
            expect(proj.calls.title).toEqual(["example.com"]);
        });

        it("does NOT project cells that didn't change between dispatches", () => {
            const proj = mkProj();
            registerPane("blk-1", proj);
            dispatch("blk-1", { type: "OpenTab", url: "https://x" });
            // Clear baseline projections.
            for (const k of Object.keys(proj.calls) as Array<
                keyof typeof proj.calls
            >) {
                proj.calls[k].length = 0;
            }
            // Dispatch a Navigate to the SAME url — url/title/favicon/loading
            // values shouldn't change because the tab is already at https://x
            // and loading=true.
            dispatch("blk-1", { type: "Navigate", url: "https://x" });
            // The reducer DOES rebuild the tab record on Navigate, but the
            // projection differ compares value equality (===) on each field
            // and the values are identical. So no setter fires.
            expect(proj.calls.loading).toEqual([]);
            expect(proj.calls.url).toEqual([]);
            expect(proj.calls.faviconUrl).toEqual([]);
            expect(proj.calls.title).toEqual([]);
        });

        it("switching tabs re-projects ALL changed active-tab fields", () => {
            const proj = mkProj();
            registerPane("blk-1", proj);
            // Open two tabs at different URLs.
            dispatch("blk-1", { type: "OpenTab", url: "https://a.com" });
            dispatch("blk-1", { type: "OpenTab", url: "https://b.com" });
            // After these, active is b. Set b loading=false, error=null via
            // a synthetic TabLoadingChanged so values differ between tabs.
            const snap = snapshot("blk-1")!;
            const idA = snap.tabs[0].id;
            const idB = snap.tabs[1].id;
            dispatch("blk-1", {
                type: "TabLoadingChanged",
                tabId: idB,
                loading: false,
                canGoBack: true,
                canGoForward: false,
            });
            // Clear projections to focus the switch.
            for (const k of Object.keys(proj.calls) as Array<
                keyof typeof proj.calls
            >) {
                proj.calls[k].length = 0;
            }
            // Switch to a — its fields differ from b's.
            dispatch("blk-1", { type: "SwitchTab", tabId: idA });
            // url/title/faviconUrl/loading/canGoBack all differ between
            // the two tabs (b had loading=false canGoBack=true, a has
            // loading=true canGoBack=false). All should re-project.
            expect(proj.calls.url).toEqual(["https://a.com"]);
            expect(proj.calls.title).toEqual(["a.com"]);
            expect(proj.calls.faviconUrl).toEqual([
                "https://a.com/favicon.ico",
            ]);
            expect(proj.calls.loading).toEqual([true]);
            expect(proj.calls.canGoBack).toEqual([false]);
            // canGoForward equals between the two tabs (both false), so
            // it should NOT re-project.
            expect(proj.calls.canGoForward).toEqual([]);
            // activeTabId projects on every switch.
            expect(proj.calls.activeTabId).toEqual([idA]);
        });

        it("tab field change fires only the setter that changed (not the others)", () => {
            const proj = mkProj();
            registerPane("blk-1", proj);
            dispatch("blk-1", { type: "OpenTab", url: "https://x" });
            const id = snapshot("blk-1")!.activeTabId!;
            // Clear projections.
            for (const k of Object.keys(proj.calls) as Array<
                keyof typeof proj.calls
            >) {
                proj.calls[k].length = 0;
            }
            // Just title — only `title` should project; url / loading /
            // favicon / canGoBack / canGoForward are untouched.
            dispatch("blk-1", {
                type: "TabTitleChanged",
                tabId: id,
                title: "Real Title",
            });
            expect(proj.calls.title).toEqual(["Real Title"]);
            expect(proj.calls.url).toEqual([]);
            expect(proj.calls.loading).toEqual([]);
            expect(proj.calls.faviconUrl).toEqual([]);
            expect(proj.calls.canGoBack).toEqual([]);
            expect(proj.calls.canGoForward).toEqual([]);
        });

        it("closing the last tab re-projects defaults (empty URL, fallback title, etc.)", () => {
            const proj = mkProj();
            registerPane("blk-1", proj);
            dispatch("blk-1", { type: "OpenTab", url: "https://x" });
            const id = snapshot("blk-1")!.activeTabId!;
            // Clear projections.
            for (const k of Object.keys(proj.calls) as Array<
                keyof typeof proj.calls
            >) {
                proj.calls[k].length = 0;
            }
            dispatch("blk-1", { type: "CloseTab", tabId: id });
            expect(snapshot("blk-1")?.tabs.length).toBe(0);
            expect(snapshot("blk-1")?.activeTabId).toBeNull();
            // Defaults projected back: url "", title "Browser",
            // faviconUrl "", loading false.
            expect(proj.calls.url).toEqual([""]);
            expect(proj.calls.title).toEqual(["Browser"]);
            expect(proj.calls.faviconUrl).toEqual([""]);
            expect(proj.calls.loading).toEqual([false]);
            expect(proj.calls.activeTabId).toEqual([null]);
        });
    });

    // ─────────────────────────────────────────────────────────────
    // Events
    // ─────────────────────────────────────────────────────────────
    describe("events", () => {
        it("emits navigate event through the event sink", () => {
            const proj = mkProj();
            const sink = vi.fn<(blockId: string, event: BrowserPaneEvent) => void>();
            setEventSink(sink);
            registerPane("blk-1", proj);
            dispatch("blk-1", { type: "OpenTab", url: "https://seed" });
            sink.mockClear();
            dispatch("blk-1", { type: "Navigate", url: "https://x" });
            expect(sink).toHaveBeenCalledWith("blk-1", {
                type: "navigate",
                url: "https://x",
            });
        });

        it("emits tab-opened + tab-activated on OpenTab", () => {
            const proj = mkProj();
            const seen: BrowserPaneEvent[] = [];
            setEventSink((_blockId, ev) => seen.push(ev));
            registerPane("blk-1", proj);
            dispatch("blk-1", { type: "OpenTab", url: "https://x" });
            const types = seen.map((e) => e.type);
            expect(types).toContain("tab-opened");
            expect(types).toContain("tab-activated");
        });

        it("PaneClicked routes through the event sink without state change", () => {
            const proj = mkProj();
            const sink = vi.fn<(blockId: string, event: BrowserPaneEvent) => void>();
            setEventSink(sink);
            registerPane("blk-1", proj);
            const before = snapshot("blk-1");
            dispatch("blk-1", { type: "PaneClicked" });
            const after = snapshot("blk-1");
            expect(after).toEqual(before);
            expect(sink).toHaveBeenCalledWith("blk-1", { type: "pane-clicked" });
        });

        it("Disposed gates subsequent dispatches", () => {
            const proj = mkProj();
            const sink = vi.fn<(blockId: string, event: BrowserPaneEvent) => void>();
            setEventSink(sink);
            registerPane("blk-1", proj);
            dispatch("blk-1", { type: "OpenTab", url: "https://x" });
            dispatch("blk-1", { type: "Disposed" });
            sink.mockClear();
            const lateProjCallCount = proj.calls.url.length;
            dispatch("blk-1", { type: "Navigate", url: "https://late" });
            expect(proj.calls.url.length).toBe(lateProjCallCount);
            expect(sink).toHaveBeenCalledWith("blk-1", {
                type: "post-close-command-dropped",
                commandType: "Navigate",
            });
        });
    });

    it("multi-pane: dispatches don't cross blockIds", () => {
        const a = mkProj();
        const b = mkProj();
        registerPane("blk-a", a);
        registerPane("blk-b", b);
        dispatch("blk-a", { type: "OpenTab", url: "https://a" });
        dispatch("blk-b", { type: "OpenTab", url: "https://b" });
        expect(a.calls.url).toEqual(["https://a"]);
        expect(b.calls.url).toEqual(["https://b"]);
    });
});
