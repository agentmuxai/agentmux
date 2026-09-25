// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for the Help pane's Ctrl+Wheel zoom (helpview.tsx).
 *
 * This handler was missed entirely by the original research/spec for
 * SPEC_CTRL_SHIFT_SCROLL_ZOOM_ALL_PANES_2026_09_07.md — Codex caught it on
 * PR #3090's review, after the other six duplicated Ctrl+Wheel handlers
 * (term/editor/armory/warden/swarm/AgentShellSubblock) had already been
 * fixed and tested. This file exists so a future change to this same class
 * of gesture doesn't have to rediscover this pane the same way twice.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

// Help is the first NATIVE pane tab (Pane Tab contract Phase 2b): it reads
// and writes its block only through the host context.
const setMetaMock = vi.fn((..._args: unknown[]) => Promise.resolve(undefined));

const showZoomIndicatorMock = vi.fn();
vi.mock("@/app/store/zoom", () => ({
    showZoomIndicator: (...args: unknown[]) => showZoomIndicatorMock(...args),
}));

import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import { helpPaneTab, HelpView } from "./helpview";

describe("HelpView zoom", () => {
    afterEach(() => {
        cleanup();
        setMetaMock.mockClear();
        showZoomIndicatorMock.mockClear();
    });

    function renderHelp(meta: Record<string, unknown> = {}) {
        const ctx: PaneTabHostContext = {
            blockId: "test-block",
            meta: () => meta as MetaType,
            setMeta: (patch) => setMetaMock(patch),
            isFocused: () => false,
            visibility: () => "active",
        };
        return render(() => <HelpView ctx={ctx} />);
    }

    it("Ctrl+Wheel writes help:zoom through the host context", () => {
        const { container } = renderHelp();
        const root = container.querySelector("[tabindex]") as HTMLElement;
        root.dispatchEvent(new WheelEvent("wheel", { ctrlKey: true, deltaY: -100, bubbles: true, cancelable: true }));
        expect(setMetaMock).toHaveBeenCalledWith({ "help:zoom": 1.05 });
    });

    it("starts at the zoom saved in the block's meta", () => {
        const { container } = renderHelp({ "help:zoom": 1.5 });
        const root = container.querySelector("[tabindex]") as HTMLElement;
        root.dispatchEvent(new WheelEvent("wheel", { ctrlKey: true, deltaY: -100, bubbles: true, cancelable: true }));
        expect(setMetaMock).toHaveBeenCalledWith({ "help:zoom": 1.55 });
    });

    it("registers as a native pane tab with Help's label and icon", () => {
        expect(helpPaneTab.view).toBe("help");
        expect(helpPaneTab.label).toBe("Help");
        expect(helpPaneTab.icon).toBe("circle-question");
        expect(helpPaneTab.create).toBeTypeOf("function");
        expect(helpPaneTab.viewModelClass).toBeUndefined();
    });

    it("plain wheel (no Ctrl) does not trigger a zoom RPC call", () => {
        const { container } = renderHelp();
        const root = container.querySelector("[tabindex]") as HTMLElement;
        root.dispatchEvent(new WheelEvent("wheel", { ctrlKey: false, deltaY: -100, bubbles: true, cancelable: true }));
        expect(setMetaMock).not.toHaveBeenCalled();
    });

    // Codex review, PR #3090: this handler had no shiftKey guard at all,
    // so Ctrl+Shift+Scroll over the Help pane zoomed only Help (via
    // stopPropagation()) instead of reaching AppAllPanesZoomHandler
    // (app.tsx) and zooming every pane in the window. This was a real gap
    // missed by the original six-view fix.
    it("Ctrl+Shift+Wheel does not trigger this pane's own zoom RPC call", () => {
        const { container } = renderHelp();
        const root = container.querySelector("[tabindex]") as HTMLElement;
        root.dispatchEvent(
            new WheelEvent("wheel", { ctrlKey: true, shiftKey: true, deltaY: -100, bubbles: true, cancelable: true })
        );
        expect(setMetaMock).not.toHaveBeenCalled();
    });
});
