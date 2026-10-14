// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    setOpenWindowLabelsAtom: vi.fn(),
    setOpenWindowEntriesAtom: vi.fn(),
    setOpenFloatingPaneEntriesAtom: vi.fn(),
    setWindowCountAtom: vi.fn(),
}));
vi.mock("@/app/store/global", () => h);
vi.mock("@/store/app-api", () => ({ getApi: () => ({ listWindowInstances: () => Promise.resolve([]) }) }));

import {
    isApplyingRemoteEvent,
    reconcileKnownEntriesFromSnapshot,
    seedKnownEntriesFromSnapshot,
} from "./launcher-event-store";

const lastCall = <T,>(fn: { mock: { calls: T[][] } }): T => fn.mock.calls[fn.mock.calls.length - 1][0];

beforeEach(() => {
    for (const fn of Object.values(h)) fn.mockClear();
});

// One module-level state cell, so the cases build on each other in order.
describe("launcher-event-store", () => {
    it("seeds instance windows and floating panes into their atoms, ignoring other labels", () => {
        seedKnownEntriesFromSnapshot([
            { label: "main", windowId: "w-main" },
            { label: "window-a", windowId: "w-a" },
            { label: "floating-1", windowId: "w-f1" },
            { label: "floating-pool-x", windowId: null },
            { label: "browser-pane-1", windowId: null },
        ]);
        expect([...lastCall(h.setOpenWindowLabelsAtom)].sort()).toEqual(["main", "window-a"]);
        expect(lastCall(h.setWindowCountAtom)).toBe(2);
        expect(lastCall(h.setOpenFloatingPaneEntriesAtom)).toEqual([
            { label: "floating-1", windowId: "w-f1" },
        ]);
    });

    it("a later seed only adds, and leaves known entries alone", () => {
        seedKnownEntriesFromSnapshot([
            { label: "window-a", windowId: null },
            { label: "window-b", windowId: "w-b" },
            { label: "floating-1", windowId: null },
        ]);
        const entries = lastCall(h.setOpenWindowEntriesAtom) as { label: string; windowId: string | null }[];
        expect(entries.find((e) => e.label === "window-a")?.windowId).toBe("w-a");
        expect(entries.map((e) => e.label).sort()).toEqual(["main", "window-a", "window-b"]);
        // Nothing new among the floating panes, so they aren't re-projected.
        expect(h.setOpenFloatingPaneEntriesAtom).not.toHaveBeenCalled();
    });

    it("reconcile replaces both sets with the snapshot", () => {
        reconcileKnownEntriesFromSnapshot([
            { label: "main", windowId: "w-main" },
            { label: "floating-2", windowId: null },
        ]);
        expect(lastCall(h.setOpenWindowLabelsAtom)).toEqual(["main"]);
        expect(lastCall(h.setWindowCountAtom)).toBe(1);
        expect(lastCall(h.setOpenFloatingPaneEntriesAtom)).toEqual([
            { label: "floating-2", windowId: null },
        ]);
    });

    it("is not mid-apply outside a launcher event", () => {
        expect(isApplyingRemoteEvent()).toBe(false);
    });
});
