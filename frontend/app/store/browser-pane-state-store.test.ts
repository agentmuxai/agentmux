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
import type { BrowserPaneEvent } from "./browser-pane-state/types";

function mkProj(): BrowserPaneProjections & {
    calls: Record<keyof BrowserPaneProjections, unknown[]>;
} {
    const calls = {
        closed: [] as unknown[],
        loading: [] as unknown[],
        error: [] as unknown[],
        canGoBack: [] as unknown[],
        canGoForward: [] as unknown[],
        title: [] as unknown[],
        url: [] as unknown[],
        faviconUrl: [] as unknown[],
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
        calls,
    };
}

describe("browser-pane-state-store (slice #9 Phase 4)", () => {
    afterEach(() => {
        __resetAllSlots();
        // Reset event sink so tests don't leak state into each other.
        setEventSink(() => {});
    });

    it("dispatch on unregistered blockId throws (no silent drops)", () => {
        expect(() =>
            dispatch("nope", { type: "Navigate", url: "https://x" }),
        ).toThrowError(/unregistered pane/);
    });

    it("registers a pane and projects state changes", () => {
        const proj = mkProj();
        registerPane("blk-1", proj);
        dispatch("blk-1", { type: "Navigate", url: "https://example.com" });
        expect(proj.calls.url).toEqual(["https://example.com"]);
        expect(proj.calls.loading).toEqual([true]);
        expect(proj.calls.faviconUrl).toEqual([
            "https://example.com/favicon.ico",
        ]);
        // Cells that didn't change shouldn't be projected.
        expect(proj.calls.closed).toEqual([]);
        expect(proj.calls.title).toEqual([]);
    });

    it("does NOT project cells that didn't change between dispatches", () => {
        const proj = mkProj();
        registerPane("blk-1", proj);
        dispatch("blk-1", { type: "Navigate", url: "https://x" });
        dispatch("blk-1", { type: "Navigate", url: "https://x" });
        // Second dispatch had identical url + already-loading, but
        // Navigate always sets loading=true (idempotent on signal write,
        // not on reducer state). So loading should fire twice — but
        // url is identical and faviconUrl is identical, so those
        // shouldn't fire again.
        expect(proj.calls.url).toEqual(["https://x"]);
        expect(proj.calls.faviconUrl).toEqual(["https://x/favicon.ico"]);
    });

    it("emits events through the event sink", () => {
        const proj = mkProj();
        const sink = vi.fn<(blockId: string, event: BrowserPaneEvent) => void>();
        setEventSink(sink);
        registerPane("blk-1", proj);
        dispatch("blk-1", { type: "Navigate", url: "https://x" });
        expect(sink).toHaveBeenCalledWith("blk-1", {
            type: "navigate",
            url: "https://x",
        });
    });

    it("PaneClicked routes through the event sink without state change", () => {
        const proj = mkProj();
        const sink = vi.fn<(blockId: string, event: BrowserPaneEvent) => void>();
        setEventSink(sink);
        registerPane("blk-1", proj);
        const before = snapshot("blk-1");
        dispatch("blk-1", { type: "PaneClicked" });
        const after = snapshot("blk-1");
        expect(after).toEqual(before); // no state change
        expect(sink).toHaveBeenCalledWith("blk-1", { type: "pane-clicked" });
    });

    it("Disposed gates subsequent dispatches", () => {
        const proj = mkProj();
        const sink = vi.fn<(blockId: string, event: BrowserPaneEvent) => void>();
        setEventSink(sink);
        registerPane("blk-1", proj);
        dispatch("blk-1", { type: "Navigate", url: "https://x" });
        dispatch("blk-1", { type: "Disposed" });
        sink.mockClear();
        const lateProjCallCount = proj.calls.url.length;
        dispatch("blk-1", { type: "Navigate", url: "https://late" });
        // Reducer drops the late command — no url projection.
        expect(proj.calls.url.length).toBe(lateProjCallCount);
        // But the post-close-command-dropped event is emitted.
        expect(sink).toHaveBeenCalledWith("blk-1", {
            type: "post-close-command-dropped",
            commandType: "Navigate",
        });
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
        dispatch("blk-1", { type: "Navigate", url: "https://x" });
        expect(snapshot("blk-1")?.url).toBe("https://x");
        registerPane("blk-1", proj);
        expect(snapshot("blk-1")?.url).toBe("");
    });

    it("multi-pane: dispatches don't cross blockIds", () => {
        const a = mkProj();
        const b = mkProj();
        registerPane("blk-a", a);
        registerPane("blk-b", b);
        dispatch("blk-a", { type: "Navigate", url: "https://a" });
        dispatch("blk-b", { type: "Navigate", url: "https://b" });
        expect(a.calls.url).toEqual(["https://a"]);
        expect(b.calls.url).toEqual(["https://b"]);
    });
});
