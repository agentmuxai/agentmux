// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AnchoredPopover owns what every status bar popover and the agent pane's
 * runtime/session panels used to copy: dismiss, the positioning lifecycle,
 * the airspace cut and the chrome-zoom body. CSS `zoom` has no layout in
 * jsdom, so these pin the structure and the lifecycle; the geometry was
 * measured in Chrome 154 (docs/reports/REPORT_CHROME_ZOOM_POPOVERS_2026_10_02.md §4).
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { autoUpdate } from "@floating-ui/dom";
import { usePaneOverlay } from "@/app/platform/pane-overlay";

vi.mock("@floating-ui/dom", () => ({
    autoUpdate: vi.fn(() => vi.fn()),
}));
vi.mock("@/app/platform/pane-overlay", () => ({
    usePaneOverlay: vi.fn(),
}));
vi.mock("@/app/util/menu-position", () => ({
    computeMenuPosition: vi.fn(async () => ({ style: { position: "fixed", left: "0px", top: "0px" } })),
}));

import { AnchoredPopover } from "./anchored-popover";

let pendingFrames: Array<{ id: number; cb: FrameRequestCallback }> = [];
let nextFrameId = 1;
const flushFrames = (): void => {
    const due = pendingFrames;
    pendingFrames = [];
    for (const frame of due) frame.cb(0);
};

function mount(opts: { onDismiss?: () => void; zoom?: "chrome" | "none" } = {}) {
    const anchor = document.createElement("button");
    document.body.appendChild(anchor);
    const result = render(() => (
        <AnchoredPopover anchor={anchor} placement="top-end" onDismiss={opts.onDismiss} zoom={opts.zoom} class="my-panel">
            <span class="inside">content</span>
            <input class="field" />
        </AnchoredPopover>
    ));
    const body = document.querySelector<HTMLElement>(".my-panel")!;
    const shell = body.parentElement!;
    return { anchor, body, shell, ...result };
}

describe("AnchoredPopover", () => {
    beforeEach(() => {
        pendingFrames = [];
        nextFrameId = 1;
        vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
            const id = nextFrameId++;
            pendingFrames.push({ id, cb });
            return id;
        });
        vi.stubGlobal("cancelAnimationFrame", (id: number) => {
            pendingFrames = pendingFrames.filter((frame) => frame.id !== id);
        });
    });

    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
        vi.mocked(autoUpdate).mockClear();
        vi.mocked(usePaneOverlay).mockClear();
        document.body.innerHTML = "";
    });

    it("portals a positioned shell around a body that carries the panel's class and chrome zoom", () => {
        const { body, shell } = mount();
        expect(shell.classList.contains("anchored-popover")).toBe(true);
        expect(shell.hasAttribute("data-pane-overlay")).toBe(true);
        expect(shell.getAttribute("data-zoom-scope")).toBe("chrome");
        expect(shell.style.position).toBe("fixed");
        expect(body.classList.contains("anchored-popover-body")).toBe(true);
        expect(body.classList.contains("anchored-popover-body--chrome-zoom")).toBe(true);
        expect(body.querySelector(".inside")).not.toBeNull();
    });

    it("leaves the body unzoomed with zoom=none", () => {
        const { body, shell } = mount({ zoom: "none" });
        expect(body.classList.contains("anchored-popover-body--chrome-zoom")).toBe(false);
        expect(shell.hasAttribute("data-zoom-scope")).toBe(false);
    });

    it("registers the airspace cut on every platform", () => {
        mount();
        expect(usePaneOverlay).toHaveBeenCalledTimes(1);
    });

    it("dismisses on a mousedown outside, but not inside or on the anchor", () => {
        const onDismiss = vi.fn();
        const { anchor, body } = mount({ onDismiss });
        fireEvent.mouseDown(body.querySelector(".inside")!);
        fireEvent.mouseDown(anchor);
        expect(onDismiss).not.toHaveBeenCalled();
        fireEvent.mouseDown(document.body);
        expect(onDismiss).toHaveBeenCalledTimes(1);
    });

    it("dismisses on Esc, but leaves Esc in a field inside the panel to the field", () => {
        const onDismiss = vi.fn();
        const { body } = mount({ onDismiss });
        fireEvent.keyDown(body.querySelector(".field")!, { key: "Escape" });
        expect(onDismiss).not.toHaveBeenCalled();
        fireEvent.keyDown(document.body, { key: "Escape" });
        expect(onDismiss).toHaveBeenCalledTimes(1);
    });

    it("registers no dismiss listeners without onDismiss (a hover tip)", () => {
        const docAdd = vi.spyOn(document, "addEventListener");
        const winAdd = vi.spyOn(window, "addEventListener");
        try {
            mount();
            expect(docAdd.mock.calls.filter(([type]) => type === "mousedown")).toHaveLength(0);
            expect(winAdd.mock.calls.filter(([type]) => type === "keydown")).toHaveLength(0);
        } finally {
            docAdd.mockRestore();
            winAdd.mockRestore();
        }
    });

    it("registers both dismiss listeners with onDismiss", () => {
        const docAdd = vi.spyOn(document, "addEventListener");
        const winAdd = vi.spyOn(window, "addEventListener");
        try {
            mount({ onDismiss: vi.fn() });
            expect(docAdd.mock.calls.filter(([type]) => type === "mousedown")).toHaveLength(1);
            expect(winAdd.mock.calls.filter(([type]) => type === "keydown")).toHaveLength(1);
        } finally {
            docAdd.mockRestore();
            winAdd.mockRestore();
        }
    });

    it("starts autoUpdate on the first frame, against a reference that reads the captured anchor", () => {
        const { anchor } = mount();
        anchor.getBoundingClientRect = () => new DOMRect(4, 8, 16, 32);
        expect(autoUpdate).not.toHaveBeenCalled();
        flushFrames();
        expect(autoUpdate).toHaveBeenCalledTimes(1);
        const reference = vi.mocked(autoUpdate).mock.calls[0]![0] as { getBoundingClientRect: () => DOMRect };
        expect(reference.getBoundingClientRect().x).toBe(4);
    });

    it("stops autoUpdate when unmounted", () => {
        const stop = vi.fn();
        vi.mocked(autoUpdate).mockReturnValueOnce(stop);
        const { unmount } = mount();
        flushFrames();
        unmount();
        expect(stop).toHaveBeenCalledTimes(1);
    });

    it("never starts autoUpdate when unmounted before its first frame", () => {
        const { unmount } = mount();
        unmount();
        flushFrames();
        expect(autoUpdate).not.toHaveBeenCalled();
    });
});
