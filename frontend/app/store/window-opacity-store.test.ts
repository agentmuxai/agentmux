// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// SPEC_PER_WINDOW_THEME_AND_OPACITY_2026_10_12 §4.2: the status bar slider and
// the hamburger set a window's opacity through one setter.

import { beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    setWindowOpacity: vi.fn(() => Promise.resolve()),
    updateMeta: vi.fn((_oref: string, _meta: Record<string, unknown>) => Promise.resolve()),
}));
vi.mock("@/store/global", () => ({ getApi: () => ({ setWindowOpacity: h.setWindowOpacity }) }));
vi.mock("@/store/services", () => ({ ObjectService: { UpdateObjectMeta: h.updateMeta } }));
vi.mock("@/store/mos", () => ({ makeORef: (otype: string, oid: string) => `${otype}:${oid}` }));

import { liveWindowOpacity, setWindowOpacityFor } from "./window-opacity-store";

beforeEach(() => {
    h.setWindowOpacity.mockClear();
    h.updateMeta.mockClear();
});

describe("setWindowOpacityFor", () => {
    it("live applies it to the window now and stores nothing", () => {
        setWindowOpacityFor({ label: "win-a", windowId: "w-a" }, 0.6, "live");
        expect(h.setWindowOpacity).toHaveBeenCalledWith("win-a", 0.6);
        expect(liveWindowOpacity("win-a")).toBe(0.6);
        expect(h.updateMeta).not.toHaveBeenCalled();
    });

    it("commit also stores it as the window's own value", () => {
        setWindowOpacityFor({ label: "win-a", windowId: "w-a" }, 0.7, "commit");
        expect(h.setWindowOpacity).toHaveBeenCalledWith("win-a", 0.7);
        expect(h.updateMeta).toHaveBeenCalledWith("window:w-a", { "window:opacity": 0.7 });
    });

    it("stores 100% as 1, so the window doesn't fall back to a see-through default", () => {
        setWindowOpacityFor({ label: "win-a", windowId: "w-a" }, 1, "commit");
        expect(h.updateMeta).toHaveBeenCalledWith("window:w-a", { "window:opacity": 1 });
    });

    it("never stores a floating pane's", () => {
        setWindowOpacityFor({ label: "float-1", windowId: null }, 0.5, "commit");
        expect(h.setWindowOpacity).toHaveBeenCalledWith("float-1", 0.5);
        expect(h.updateMeta).not.toHaveBeenCalled();
    });
});
